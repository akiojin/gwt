/** Issue #6809: observe the real checkout for five minutes with 13,000 dead stores. */
import { expect, test } from "@playwright/test";
import { readFile, readdir, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const base = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";

async function logContents(directory: string): Promise<string[]> {
  const files = await readdir(directory, { withFileTypes: true }).catch(error => {
    if (error.code === "ENOENT") return [];
    throw error;
  });
  return (await Promise.all(files.map(async file => {
    const path = join(directory, file.name);
    if (file.isDirectory()) return logContents(path);
    if (!file.name.startsWith("gwt.log")) return [];
    return [await readFile(path, "utf8")];
  }))).flat();
}

test.use({ headless: false, viewport: { width: 1440, height: 1000 } });
test.describe("Issue #6809 project store discovery", () => {
  test.skip(!base || process.env.GWT_PLAYWRIGHT_ISSUE_6809 !== "1",
    "requires issue-6809-browser-check.ps1 and its isolated 13,000-store fixture");
  test.setTimeout(420_000);

  test("stale stores stay quiet throughout five minutes of GUI operation", async ({ page }, info) => {
    expect(await realpath(home)).not.toBe(await realpath(homedir()));
    const fixture = JSON.parse(await readFile(join(home, "issue-6809-fixture.json"), "utf8"));
    expect(fixture.store_count).toBe(13_000);
    expect(fixture.owned_gui_pid).toBeGreaterThan(0);
    expect(new URL(base).origin).toBe(new URL(fixture.url).origin);
    expect(await realpath(fixture.checkout)).toBe(await realpath(process.env.GWT_PLAYWRIGHT_PROJECT_ROOT!));
    const projects = await readdir(join(home, ".gwt/projects"), { withFileTypes: true });
    expect(projects.filter(entry => entry.isDirectory())).toHaveLength(13_001);
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await gotoLiveGwt(page, base, { enableTestBridge: true });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(page.locator("#close-project-button")).toBeVisible();
    await expect(page.locator("#app-version")).toHaveText(/^v\d+\.\d+\.\d+$/);
    await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .some(({ payload }: any) => payload.kind === "workspace_state"));

    // Observe elapsed time instead of assuming the scheduler wakes on a deadline.
    // Both theme cases assert the same launch; the second case can reuse the
    // five-minute sample already completed by the first one.
    const launchedAt = Date.parse(fixture.launched_at);
    expect(Number.isFinite(launchedAt)).toBe(true);
    await expect.poll(() => Date.now() - launchedAt,
      { timeout: 330_000, intervals: [1000], message: "observe at least five minutes after the owned GUI launch" })
      .toBeGreaterThanOrEqual(300_000);
    await expect(page.locator("#close-project-button")).toBeVisible();
    const logs = [
      ...await logContents(join(home, ".gwt/logs")),
      ...await logContents(join(home, ".gwt/projects", fixture.repo_hash, "logs")),
    ];
    expect(logs.length, "the owned GUI produced structured logs").toBeGreaterThan(0);
    const warnings = logs.reduce((count, text) => count + (text.match(/no repository identity resolved/g) ?? []).length, 0);
    const evidence = { store_count: fixture.store_count, owned_gui_pid: fixture.owned_gui_pid,
      theme, launched_at: fixture.launched_at, observed_at: new Date().toISOString(),
      elapsed_ms: Date.now() - launchedAt, identity_fallback_warnings: warnings,
      console_page_errors: errors };
    await info.attach("five-minute-project-scan", { body: JSON.stringify(evidence, null, 2), contentType: "application/json" });
    await page.screenshot({ path: info.outputPath(`project-scan-${theme}.png`) });
    expect(warnings, "fewer than ten fallback warnings in the isolated five-minute GUI run").toBeLessThan(10);
    expect(errors).toEqual([]);
  });
});
