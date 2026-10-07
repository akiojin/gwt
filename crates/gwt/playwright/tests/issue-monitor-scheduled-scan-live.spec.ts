import { readFile, readdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_5119_CHECK_HOME ?? "";
const PREFS = process.env.GWT_5119_PREFS_PATH ?? "";
const LOGS = process.env.GWT_5119_LOG_DIR ?? "";
const COMPLETIONS = ["IssueMonitorScheduledScanComplete", "IssueMonitorScheduledScanPrepared"];

test.describe("scheduled Issue Monitor completion (isolated live backend)", () => {
  test.skip(!BASE || !CHECK_HOME, "requires the isolated checkout launcher");
  test.setTimeout(380_000);

  test("scheduled completion stays responsive and each GUI dispatch fits 30 ms", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      expect(resolve(PREFS).startsWith(resolve(CHECK_HOME) + "\\")
        || resolve(PREFS).startsWith(resolve(CHECK_HOME) + "/")).toBe(true);
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => {
        if (message.type() === "error") errors.push(message.text());
      });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute(
        "data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark",
      );
      const before = await dispatchSamples();
      // Only isolated controls change. Automatic launch remains disabled and
      // the impossible label excludes all real Issues from admission.
      await writeFile(PREFS, JSON.stringify({
        enabled: true, launch_auto: false, max_active_agents: 1,
        allowed_labels: ["gwt-5119-isolated-no-candidates"],
        priority_order: [], auto_apply_updates: false,
      }));
      await expect.poll(async () => {
        const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
        // This GUI-owned query always replies; monitor status can be omitted
        // by a live daemon and its synchronous IPC is a separate surface.
        await sendLiveGwtEvent(page, { kind: "list_windows" });
        await page.waitForFunction(cursor => (window as any).__gwtPlaywrightMessages.some(
          (entry: any) => entry.sequence > cursor && entry.payload.kind === "window_list",
        ), cursor, { timeout: 5_000 });
        const current = await dispatchSamples();
        return COMPLETIONS.every(name => current[name].length > before[name].length);
      }, { timeout: 345_000, intervals: [5_000] }).toBe(true);
      const after = await dispatchSamples();
      const measured = Object.fromEntries(COMPLETIONS.map(name => [name,
        after[name].slice(before[name].length),
      ]));
      for (const samples of Object.values(measured)) {
        expect(samples.length).toBeGreaterThan(0);
        expect(Math.max(...samples)).toBeLessThanOrEqual(30);
      }
      await testInfo.attach("all-completion-dispatch-samples", {
        body: JSON.stringify(measured, null, 2), contentType: "application/json",
      });
      const screenshot = testInfo.outputPath("scheduled-completion.png");
      await page.screenshot({ path: screenshot });
      await testInfo.attach("scheduled-completion", { path: screenshot, contentType: "image/png" });
      expect(errors).toEqual([]);
    });
  });
});

async function dispatchSamples(): Promise<Record<string, number[]>> {
  const samples: Record<string, number[]> = Object.fromEntries(COMPLETIONS.map(name => [name, []]));
  for (const name of (await readdir(LOGS)).filter(name => name.startsWith("gwt.log"))) {
    // A non-blocking log writer may be appending the last record right now.
    for (const line of (await readFile(join(LOGS, name), "utf8")).split("\n").slice(0, -1)) {
      if (!line.includes("gwt.frontend.timing")) continue;
      const record = JSON.parse(line);
      if (COMPLETIONS.includes(record.fields?.event)) {
        samples[record.fields.event].push(record.fields.elapsed_ms);
      }
    }
  }
  return samples;
}
