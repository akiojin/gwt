import { spawnSync } from "node:child_process";
import { readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const PROJECT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";

async function isolatedCli() {
  expect(CHECK_HOME, "requires a browser-check isolated HOME").not.toBe("");
  expect(PROJECT, "requires the checkout project root").not.toBe("");
  const checkHome = await realpath(CHECK_HOME);
  const project = await realpath(PROJECT);
  expect(checkHome).not.toBe(await realpath(homedir()));
  const marker = JSON.parse(await readFile(join(checkHome, "issue-4774-isolated.json"), "utf8"));
  expect(await realpath(marker.project)).toBe(project);
  const pm = JSON.parse(await readFile(join(checkHome, ".gwt/projects", marker.repo_hash, "project-state/pm.json"), "utf8"));
  expect(pm.settings.auto_start).toBe(false);
  const servedUrl = (await readFile(join(checkHome, "url"), "utf8")).trim();
  expect(new URL(BASE).origin).toBe(new URL(servedUrl).origin);
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GWT_")));
  env.HOME = checkHome;
  env.USERPROFILE = checkHome;
  return (operation: string, params: Record<string, unknown> = {}) => {
    const result = spawnSync(join(project, "target/debug/gwtd"), [], {
      cwd: project,
      env,
      input: JSON.stringify({ schema_version: 1, operation, params: { ...params, project_root: project } }),
      encoding: "utf8",
      timeout: 30_000,
    });
    expect(result.status, `${operation}: ${result.stderr}`).toBe(0);
    const envelope = JSON.parse(result.stdout);
    expect(envelope.ok).toBe(true);
    return JSON.parse(envelope.output);
  };
}

test.describe("Issue Monitor automatic tiers", () => {
  test.skip(!BASE, "requires an isolated checkout gwt instance");
  // The real disabled Monitor fallback can spend up to 60 seconds scanning.
  test.setTimeout(180_000);
  test.use({ viewport: { width: 1440, height: 1000 } });

  test("default auto tiers reach the live Issue surface without manual profile setup", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      const cli = await isolatedCli();
      const before = cli("issue.monitor.status");
      expect(before.enabled).toBe(false);
      expect(before.autonomous_mode).toBe(false);
      const saved = cli("issue.monitor.tiers.set", { auto: true });
      expect(saved.auto).toBe(true);
      expect(saved.configured_tiers).toEqual([]);
      expect(saved.tiers).toHaveLength(3);
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => {
        if (message.type() === "error") errors.push(message.text());
      });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator("#close-project-button")).toBeVisible();
      const issueWindows = page.locator('.workspace-window[data-preset="issue"]');
      let windowId = await issueWindows.count() ? await issueWindows.first().getAttribute("data-id") : null;
      const createdWindow = !windowId;
      if (!windowId) {
        await sendLiveGwtEvent(page, { kind: "create_window", preset: "issue", bounds: { x: 60, y: 50, width: 1040, height: 820 } });
        await expect(issueWindows.first()).toBeVisible();
        windowId = await issueWindows.first().getAttribute("data-id");
      }
      try {
        // Opening the Issue surface requests its status. A second explicit
        // request queues another full scan on a fresh disabled Monitor.
        const status = await page.waitForFunction(() => {
          const frame = ((window as any).__gwtPlaywrightMessages ?? []).find((entry: any) =>
            entry.payload.kind === "issue_monitor_status"
            && entry.payload.status?.launch_profile_summary === "Auto (3 tiers)");
          return frame?.payload.status;
        }, undefined, { timeout: 75_000 }).then(handle => handle.jsonValue());
        expect(status.launch_profile_summary).toBe("Auto (3 tiers)");
        expect(status.enabled).toBe(false);
        expect(status.state).not.toBe("settings_required");
        expect(status.launch_profile_source).toBe("saved");
        expect(status.active_count).toBe(0);
        const surface = page.locator(`.workspace-window[data-id="${windowId}"]`);
        await expect(surface).toBeVisible();
        const settings = surface.locator('[data-action="monitor-settings"]');
        await expect(settings).toBeVisible();
        await expect(settings).toHaveAttribute("title", /Auto \(3 tiers\)/);
        await expect(surface.locator('[data-action="monitor-setup"]')).toBeHidden();
        await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
        await page.screenshot({ path: join(CHECK_HOME, `auto-tiers-${testInfo.project.name}.png`), fullPage: true });
        const after = cli("issue.monitor.status");
        expect(after.enabled).toBe(false);
        expect(after.autonomous_mode).toBe(false);
        expect(after.active_launches).toEqual([]);
      } finally {
        if (createdWindow && !page.isClosed()) await sendLiveGwtEvent(page, { kind: "close_window", id: windowId });
      }
      expect(errors).toEqual([]);
    });
  });
});
