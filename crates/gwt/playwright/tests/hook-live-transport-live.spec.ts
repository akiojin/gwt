/** Issue #5542: real cold transport plus independent real GUI responsiveness.
 * The native receiver models the transport; it is not a deployed agent fleet.
 */
import { execFile } from "node:child_process";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { promisify } from "node:util";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

const runFile = promisify(execFile);
const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
const root = process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT;

test.describe("Issue #5542 hook transport", () => {
  test.skip(!base || !root, "requires the isolated Issue #5542 browser-check launcher");
  test.setTimeout(90_000);
  test("twelve cold senders deliver while the checkout UI responds", async ({ page }, info) => {
    const errors: string[] = [];
    let viewportX: number | undefined;
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("websocket", socket => socket.on("framereceived", ({ payload }) => {
      const event = JSON.parse(String(payload));
      if (event.kind === "workspace_state") {
        viewportX = event.workspace.tabs?.[0]?.workspace?.viewport?.x ?? event.workspace.viewport?.x;
      }
    }));
    await gotoLiveGwt(page, base!, { enableTestBridge: true });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(page.locator("#close-project-button")).toBeVisible();
    const suffix = process.platform === "win32" ? ".exe" : "";
    const output = info.outputPath("transport.jsonl");
    let finished = false;
    const workload = runFile("python", [join(root!, "docs/perf/issue-5542/measure.py"),
      "--binary", join(root!, `target/issue-5542-after${suffix}`),
      "--receiver", join(root!, `target/issue-5542-receiver${suffix}`),
      "--output", output, "--seconds", "10", "--workers", "12", "--modes", "current"],
      { cwd: root, windowsHide: true, timeout: 60_000 }).finally(() => { finished = true; });
    // Pacing avoids introducing the unrelated 500-event viewport stress gate.
    let responses = 0;
    try {
      while (!finished) {
        const x = (theme === "dark" ? 100_000 : 200_000) + responses;
        await sendLiveGwtEvent(page, { kind: "update_viewport", viewport: { x, y: 0, zoom: 1 } });
        await expect.poll(() => viewportX, { timeout: 30_000 }).toBe(x);
        responses++;
        await new Promise(resolve => setTimeout(resolve, 500));
      }
    } finally {
      await workload;
    }
    const summaryPath = output.replace(/\.jsonl$/, ".summary.json");
    const summary = JSON.parse(await readFile(summaryPath, "utf8"));
    expect(summary.workers).toBe(12);
    expect(summary.modes.current.samples).toBeGreaterThanOrEqual(12);
    expect(summary.modes.current.failures).toBe(0);
    expect(responses).toBeGreaterThan(0);
    expect(errors).toEqual([]);
    await info.attach(`${theme}-transport`, { path: summaryPath, contentType: "application/json" });
    await page.screenshot({ path: info.outputPath(`${theme}-checkout.png`) });
    console.log(`${theme}: ${summary.modes.current.samples} delivered, GUI responses=${responses}, errors=0`);
  });
});
