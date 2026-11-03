/* Issue #6678: real checkout GUI observation. This deliberately keeps PM and
 * Monitor off; the busy-owner Deferred-loop regression is a separate Rust test.
 */
import { expect, test } from "@playwright/test";
import { readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { performance } from "node:perf_hooks";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
const state = process.env.GWT_PLAYWRIGHT_PROJECT_STATE;

test.use({ headless: false });

test("isolated GUI keeps PM diagnostic volume bounded", async ({ page }, info) => {
  test.skip(!base || !home || !state, "requires issue-6678-browser-check.ps1");
  test.setTimeout(720_000);
  const theme = info.project.name.includes("light") ? "light" : "dark";
  // The dark run is the ten-minute volume measurement. Light verifies the same
  // real GUI, console and page contracts without claiming another ten minutes.
  const durationMs = theme === "dark" ? 600_000 : 10_000;
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  await gotoLiveGwt(page, base!);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await expect(page.locator("#close-project-button")).toBeVisible();
  const startedAt = new Date().toISOString();
  const started = performance.now();
  let nextLivenessCheck = 0;
  while (performance.now() - started < durationMs) {
    if (performance.now() - started >= nextLivenessCheck) {
      const response = await page.request.head(base!, { timeout: 5000 });
      expect(response.status(), "owned GUI remains available throughout the sample").toBe(200);
      nextLivenessCheck += 10_000;
    }
    await page.waitForTimeout(Math.min(1000, durationMs - (performance.now() - started)));
    expect(errors).toEqual([]);
  }
  const elapsedMs = performance.now() - started;
  const endedAt = new Date().toISOString();
  let matchingLines = 0;
  let inspectedLines = 0;
  let bytes = 0;
  const directories = [join(home!, ".gwt", "logs"), join(state!, "..", "logs")];
  for (const directory of directories) {
    for (const entry of await readdir(directory, { withFileTypes: true }).catch(() => [])) {
      if (!entry.isFile() || !/^gwt\.log\.\d{4}-\d{2}-\d{2}$/.test(entry.name)) continue;
      const contents = await readFile(join(directory, entry.name), "utf8");
      const lines = contents.split("\n");
      if (!contents.endsWith("\n")) lines.pop();
      for (const line of lines.filter(Boolean)) {
        const row = JSON.parse(line);
        const timestamp = Date.parse(row.timestamp);
        if (timestamp < Date.parse(startedAt) || timestamp > Date.parse(endedAt)) continue;
        inspectedLines++;
        bytes += Buffer.byteLength(line) + 1;
        if (row.target === "gwt.pm.refresh") matchingLines++;
      }
    }
  }
  const preferences = JSON.parse(await readFile(join(state!, "pm.json"), "utf8"));
  const monitor = JSON.parse(await readFile(join(state!, "issue-monitor.json"), "utf8"));
  expect(preferences.settings.auto_start).toBe(false);
  expect(monitor.enabled).toBe(false);
  await expect(page.locator("#close-project-button")).toBeVisible();
  expect(elapsedMs).toBeGreaterThanOrEqual(durationMs);
  expect(matchingLines).toBeLessThanOrEqual(350 * elapsedMs / 1000 / 100);
  expect(errors).toEqual([]);
  const evidence = { theme, started_at: startedAt, ended_at: endedAt, elapsed_ms: elapsedMs,
    measured_ten_minutes: elapsedMs >= 600_000, headed: true, console_page_errors: errors,
    inspected_lines: inspectedLines, pm_refresh_lines: matchingLines, diagnostic_bytes: bytes,
    baseline_lines_per_second: 350, maximum_fraction: 0.01,
    scenario: "isolated idle GUI; PM auto-start and Monitor disabled",
    limitation: "Different workload from the reported busy-owner loop; root regression is covered separately by Rust.",
    fixture: JSON.parse(await readFile(join(home!, "fixture.json"), "utf8")) };
  const path = info.outputPath(`pm-refresh-log-budget-${theme}.json`);
  await writeFile(path, JSON.stringify(evidence, null, 2));
  await info.attach("PM refresh log observation", { path, contentType: "application/json" });
  await info.attach(`GUI ${theme}`, { body: await page.screenshot(), contentType: "image/png" });
});
