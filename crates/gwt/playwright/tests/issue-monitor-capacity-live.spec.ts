/* Issue #3620 — machine-aware capacity and manual override through the real
 * checkout backend. Run headed in both themes against browser-check's fresh
 * HOME. CHECK_HOME/issue-3620-isolated.json contains {project, repo_hash, url};
 * CHECK_HOME/url identifies the served checkout process. No agents are launched.
 */
import { readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const PROJECT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";

async function isolatedPreferences() {
  expect(CHECK_HOME, "requires browser-check's fresh HOME").not.toBe("");
  expect(PROJECT, "requires the launch checkout").not.toBe("");
  const home = await realpath(CHECK_HOME);
  const project = await realpath(PROJECT);
  expect(home).not.toBe(await realpath(homedir()));
  const marker = JSON.parse(await readFile(join(home, "issue-3620-isolated.json"), "utf8"));
  expect(await realpath(marker.project)).toBe(project);
  expect(marker.repo_hash).toMatch(/^[a-zA-Z0-9_-]+$/);
  const url = (await readFile(join(home, "url"), "utf8")).trim();
  expect(new URL(BASE).origin).toBe(new URL(url).origin);
  expect(new URL(marker.url).origin).toBe(new URL(url).origin);
  const directory = join(home, ".gwt/projects", marker.repo_hash, "project-state");
  const pm = JSON.parse(await readFile(join(directory, "pm.json"), "utf8"));
  expect(pm.settings.auto_start).toBe(false);
  const preferences = join(directory, "issue-monitor.json");
  expect(JSON.parse(await readFile(preferences, "utf8")).enabled).toBe(false);
  return preferences;
}

async function latestStatus(page: Page): Promise<any> {
  return page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
    .filter(({ payload }: any) => payload.kind === "issue_monitor_status").at(-1)?.payload.status);
}

async function expectOverride(page: Page, override: number | null) {
  await expect.poll(() => latestStatus(page), { timeout: 75_000, intervals: [1_000, 2_000] })
    .toMatchObject({ max_active_agents_override: override });
  return latestStatus(page);
}

