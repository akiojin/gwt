import { test, expect } from "@playwright/test";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

test.describe("SessionStart foreground responsiveness (#5042)", () => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  test.skip(!base || !home, "requires an isolated browser-check checkout instance");
  test.setTimeout(90_000);

  test("foreground dispatch remains within budget during a viewport burst", async ({ page }, info) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    let lastX: number | undefined;
    page.on("websocket", socket => socket.on("framereceived", ({ payload }) => {
      const event = JSON.parse(String(payload));
      if (event.kind === "workspace_state") {
        lastX = event.workspace.tabs?.[0]?.workspace?.viewport?.x ?? event.workspace.viewport?.x;
      }
    }));
    await gotoLiveGwt(page, base!, { enableTestBridge: true });
    await expect(page.locator("#close-project-button")).toBeVisible();
    const theme = info.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

    // Use real frontend -> WebSocket -> GUI dispatch, including asynchronous
    // persistence. The exact SessionStart transaction/races have queued-worker
    // Rust regressions; this test measures the foreground event-loop budget.
    const count = 500;
    const offset = theme === "dark" ? 100_000 : 200_000;
    const startedAt = Date.now();
    await page.evaluate(({ count, offset }) => {
      for (let index = 1; index <= count; index++) {
        window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail: {
          kind: "update_viewport", viewport: { x: offset + index, y: 0, zoom: 1 },
        } }));
      }
    }, { count, offset });
    await expect.poll(() => lastX, { timeout: 30_000 }).toBe(offset + count);

    const samples = async (): Promise<number[]> => {
      const directory = join(home!, ".gwt", "logs");
      const names = (await readdir(directory)).filter(name => name.startsWith("gwt.log"));
      const logs = await Promise.all(names.map(name => readFile(join(directory, name), "utf8")));
      return logs.flatMap(log => log.split("\n").flatMap(line => {
        try {
          const row = JSON.parse(line);
          return row.target === "gwt.frontend.timing" && row.fields?.event === "UpdateViewport"
            && Date.parse(row.timestamp) >= startedAt && typeof row.fields.queue_wait_ms === "number"
            ? [row.fields.queue_wait_ms] : [];
        } catch { return []; }
      }));
    };
    await expect.poll(async () => (await samples()).length, { timeout: 30_000 }).toBeGreaterThanOrEqual(count);
    const waits = (await samples()).sort((left, right) => left - right);
    const p50 = waits[Math.floor(waits.length / 2)];
    const max = waits[waits.length - 1];
    await info.attach("foreground-queue-wait", {
      body: JSON.stringify({ theme, samples: waits.length, p50_ms: p50, max_ms: max, queue_wait_ms: waits }),
      contentType: "application/json",
    });
    console.log(`${theme}: foreground queue wait n=${waits.length}, p50=${p50}ms, max=${max}ms`);
    expect(p50).toBeLessThanOrEqual(200);
    expect(max).toBeLessThanOrEqual(2000);
    await sendLiveGwtEvent(page, { kind: "update_viewport", viewport: { x: 0, y: 0, zoom: 1 } });
    await expect.poll(() => lastX).toBe(0);
    expect(errors).toEqual([]);
    const screenshot = info.outputPath(`${theme}-workspace.png`);
    await page.screenshot({ path: screenshot });
    await info.attach(`${theme}-workspace`, { path: screenshot, contentType: "image/png" });
  });
});
