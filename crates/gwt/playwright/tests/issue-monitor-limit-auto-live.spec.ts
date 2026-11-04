import { readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const PROJECT_STATE = process.env.GWT_PLAYWRIGHT_PROJECT_STATE ?? "";

async function latestStatus(page: Page): Promise<any> {
  return page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
    .filter(({ payload }: any) => payload.kind === "issue_monitor_status").at(-1)?.payload.status);
}

test.describe("Issue #6732: Agent limit Auto recovery (live backend)", () => {
  test.skip(!BASE, "requires browser-check's isolated checkout instance");
  test.setTimeout(180_000);
  test.use({ viewport: { width: 1440, height: 1100 } });

  test("returns a roster-managed Manual limit to Auto and preserves it after reload", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      expect(CHECK_HOME).not.toBe("");
      expect(PROJECT_STATE).not.toBe("");
      expect(await realpath(CHECK_HOME)).not.toBe(await realpath(homedir()));
      expect((await realpath(PROJECT_STATE)).startsWith(await realpath(CHECK_HOME))).toBe(true);
      const preferences = join(PROJECT_STATE, "issue-monitor.json");
      const machine = join(CHECK_HOME, ".gwt/machine-state/agent-settings.json");
      const policy = async () => JSON.parse(await readFile(machine, "utf8"));
      expect(JSON.parse(await readFile(join(PROJECT_STATE, "pm.json"), "utf8")).settings.auto_start).toBe(false);
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
      const windows = page.locator('.workspace-window[data-preset="issue"]');
      const created = !(await windows.count());
      if (created) await sendLiveGwtEvent(page, { kind: "create_window", preset: "issue" });
      await expect(windows.first()).toBeVisible();
      const windowId = await windows.first().getAttribute("data-id");
      try {
        // Each theme exercises the same transition through the real save lane.
        await sendLiveGwtEvent(page, { kind: "set_issue_monitor_max_active_agents", max_active_agents: 8 });
        await expect.poll(policy).toMatchObject({ mode: "manual", manual_limit: 8 });
        await expect.poll(() => latestStatus(page)).toMatchObject({ enabled: false, max_active_agents_override: 8 });
        const roster = JSON.parse(await readFile(preferences, "utf8")).agent_roster;
        expect(roster.members.some((member: any) => member.enabled)).toBe(true);
        const auto = windows.first().getByRole("button", { name: "Set Agent limit to Auto", exact: true });
        await auto.scrollIntoViewIfNeeded();
        await expect(auto).toBeVisible();
        await expect(auto).toBeEnabled();
        await testInfo.attach(`agent-limit-manual-${testInfo.project.name}`, {
          body: await windows.first().screenshot({ path: testInfo.outputPath("agent-limit-manual.png") }), contentType: "image/png",
        });
        await auto.click();
        await expect.poll(policy).toMatchObject({ mode: "auto", manual_limit: null,
          last_change: { source: "gui", max_active_agents_override: null } });
        await expect.poll(() => latestStatus(page)).toMatchObject({ enabled: false, max_active_agents_override: null });
        await expect(auto).toBeDisabled();
        expect((await latestStatus(page)).last_error ?? "").not.toMatch(/authority recovery|accepted control|control rejected/i);
        await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
        await testInfo.attach(`agent-limit-auto-${testInfo.project.name}`, {
          body: await windows.first().screenshot({ path: testInfo.outputPath("agent-limit-auto.png") }), contentType: "image/png",
        });
        await page.reload();
        await expect.poll(() => latestStatus(page)).toMatchObject({ enabled: false, max_active_agents_override: null });
        await expect(windows.first().getByRole("button", { name: "Set Agent limit to Auto", exact: true })).toBeDisabled();
        expect(await policy()).toMatchObject({ mode: "auto", manual_limit: null });
        expect(JSON.parse(await readFile(preferences, "utf8")).enabled).toBe(false);
      } finally {
        if (created && !page.isClosed()) await sendLiveGwtEvent(page, { kind: "close_window", id: windowId });
      }
      expect(errors, "zero console/page errors").toEqual([]);
    });
  });
});
