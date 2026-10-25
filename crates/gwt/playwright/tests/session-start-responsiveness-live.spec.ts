import { test, expect } from "@playwright/test";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

test.describe("SessionStart foreground responsiveness (#5042)", () => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  test.skip(!base || !home, "requires an isolated browser-check checkout instance");
  test.setTimeout(90_000);

  test("foreground dispatch remains within budget during a focus burst", async ({ page }, info) => {
    await withLiveGwtBackendLock(base!, info, async () => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      let windows: Array<{ id: string; preset: string; z_index: number }> = [];
      let targetId: string | undefined;
      let lastFocusOrder: number | undefined;
      page.on("websocket", socket => socket.on("framereceived", ({ payload }) => {
        const event = JSON.parse(String(payload));
        if (event.kind === "workspace_state") {
          windows = (event.workspace.tabs ?? []).flatMap((tab: any) => tab.workspace?.windows ?? []);
          lastFocusOrder = windows.find(window => window.id === targetId)?.z_index;
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
      const known = new Set(windows.map(window => window.id));
      // A static surface avoids adding initial log loading and live log
      // rendering to the dispatch measurement as FocusWindow records actions.
      await sendLiveGwtEvent(page, { kind: "create_window", preset: "agent_kanban" });
      await expect.poll(() => windows.find(window => window.preset === "agent_kanban" && !known.has(window.id))?.id).toBeTruthy();
      targetId = windows.find(window => window.preset === "agent_kanban" && !known.has(window.id))!.id;
      const initialOrder = windows.find(window => window.id === targetId)!.z_index;
      try {
        const startedAt = Date.now();
        await page.evaluate(({ count, id }) => {
          for (let index = 1; index <= count; index++) {
            window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail: {
              kind: "focus_window", id,
            } }));
          }
        }, { count, id: targetId });
        await expect.poll(() => lastFocusOrder, { timeout: 30_000 }).toBeGreaterThanOrEqual(initialOrder + count);

        const samples = async (): Promise<number[]> => {
          const directory = join(home!, ".gwt", "logs");
          const names = (await readdir(directory)).filter(name => name.startsWith("gwt.log"));
          const logs = await Promise.all(names.map(name => readFile(join(directory, name), "utf8")));
          return logs.flatMap(log => log.split("\n").flatMap(line => {
            try {
              const row = JSON.parse(line);
              return row.target === "gwt.frontend.timing" && row.fields?.event === "FocusWindow"
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
        expect(errors).toEqual([]);
        const screenshot = info.outputPath(`${theme}-workspace.png`);
        await page.screenshot({ path: screenshot });
        await info.attach(`${theme}-workspace`, { path: screenshot, contentType: "image/png" });
      } finally {
        await sendLiveGwtEvent(page, { kind: "close_window", id: targetId });
      }
    });
  });
});