test.describe("Issue Monitor machine capacity (live backend)", () => {
  test.skip(!BASE, "requires an isolated checkout gwt instance");
  test.setTimeout(900_000);
  test.use({ viewport: { width: 1440, height: 1100 } });

  test("displays machine usage, allows manual excess and restores Auto after reload", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      const preferences = await isolatedPreferences();
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator("#close-project-button")).toBeVisible();
      const windows = page.locator('.workspace-window[data-preset="issue"]');
      const created = !(await windows.count());
      if (created) await sendLiveGwtEvent(page, {
        kind: "create_window", preset: "issue", bounds: { x: 60, y: 50, width: 1120, height: 930 },
      });
      await expect(windows.first()).toBeVisible();
      const windowId = await windows.first().getAttribute("data-id");
      const surface = page.locator(`.workspace-window[data-id="${windowId}"]`);
      await sendLiveGwtEvent(page, {
        kind: "update_window_geometry", id: windowId,
        geometry: { x: 60, y: 50, width: 720, height: 420 }, cols: 80, rows: 24,
      });
      const mode = surface.locator('[data-role="monitor-capacity-mode"]');
      const input = surface.getByRole("spinbutton", { name: "Max active agents", exact: true });
      const budget = surface.locator(".knowledge-monitor-capacity");
      const warning = surface.locator(".knowledge-monitor-capacity-warning");
      // Initial target measurement advances in bounded background ticks; this
      // checkout took 419s cold. Keep UI action deadlines separate at 75s.
      await expect.poll(() => latestStatus(page), { timeout: 600_000 })
        .toMatchObject({ enabled: false, agent_capacity: { measurement_complete: true } });
      const before = await latestStatus(page);
      try {
        if (before.max_active_agents_override !== null) {
          await surface.getByRole("button", { name: "Use Auto", exact: true }).click();
          await expectOverride(page, null);
        }
        await expect(mode).toHaveText("Auto");
        const automatic = await latestStatus(page);
        const capacity = automatic.agent_capacity;
        await expect(input).toHaveValue(String(automatic.max_active_agents));
        await budget.locator("summary").click();
        // Measurements remain live during the journey; compare the rendered
        // projection with the latest backend event in one browser observation.
        await expect.poll(() => page.evaluate((id) => {
          const status = ((window as any).__gwtPlaywrightMessages ?? [])
            .filter(({ payload }: any) => payload.kind === "issue_monitor_status").at(-1)?.payload.status;
          const current = status?.agent_capacity;
          const text = document.querySelector(`.workspace-window[data-id="${id}"] .knowledge-monitor-capacity`)?.textContent ?? "";
          if (!current) return false;
          const expected = [
            `${current.recommended_worker_limit} monitor workers`,
            `${current.recommended_implementation_count} implementation agents`,
            `${current.recommended_total_count} total including PM`,
            `Machine live: ${current.machine_live_agents}`,
            `Other projects: ${current.other_live_agents}`,
            `PM: ${current.own_pm_agents}`,
            `GUI CPU reserved: ${current.gui_cpu_millicores / 1000} cores`,
            ...current.constraints.map((constraint: any) => constraint.reason),
          ];
          return current.constraints.map((constraint: any) => constraint.resource).sort().join(",") === "cpu,disk,ram"
            && expected.every(value => text.includes(value));
        }, windowId)).toBe(true);

        // GUI CPU usage can change during the journey. Stay above the physical
        // CPU ceiling so the warning remains required for every fresh sample.
        const machine = JSON.parse(await readFile(join(CHECK_HOME, ".gwt/machine-state/agent-capacity.json"), "utf8"));
        expect(machine.performance_cores).toBeGreaterThan(0);
        const manual = Math.max(machine.performance_cores, capacity.recommended_worker_limit) + 2;
        await input.fill(String(manual));
        await input.press("Tab");
        await expectOverride(page, manual);
        await expect(mode).toHaveText("Manual");
        await expect(input).toHaveValue(String(manual));
        await expect.poll(async () => JSON.parse(await readFile(preferences, "utf8")))
          .toMatchObject({ max_active_agents_mode: "manual", max_active_agents: manual });
        await expect(warning).toBeVisible();
        await expect.poll(() => page.evaluate(({ id, value }) => {
          const status = ((window as any).__gwtPlaywrightMessages ?? [])
            .filter(({ payload }: any) => payload.kind === "issue_monitor_status").at(-1)?.payload.status;
          const current = status?.agent_capacity;
          const text = document.querySelector(`.workspace-window[data-id="${id}"] .knowledge-monitor-capacity-warning`)?.textContent ?? "";
          return !!current && text.includes(`${value - current.recommended_worker_limit} agents above recommendation`)
            && text.includes(current.limiting_constraint.toUpperCase());
        }, { id: windowId, value: manual })).toBe(true);
        await expect(warning).toContainText("Verification may not finish. Timing-dependent test failures may block unrelated PRs.");
        await expect(warning).toBeInViewport({ ratio: 1 });
        await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
        await testInfo.attach(`agent-capacity-warning-${testInfo.project.name}`, { body: await surface.screenshot(), contentType: "image/png" });
        await sendLiveGwtEvent(page, {
          kind: "update_window_geometry", id: windowId,
          geometry: { x: 60, y: 50, width: 1120, height: 930 }, cols: 80, rows: 24,
        });
        await expect.poll(() => surface.evaluate(element => element.getBoundingClientRect().height)).toBeGreaterThan(850);
        await testInfo.attach(`agent-capacity-${testInfo.project.name}`, { body: await surface.screenshot(), contentType: "image/png" });

        await gotoLiveGwt(page, BASE, { enableTestBridge: true });
        await expectOverride(page, manual);
        await expect(mode).toHaveText("Manual");
        await expect(input).toHaveValue(String(manual));
        await surface.getByRole("button", { name: "Use Auto", exact: true }).click();
        const restored = await expectOverride(page, null);
        await expect(mode).toHaveText("Auto");
        await expect(input).toHaveValue(String(restored.max_active_agents));
        await expect(warning).toBeHidden();
        await expect.poll(async () => JSON.parse(await readFile(preferences, "utf8")))
          .toMatchObject({ max_active_agents_mode: "auto", max_active_agents: 1 });
        await gotoLiveGwt(page, BASE, { enableTestBridge: true });
        await expectOverride(page, null);
        await expect(mode).toHaveText("Auto");
        expect((await latestStatus(page)).enabled).toBe(false);
      } finally {
        await sendLiveGwtEvent(page, { kind: "set_issue_monitor_max_active_agents", max_active_agents: before.max_active_agents_override });
        await expectOverride(page, before.max_active_agents_override);
        if (created && !page.isClosed()) await sendLiveGwtEvent(page, { kind: "close_window", id: windowId });
      }
      expect(errors, "zero console/page errors").toEqual([]);
    });
  });
});
