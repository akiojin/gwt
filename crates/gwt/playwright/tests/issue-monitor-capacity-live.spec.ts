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
      const machinePreferences = join(CHECK_HOME, ".gwt/machine-state/agent-settings.json");
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
      await expect(page.locator("#close-project-button")).toBeVisible();
      const windows = page.locator('.workspace-window[data-preset="settings"]');
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
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      const createdSettings = !(await settings.count());
      const issues = settings.locator('[data-settings-panel="issues"]');
      const input = issues.getByLabel("Max active", { exact: true });
      const mode = issues.getByLabel("Max active mode", { exact: true });
      const showIssues = () => page.getByRole("button", { name: "Issues", exact: true }).click();
      const openIssuesSettings = async () => {
        await showIssues();
        await expect(settings.locator('[data-settings-tab="issues"]')).toHaveAttribute("aria-selected", "true");
        await expect(issues).toBeVisible();
        const options = issues.locator(".settings-issues-options");
        if (!await options.evaluate(element => (element as HTMLDetailsElement).open)) {
          await surface.getByRole("button", { name: "Issues settings", exact: true }).click();
        }
        await expect(options).toHaveAttribute("open", "");
      };
      const notice = page.locator('.notification-center__item[data-error-key="issue-monitor:capacity"]');
      const budget = notice.locator(".notification-center__details");
      const warning = surface.locator(".knowledge-monitor-capacity-badge");
      // Initial target measurement advances in bounded background ticks; this
      // checkout took 419s cold. Keep UI action deadlines separate at 75s.
      await expect.poll(() => latestStatus(page), { timeout: 600_000 })
        .toMatchObject({ enabled: false, agent_capacity: { measurement_complete: true } });
      const before = await latestStatus(page);
      try {
        await openIssuesSettings();
        await expect(issues).toContainText("All projects on this machine.");
        await expect.poll(() => latestStatus(page)).toMatchObject({ max_active_agents_scope: "machine" });
        if (before.max_active_agents_override !== null) {
          await mode.selectOption("auto");
          await expectOverride(page, null);
        }
        await expect(mode).toHaveValue("auto");
        await expect(input).toBeDisabled();
        const automatic = await latestStatus(page);
        const capacity = automatic.agent_capacity;
        await expect(input).toHaveValue(String(automatic.max_active_agents));
        await expect(issues).toContainText(`This project allocation: ${automatic.max_active_agents}`);
        await expect(issues).toContainText(`Other project workers: ${Math.max(0, capacity.other_live_agents - capacity.other_pm_agents)}`);
        await showIssues();
        await warning.click();
        await budget.locator("summary").click();
        // Measurements remain live during the journey; compare the rendered
        // projection with the latest backend event in one browser observation.
        await expect.poll(() => page.evaluate((id) => {
          const status = ((window as any).__gwtPlaywrightMessages ?? [])
            .filter(({ payload }: any) => payload.kind === "issue_monitor_status").at(-1)?.payload.status;
          const current = status?.agent_capacity;
          // Healthy details are an explicitly requested snapshot, not a passive notification.
          (document.querySelector(`.workspace-window[data-id="${id}"] .knowledge-monitor-capacity-badge`) as HTMLButtonElement)?.click();
          const text = document.querySelector('.notification-center__item[data-error-key="issue-monitor:capacity"]')?.textContent ?? "";
          if (!current) return false;
          const expected = [
            `${current.recommended_worker_limit} monitor workers`,
            `${current.recommended_implementation_count} implementation agents`,
            `${current.recommended_total_count} total including PM`,
            `Machine live: ${current.machine_live_agents}`,
            `Other projects: ${current.other_live_agents}`,
            `PM: ${current.own_pm_agents}`,
            `GUI CPU reserved: ${current.gui_cpu_millicores / 1000} cores`,
            ...current.constraints.flatMap((constraint: any) => constraint.reason.split(/[;\n]+/).map((part: string) => part.trim()).filter(Boolean)),
          ];
          return current.constraints.map((constraint: any) => constraint.resource).sort().join(",") === "cpu,disk,ram"
            && expected.every(value => text.includes(value));
        }, windowId)).toBe(true);
        await page.getByRole("button", { name: "Close notifications", exact: true }).click();

        // GUI CPU usage can change during the journey. Stay above the physical
        // CPU ceiling so the warning remains required for every fresh sample.
        const machine = JSON.parse(await readFile(join(CHECK_HOME, ".gwt/machine-state/agent-capacity.json"), "utf8"));
        expect(machine.performance_cores).toBeGreaterThan(0);
        const manual = Math.max(machine.performance_cores, capacity.recommended_worker_limit) + 2;
        await openIssuesSettings();
        await mode.selectOption("manual");
        await expect(input).toBeEnabled();
        await input.fill(String(manual));
        await input.press("Tab");
        await issues.getByRole("button", { name: "Apply Max active", exact: true }).click();
        await expectOverride(page, manual);
        await expect(mode).toHaveValue("manual");
        await expect(input).toHaveValue(String(manual));
        await expect.poll(async () => JSON.parse(await readFile(machinePreferences, "utf8")))
          .toMatchObject({ mode: "manual", manual_limit: manual });
        await expect.poll(async () => Object.keys(JSON.parse(await readFile(preferences, "utf8"))))
          .not.toContain("max_active_agents_mode");
        await testInfo.attach(`shared-agent-settings-${testInfo.project.name}`, {
          body: await issues.screenshot(), contentType: "image/png",
        });
        await showIssues();
        await expect(warning).toBeVisible();
        await expect.poll(() => page.evaluate(({ id, value }) => {
          const status = ((window as any).__gwtPlaywrightMessages ?? [])
            .filter(({ payload }: any) => payload.kind === "issue_monitor_status").at(-1)?.payload.status;
          const current = status?.agent_capacity;
          const text = document.querySelector('.notification-center__item[data-error-key="issue-monitor:capacity"] .notification-center__message')?.textContent ?? "";
          const machineWorkers = current && Math.max(0, current.machine_budget
            - current.other_pm_agents - Math.max(current.own_pm_agents, 1));
          return !!current && text.includes(`${value - machineWorkers} over recommended`);
        }, { id: windowId, value: manual })).toBe(true);
        await expect(notice).toContainText("Lower Max active if slow.");
        await expect(warning).toHaveAttribute("data-tone", "info");
        await expect(budget).toContainText("Verification may not finish. Timing-dependent test failures may block unrelated PRs.");
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
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
        await expectOverride(page, manual);
        await openIssuesSettings();
        await expect(mode).toHaveValue("manual");
        await expect(input).toHaveValue(String(manual));
        await test.step("Issue #5227: grouped diagnostics preserve saved Manual 9", async () => {
          // The UI edit/save/reload path is covered above. Establish this
          // diagnostic fixture through the real backend before projecting it.
          await sendLiveGwtEvent(page, { kind: "set_issue_monitor_max_active_agents", max_active_agents: 9 });
          await expectOverride(page, 9);
          await expect.poll(async () => JSON.parse(await readFile(machinePreferences, "utf8")))
            .toMatchObject({ mode: "manual", manual_limit: 9 });
          await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
          const saved = await expectOverride(page, 9);
          await openIssuesSettings();
          await expect(mode).toHaveValue("manual");
          await expect(input).toHaveValue("9");
          await showIssues();
          const diskReason = "Disk measurement unavailable; check that the worktree volume is mounted and free at least 4 GiB.";
          const reason = `inventory_uncertain: child_identity_missing ×37; ${diskReason}`;
          try {
            // Exercise the existing incoming-status renderer. Rust tests cover
            // inventory aggregation; the real backend above owns persistence.
            await page.evaluate(status => {
              const socket = (window as any).__gwtPlaywrightSockets.find((candidate: WebSocket) =>
                candidate.readyState === WebSocket.OPEN && candidate.url.includes("repo_hash="));
              if (!socket) throw new Error("the live Project socket is unavailable");
              const fixture = new MessageEvent("message", { data: JSON.stringify({ kind: "issue_monitor_status", status }) });
              // Keep periodic live measurements from replacing this projection
              // during assertions. Navigation below removes the page-local filter.
              socket.addEventListener("message", (event: MessageEvent) => {
                if (event !== fixture && JSON.parse(String(event.data)).kind === "issue_monitor_status") {
                  event.stopImmediatePropagation();
                }
              }, { capture: true });
              socket.dispatchEvent(fixture);
            }, { ...saved, agent_capacity: {
              ...saved.agent_capacity, measurement_complete: false, machine_budget: null,
              recommended_worker_limit: 0, reason,
              constraints: [{ resource: "disk", capacity: null, binding: true, reason: diskReason }],
            } });
            await expect(warning).toBeVisible();
            await expect(notice).toContainText("Manual limit 9; no action needed.");
            await expect(warning).not.toContainText(reason);
            await expect(warning).toHaveAttribute("data-tone", "info");
            const text = await warning.textContent() ?? "";
            expect([...text].length, "compact default summary").toBeLessThanOrEqual(130);
            expect(text).not.toContain("child_identity_missing");
            await expect(budget).toContainText(reason);
            await expect(mode).toHaveValue("manual");
            await expect(input).toHaveValue("9");
            await expect(warning).toBeInViewport({ ratio: 1 });
            await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
            await testInfo.attach(`grouped-capacity-warning-${testInfo.project.name}`, { body: await surface.screenshot(), contentType: "image/png" });
          } finally {
            await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
            await expectOverride(page, 9);
          }
        });
        await openIssuesSettings();
        await mode.selectOption("auto");
        const restored = await expectOverride(page, null);
        await expect(mode).toHaveValue("auto");
        await expect(input).toBeDisabled();
        await expect(input).toHaveValue(String(restored.max_active_agents));
        await showIssues();
        await expect(warning).toHaveText(/Capacity \d+ · Details/);
        await expect.poll(async () => JSON.parse(await readFile(machinePreferences, "utf8")))
          .toMatchObject({ mode: "auto", manual_limit: null });
        await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
        await expectOverride(page, null);
        await openIssuesSettings();
        await expect(mode).toHaveValue("auto");
        await expect(input).toBeDisabled();
        expect((await latestStatus(page)).enabled).toBe(false);
      } finally {
        await sendLiveGwtEvent(page, { kind: "set_issue_monitor_max_active_agents", max_active_agents: before.max_active_agents_override });
        await expectOverride(page, before.max_active_agents_override);
        if (createdSettings && !page.isClosed()) {
          const settingsId = await settings.getAttribute("data-id");
          if (settingsId) await sendLiveGwtEvent(page, { kind: "close_window", id: settingsId });
        }
        if (created && !page.isClosed()) await sendLiveGwtEvent(page, { kind: "close_window", id: windowId });
      }
      expect(errors, "zero console/page errors").toEqual([]);
    });
  });

  test("Issue #5284: compact capacity summary, collapsed details and current errors", async ({ page }, testInfo) => {
    test.setTimeout(120_000);
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      await isolatedPreferences();
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await page.bringToFront();
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
      const windows = page.locator('.workspace-window[data-preset="settings"]');
      if (!(await windows.count())) await sendLiveGwtEvent(page, {
        kind: "create_window", preset: "issue", bounds: { x: 50, y: 40, width: 1120, height: 850 },
      });
      const surface = windows.first();
      await expect(surface).toBeVisible();
      const saved = await latestStatus(page) ?? {};
      // Rust stop-only tests cover the actual stop/state projection. Feed its
      // wire result through the real checkout's incoming renderer here; no
      // agents or production state are used by this presentation scenario.
      const apply = async (status: any) => page.evaluate(value => {
        // The existing page-scoped injection bridge also suppresses live
        // monitor updates, including frames queued before this fixture.
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
          detail: { kind: "issue_monitor_status", status: value },
        }));
      }, status);
      const repeated = Array(37).fill("inventory_uncertain: child_identity_missing").join("; ");
      const reason = `inventory_complete=false; own_pm=0; target: /Users/example/target; disk measurement pending; disk measurement pending; ${repeated}`;
      const status = { ...saved, enabled: true, state: "idle", active_count: 0, queue_len: 0,
        last_error: null, agent_blackout: null,
        max_active_agents: 5, max_active_agents_override: 5,
        agent_capacity: { ...saved.agent_capacity, measurement_complete: false, reason,
          recommended_worker_limit: 0, recommended_implementation_count: 0, recommended_total_count: 1,
          machine_live_agents: 0, own_live_agents: 0, other_live_agents: 0, own_pm_agents: 0, gui_cpu_millicores: 0,
          constraints: [{ resource: "disk", capacity: null, binding: true, reason: "disk measurement pending" }] } };
      await apply(status);
      const summary = surface.locator(".knowledge-monitor-capacity-badge");
      const drawer = page.locator(".notification-center-drawer");
      const notice = page.locator('.notification-center__item[data-error-key="issue-monitor:capacity"]');
      const details = notice.locator(".notification-center__details");
      const diagnostic = details.locator(".notification-center__details-content");
      const pill = surface.locator(".knowledge-monitor-pill");
      await expect(summary).toHaveText("Capacity pending · Details");
      await expect(summary).toHaveAttribute("data-tone", "info");
      await expect(surface).not.toContainText(/inventory_complete|own_pm|\/Users\//);
      await expect(surface.locator(".knowledge-monitor-capacity-warning, .knowledge-monitor-capacity")).toHaveCount(0);
      await expect(notice).toHaveCount(1);
      await expect(drawer).toBeHidden();
      await expect(pill).toHaveText("Waiting · Empty");
      const metrics = await summary.evaluate(element => {
        const style = getComputedStyle(element);
        const probe = document.createElement("span");
        probe.style.color = getComputedStyle(document.documentElement).getPropertyValue("--color-text-muted");
        document.body.append(probe);
        const infoColor = getComputedStyle(probe).color;
        probe.remove();
        return { color: style.color, infoColor };
      });
      expect(metrics.color).toBe(metrics.infoColor);
      const assertSingleLine = async () => {
        await summary.evaluate(element => { (element as HTMLElement).style.maxWidth = "220px"; });
        expect(await summary.evaluate(element => getComputedStyle(element).whiteSpace)).toBe("nowrap");
        expect(await summary.evaluate(element => element.scrollHeight <= element.clientHeight)).toBe(true);
      };
      await assertSingleLine();
      await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
      const summaryImage = testInfo.outputPath(`capacity-summary-${testInfo.project.name}.png`);
      await page.screenshot({ path: summaryImage, animations: "disabled" });
      await testInfo.attach(`capacity-summary-${testInfo.project.name}`, { path: summaryImage, contentType: "image/png" });
      await summary.focus();
      await summary.press("Enter");
      await expect(drawer).toBeVisible();
      await expect(notice).toBeFocused();
      await expect(notice.locator(".notification-center__message")).toHaveText("Capacity pending. Manual limit 5; no action needed.");
      await expect(diagnostic).toBeHidden();
      await details.locator("summary").click();
      await expect(diagnostic).toBeVisible();
      await expect(diagnostic).toContainText("/Users/example/target");
      expect((await diagnostic.textContent() ?? "").split("disk measurement pending")).toHaveLength(2);
      expect((await diagnostic.textContent() ?? "").split("child_identity_missing")).toHaveLength(2);
      const detailsImage = testInfo.outputPath(`capacity-details-${testInfo.project.name}.png`);
      await page.screenshot({ path: detailsImage, animations: "disabled" });
      await testInfo.attach(`capacity-details-${testInfo.project.name}`, { path: detailsImage, contentType: "image/png" });
      const retained = await notice.elementHandle();
      await apply({ ...status, max_active_agents: 6, max_active_agents_override: 6 });
      await expect(notice).toHaveCount(1);
      expect(await retained!.evaluate(element => element.isConnected)).toBe(true);
      await expect(notice.locator(".notification-center__message")).toContainText("Manual limit 6");
      await expect(notice.locator(".notification-center__count")).toHaveCount(0);
      await page.getByRole("button", { name: "Close notifications", exact: true }).click();
      await expect(summary).toBeFocused();
      const bell = page.getByRole("button", { name: "Notifications", exact: true });
      await expect(bell).toHaveAttribute("data-unread", "0");
      await apply({ ...status, max_active_agents: 6, max_active_agents_override: 6,
        agent_capacity: { ...status.agent_capacity, machine_live_agents: 9 } });
      await expect(diagnostic).toContainText("Machine live: 9");
      await expect(bell).toHaveAttribute("data-unread", "0");
      await apply({ ...status, state: "error", last_error: "GitHub scan failed" });
      await expect(pill).toHaveText("⚠ Error");
      await expect(surface).not.toContainText("GitHub scan failed");
      await pill.click();
      const currentError = page.locator('.notification-center__item[data-error-key="issue-monitor:last_error"]');
      await expect(currentError).toBeFocused();
      await expect(currentError).toContainText("GitHub scan failed");
      await page.getByRole("button", { name: "Close notifications", exact: true }).click();
      await apply(status);
      await expect(pill).toHaveText("Waiting · Empty");
      await expect(currentError).toHaveAttribute("data-resolved", "true");
      await apply({ ...status, agent_capacity: { ...status.agent_capacity,
        measurement_complete: true, machine_budget: 2, recommended_worker_limit: 1, limiting_constraint: "cpu,ram,disk" } });
      await expect(summary).toHaveText("Over recommended · Details");
      await expect(notice.locator(".notification-center__message")).toHaveText("4 over recommended. Lower Max active if slow.");
      await assertSingleLine();
      await apply({ ...status, max_active_agents: 5, max_active_agents_override: null });
      await expect(summary).toHaveAttribute("data-tone", "info");
      await expect(notice).toHaveAttribute("data-level", "info");
      await expect(notice.locator(".notification-center__message")).toContainText("Auto limit 5 retained");
      await summary.click();
      await expect(drawer).toBeVisible();
      await testInfo.attach(`capacity-retained-${testInfo.project.name}`, {
        body: await page.screenshot({ animations: "disabled" }), contentType: "image/png",
      });
      await page.getByRole("button", { name: "Close notifications", exact: true }).click();
      await apply({ ...status, max_active_agents: 0, max_active_agents_override: null });
      await expect(summary).toHaveAttribute("data-tone", "warning");
      await expect(notice).toHaveAttribute("data-level", "warn");
      await expect(notice.locator(".notification-center__message")).toContainText("Wait or set Max active manually.");
      await assertSingleLine();
      const narrowImage = testInfo.outputPath(`capacity-narrow-${testInfo.project.name}.png`);
      await page.screenshot({ path: narrowImage, animations: "disabled" });
      await testInfo.attach(`capacity-narrow-${testInfo.project.name}`, { path: narrowImage, contentType: "image/png" });
      // A current status badge remains useful after the operator clears history.
      await summary.click();
      await page.getByRole("button", { name: "Clear all", exact: true }).click();
      await page.getByRole("button", { name: "Close notifications", exact: true }).click();
      await summary.click();
      await expect(notice).toHaveCount(1);
      await expect(notice).toBeFocused();
      await expect(diagnostic).toBeHidden();
      await page.getByRole("button", { name: "Close notifications", exact: true }).click();
      expect(errors, "zero console/page errors").toEqual([]);
    });
  });
});
