/** Issue #5322 / SPEC #3200: persisted Settings scopes against two real projects. */
import { expect, test, type Locator, type Page, type TestInfo } from "@playwright/test";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import {
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

type Project = { root: string; preferences: string; hash: string };
type Backend = { url: string; home: string; pid: number; projects: Project[] };
const BACKENDS: Record<string, Backend> = JSON.parse(process.env.GWT_ISSUE_5322_BACKENDS ?? "{}");
const readPreferences = async (project: Project) => JSON.parse(await readFile(project.preferences, "utf8"));
const themeFor = (info: TestInfo) => info.project.name.endsWith("light") ? "light" : "dark";
const settings = (page: Page) => page.locator('.workspace-window[data-preset="settings"]');
const panel = (page: Page, section: string) => section === "agents"
  ? settings(page).locator("[data-settings-candidates]")
  : settings(page).locator(`[data-settings-panel="${section}"]`);
const autonomous = (page: Page) => panel(page, "issues").locator('[data-action="monitor-autonomous"]');

async function openSettings(page: Page, section: string): Promise<void> {
  await page.bringToFront();
  // The disposable Normal Git fixture may restore this unrelated focus trap.
  // Release it before the first Settings disclosure pointer action.
  await closeFixtureMigrationDrawer(page);
  await page.evaluate(section => document.dispatchEvent(new CustomEvent("settings:open", {
    detail: { section },
  })), section);
  await expect(settings(page)).toBeVisible();
  const selectedSection = section === "agents" ? "issues" : section;
  await expect(settings(page).locator(`[data-settings-tab="${selectedSection}"]`)).toHaveAttribute("aria-selected", "true");
  if (selectedSection === "issues") {
    const options = panel(page, "issues").locator(".settings-issues-options");
    if (!await options.evaluate(element => (element as HTMLDetailsElement).open)) await options.locator(":scope > summary").click();
  }
  await expect(panel(page, section)).toBeVisible();
}

async function editNumber(control: Locator, value: string): Promise<void> {
  await control.fill(value);
  await control.press("Tab");
}

async function toggleMonitor(page: Page, enabled: boolean): Promise<void> {
  const before = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
  await panel(page, "issues").locator('[data-action="monitor-toggle"]').click();
  // Daemon authorizing controls publish asynchronously; wait for this
  // project's new authoritative response before asserting its durable write.
  await page.waitForFunction(({ before, enabled }) => ((window as any).__gwtPlaywrightMessages ?? [])
    .some(({ sequence, payload }: any) => sequence > before
      && payload.kind === "issue_monitor_status" && payload.status.enabled === enabled),
  { before, enabled }, { timeout: 30_000 });
}

async function selectCapacityMode(page: Page, mode: "auto" | "manual"): Promise<void> {
  const control = panel(page, "issues").getByLabel("Max active mode", { exact: true });
  // Match native dropdown interaction so status echoes retain the editing control.
  await control.focus();
  await expect(control).toBeFocused();
  const before = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
  await control.selectOption(mode);
  await page.waitForFunction(({ before, mode }) => ((window as any).__gwtPlaywrightMessages ?? [])
    .some(({ sequence, payload }: any) => sequence > before && payload.kind === "issue_monitor_status"
      && (payload.status.max_active_agents_override == null ? "auto" : "manual") === mode),
  { before, mode }, { timeout: 30_000 });
}

async function closeFixtureMigrationDrawer(page: Page): Promise<void> {
  // These disposable Normal Git projects intentionally keep their original
  // identities. Close only the unrelated startup drawer through its renderer
  // so its focus trap is released; never migrate/create a gwt worktree here.
  await page.evaluate(async () => {
    const modalEl = document.getElementById("migration-modal");
    if (!modalEl?.classList.contains("open")) return;
    const modulePath = "/migration-modal.js";
    const { renderMigrationModal } = await import(modulePath);
    renderMigrationModal({ modalEl, dialogEl: modalEl.querySelector(".modal-shell"),
      state: { migrationModal: { open: false } } });
  });
  await expect(page.locator("#migration-modal")).not.toHaveClass(/open/);
}

async function screenshot(page: Page, section: string, info: TestInfo): Promise<void> {
  await openSettings(page, section);
  await panel(page, section).scrollIntoViewIfNeeded();
  const name = `settings-${section}-${themeFor(info)}`;
  await info.attach(name, { body: await settings(page).screenshot(), contentType: "image/png" });
  if (process.env.GWT_L3_SCREENSHOT_DIR) {
    await settings(page).screenshot({ path: join(process.env.GWT_L3_SCREENSHOT_DIR, `${name}.png`) });
  }
}

test.describe("Issue #5322 Settings", () => {
  test.skip(Object.keys(BACKENDS).length === 0, "requires verify-issue-5322.mjs isolated backends");
  test.setTimeout(180_000);
  test.use({ viewport: { width: 1500, height: 1100 }, actionTimeout: 15_000 });

  test("routes Issues, persists project controls and full profiles, and shares application settings", async ({ page, context }, info) => {
    const backend = BACKENDS[themeFor(info)];
    const [a, b] = backend.projects;
    expect(a.hash).not.toBe(b.hash);
    await withLiveGwtBackendLock(backend.url, info, async () => {
      const pageB = await context.newPage();
      const errors: string[] = [];
      const workspaceFrames: Array<Record<string, unknown>> = [];
      for (const [index, current] of [page, pageB].entries()) {
        current.on("pageerror", error => errors.push(error.message));
        current.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
        current.on("websocket", socket => {
          socket.on("framesent", ({ payload }) => {
            try {
              const frame = JSON.parse(String(payload));
              if (["create_window", "focus_window", "frontend_ready"].includes(frame.kind)
                || frame.kind.startsWith("set_issue_monitor_")) {
                workspaceFrames.push({ project: index ? "B" : "A", direction: "sent", frame });
              }
            } catch { /* unrelated non-JSON frames */ }
          });
          socket.on("framereceived", ({ payload }) => {
            try {
              const frame = JSON.parse(String(payload));
              if (frame.kind === "workspace_state") {
                workspaceFrames.push({ project: index ? "B" : "A", direction: "received", frame });
              }
            } catch { /* unrelated non-JSON frames */ }
          });
        });
      }
      try {
        for (const [current, project] of [[page, a], [pageB, b]] as const) {
          await gotoLiveGwt(current, backend.url, { enableTestBridge: true, suppressUpdateApplyStart: true });
          await openLiveGwtProject(current, project.root);
          await current.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
            .some(({ payload }: any) => payload.kind === "workspace_state"));
          await openSettings(current, "issues");
          await expect(autonomous(current)).toBeEnabled();
          await expect(panel(current, "issues")).toContainText(project.root);
        }
        expect(page.url()).not.toBe(pageB.url());
        const originalB = await readPreferences(b);
        const issues = panel(page, "issues");
        const issuesB = panel(pageB, "issues");
        await expect(autonomous(pageB)).toHaveAttribute("aria-checked", "false");
        await expect(issuesB.getByLabel("Max active", { exact: true })).toHaveValue("2");
        await expect(issuesB.getByLabel("Allowed labels", { exact: true })).toHaveValue("project-b");

        // AC-1/2: opening an existing Settings window selects, focuses and
        // reveals Issues. The Monitor gear uses the same route.
        const originalWindow = await settings(page).getAttribute("data-id");
        expect(originalWindow).toBeTruthy();
        await openSettings(page, "agents");
        await panel(page, "agents").getByRole("button", { name: "Add candidate", exact: true }).scrollIntoViewIfNeeded();
        await page.locator('[data-surface="issues"]').click();
        await expect(page.locator('.workspace-window[data-preset="issue"], .workspace-window[data-preset="issue_monitor"]')).toHaveCount(0);
        await expect(settings(page)).toHaveClass(/focused/);
        await openSettings(page, "issues");
        await expect(settings(page)).toHaveCount(1);
        expect(await settings(page).getAttribute("data-id")).toBe(originalWindow);
        await expect(settings(page)).toHaveClass(/focused/);
        await expect(issues).toBeFocused();
        await expect(autonomous(page)).toBeInViewport();
        await page.locator('[data-surface="issues"]').click();
        const gear = page.locator('[data-action="monitor-settings"]');
        await expect(gear).toBeVisible();
        const options = issues.locator(".settings-issues-options");
        if (await options.evaluate(element => (element as HTMLDetailsElement).open)) await gear.click();
        await gear.click();
        await expect(settings(page).locator('[data-settings-tab="issues"]')).toHaveAttribute("aria-selected", "true");
        await expect(options).toHaveAttribute("open", "");
        await expect(gear).toHaveAttribute("aria-expanded", "true");
        await expect(gear).toBeFocused();
        await expect(settings(page)).toHaveCount(1);

        // AC-1/5: a no-remote project with Autonomous and PM disabled can
        // persist Start/Stop without dispatching implementation work.
        await expect(issues.locator('[data-action="monitor-toggle"]')).toHaveText("Start");
        await expect(issues.locator('[data-action="monitor-toggle"]')).toBeEnabled();
        await toggleMonitor(page, true);
        await expect.poll(async () => (await readPreferences(a)).enabled).toBe(true);
        await expect(issues.locator('[data-action="monitor-toggle"]')).toHaveText("Stop");
        await toggleMonitor(page, false);
        await expect.poll(async () => (await readPreferences(a)).enabled).toBe(false);
        await expect(issues.locator('[data-action="monitor-toggle"]')).toHaveText("Start");

        // Edit the remaining project-local controls while Monitor is stopped.
        await autonomous(page).click();
        await expect.poll(async () => (await readPreferences(a)).autonomous_mode).toBe(true);
        const maxMode = issues.getByLabel("Max active mode", { exact: true });
        await selectCapacityMode(page, "manual");
        await editNumber(issues.getByLabel("Max active", { exact: true }), "6");
        await expect.poll(async () => (await readPreferences(a)).max_active_agents).toBe(6);
        await expect(maxMode).toHaveValue("manual");
        await selectCapacityMode(page, "auto");
        await expect.poll(async () => (await readPreferences(a)).max_active_agents_mode).toBe("auto");
        await expect(issues.getByLabel("Max active", { exact: true })).toBeDisabled();
        await selectCapacityMode(page, "manual");
        await expect(issues.getByLabel("Max active", { exact: true })).toBeEnabled();
        await editNumber(issues.getByLabel("Max active", { exact: true }), "4");
        await expect.poll(async () => (await readPreferences(a)).max_active_agents).toBe(4);
        await issues.getByLabel("Auto-refill", { exact: true }).check();
        await expect.poll(async () => (await readPreferences(a)).terminal_queue_auto_refill).toBe(true);
        await editNumber(issues.getByLabel("Refill limit", { exact: true }), "5");
        await expect.poll(async () => (await readPreferences(a)).terminal_queue_auto_refill_limit).toBe(5);
        await issues.getByLabel("Allowed labels", { exact: true }).fill("auto-merge, bug");
        await issues.getByLabel("Allowed labels", { exact: true }).press("Tab");
        await expect.poll(async () => (await readPreferences(a)).allowed_labels).toEqual(["auto-merge", "bug"]);
        await expect(autonomous(pageB)).toHaveAttribute("aria-checked", "false");
        await expect(issuesB.getByLabel("Max active", { exact: true })).toHaveValue("2");
        await expect(issuesB.getByLabel("Allowed labels", { exact: true })).toHaveValue("project-b");

        // AC-4/5: read full stored profiles, edit, reorder, remove and add.
        await openSettings(page, "agents");
        await openSettings(pageB, "agents");
        const agents = panel(page, "agents");
        const agentsB = panel(pageB, "agents");
        await sendLiveGwtEvent(page, { kind: "get_issue_monitor_profiles" });
        await page.waitForFunction(root => ((window as any).__gwtPlaywrightMessages ?? []).some(({ payload }: any) =>
          payload.kind === "issue_monitor_profiles" && payload.project_root === root
          && payload.profiles[0]?.version === "1.2.3"), a.root);
        await expect(agents.getByLabel("Model codex", { exact: true })).toHaveValue("gpt-6-astra");
        await expect(agents.getByLabel("Reasoning codex", { exact: true })).toHaveValue("high");
        await expect(agents.getByLabel("Prefer for codex", { exact: true })).toHaveValue("type:fix");
        await agents.getByLabel("Model codex", { exact: true }).fill("gpt-6-sol");
        await agents.getByLabel("Reasoning codex", { exact: true }).fill("medium");
        await agents.getByLabel("Prefer for codex", { exact: true }).fill("bug");
        await agents.getByRole("button", { name: "Save profiles", exact: true }).click();
        await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? []).some(({ payload }: any) =>
          payload.kind === "issue_monitor_toast" && payload.level === "error" && /prefer_for/.test(payload.message)));
        await sendLiveGwtEvent(page, { kind: "get_issue_monitor_profiles" });
        await expect(agents.getByLabel("Model codex", { exact: true })).toHaveValue("gpt-6-sol");
        await expect(agents.getByLabel("Prefer for codex", { exact: true })).toHaveValue("bug");
        expect((await readPreferences(a)).launch_profiles[0].model).toBe("gpt-6-astra");
        await agents.getByLabel("Prefer for codex", { exact: true }).fill("type:fix, kind:spec");
        await agents.getByLabel("Session mode codex", { exact: true }).selectOption("Continue");
        await agents.getByLabel("Runtime codex", { exact: true }).selectOption("Docker");
        await agents.getByLabel("Docker service codex", { exact: true }).fill("fixture-service");
        await agents.getByRole("button", { name: "Save profiles", exact: true }).click();
        await expect.poll(async () => (await readPreferences(a)).launch_profiles[0]).toMatchObject({
          agent_id: "codex", model: "gpt-6-sol", reasoning: "medium", prefer_for: ["type:fix", "kind:spec"],
          version: "1.2.3", runtime_target: "Docker", docker_service: "fixture-service", docker_lifecycle_intent: "Connect", session_mode: "Continue",
        });
        await agents.getByRole("button", { name: "Move down codex", exact: true }).click();
        await expect(agents.locator("fieldset[data-agent-id]").first()).toHaveAttribute("data-agent-id", "claude");
        await agents.getByRole("button", { name: "Move up codex", exact: true }).click();
        await expect(agents.locator("fieldset[data-agent-id]").first()).toHaveAttribute("data-agent-id", "codex");
        await agents.getByRole("button", { name: "Remove claude", exact: true }).click();
        await expect(agents.locator("fieldset[data-agent-id]")).toHaveCount(1);
        await expect(agents.getByRole("button", { name: "Remove codex", exact: true })).toBeDisabled();
        await agents.getByLabel("Add candidate candidate", { exact: true }).fill("claude");
        await agents.getByRole("button", { name: "Add candidate", exact: true }).click();
        await expect(agents.locator("fieldset[data-agent-id]")).toHaveCount(2);
        await agents.getByLabel("Model claude", { exact: true }).fill("sonnet");
        await agents.getByLabel("Reasoning claude", { exact: true }).fill("medium");
        await agents.getByRole("button", { name: "Save profiles", exact: true }).click();
        await expect.poll(async () => (await readPreferences(a)).launch_profiles[1].model).toBe("sonnet");
        await agents.getByRole("button", { name: "Move down codex", exact: true }).click();
        await expect.poll(async () => (await readPreferences(a)).launch_profiles.map((profile: any) => profile.agent_id)).toEqual(["claude", "codex"]);
        await expect(agentsB.getByLabel("Model codex", { exact: true })).toHaveValue("project-b-model");

        // PM ruling: the single global update toggle lives in System and
        // refreshes the other project without changing its project settings.
        await openSettings(page, "system");
        await openSettings(pageB, "system");
        const updates = panel(page, "system").getByLabel("Auto-apply updates", { exact: true });
        const updatesB = panel(pageB, "system").getByLabel("Auto-apply updates", { exact: true });
        await expect(settings(page).getByLabel("Auto-apply updates", { exact: true })).toHaveCount(1);
        await expect(issues.getByLabel("Auto-apply updates", { exact: true })).toHaveCount(0);
        await expect(updates).toBeEnabled();
        await expect(updates).not.toBeChecked();
        await updates.check();
        await expect(updatesB).toBeChecked();
        await expect(updates.locator("..")).toContainText("Auto-apply updates");
        await expect(panel(page, "system")).toContainText(/at least one open project.*Autonomous/);

        await expect(panel(page, "system")).toContainText("Saved: On · Effective: On");
        await expect(panel(page, "system")).toContainText("No update pending");
        // Update policy is GUI-only; the PM has a dedicated read-only view.
        const cliEnv = Object.fromEntries(Object.entries(process.env).filter(([key]) =>
          !/^(GWT_|CODEX_|CLAUDE_|GIT_)/i.test(key)
          && !["GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"].includes(key.toUpperCase())));
        const runCli = (operation: string, params: Record<string, unknown> = {}) => spawnSync(join(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT!, "target/debug/gwtd"), [], {
          cwd: a.root, env: { ...cliEnv, HOME: backend.home, USERPROFILE: backend.home },
          input: JSON.stringify({ schema_version: 1, operation, params }),
          encoding: "utf8", timeout: 30_000,
        });
        for (const enabled of [false, true]) {
          const rejected = runCli("issue.monitor.config.set", { auto_apply_updates: enabled });
          expect(rejected.status).not.toBe(0);
          expect(rejected.stderr + rejected.stdout).toMatch(/unknown parameter/i);
          expect(rejected.stderr + rejected.stdout).toContain("Settings > System");
        }
        const status = runCli("update.settings.status");
        expect(status.status, status.stderr).toBe(0);
        expect(JSON.parse(JSON.parse(status.stdout).output)).toMatchObject({
          auto_apply_updates: true, auto_apply_updates_effective: true,
        });
        expect(await readPreferences(a)).not.toHaveProperty("auto_apply_updates");
        await expect(updates).toBeChecked();
        const configPath = join(backend.home, ".gwt/config.toml");
        const savedConfig = await readFile(configPath, "utf8");
        try {
          await writeFile(configPath, "invalid = [\n");
          await updates.evaluate((input: HTMLInputElement) => input.click());
          await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
            .some(({ payload }: any) => payload.kind === "system_settings_error"));
          await expect(updates).toBeChecked();
        } finally { await writeFile(configPath, savedConfig); }
        await updates.uncheck();
        await expect(updates).not.toBeChecked();
        await expect(updatesB).not.toBeChecked();
        await expect(panel(page, "system")).toContainText("Saved: Off · Effective: Off");
        // A pending update targets a future version; applied-version holds
        // are deliberately reconciled away by the existing runtime policy.
        const drain = runCli("issue.monitor.config.set", {
          update_drain: { reason: "manual", version: "99.0.0", automatic: false },
        });
        expect(drain.status, drain.stderr).toBe(0);
        await sendLiveGwtEvent(page, { kind: "get_update_settings" });
        await expect(panel(page, "system")).toContainText(/Update .* pending/);
        expect(runCli("issue.monitor.config.set", { update_drain: false }).status).toBe(0);
        await sendLiveGwtEvent(page, { kind: "get_update_settings" });
        await expect(panel(page, "system")).toContainText("No update pending");
        await updates.check();
        await expect(updatesB).toBeChecked();

        for (const current of [page, pageB]) {
          await current.reload();
          // Restore the real saved workspace before dispatching another open;
          // an early open would race hydration and create a second window.
          await current.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
            .some(({ payload }: any) => payload.kind === "workspace_state"));
          await expect(settings(current)).toHaveCount(1);
          await openSettings(current, "system");
          await expect(panel(current, "system").getByLabel("Auto-apply updates", { exact: true })).toBeChecked();
          await expect(current.locator("html")).toHaveAttribute("data-theme", themeFor(info));
        }
        await openSettings(page, "issues");
        await expect(autonomous(page)).toHaveAttribute("aria-checked", "true");
        await expect(issues.getByLabel("Max active", { exact: true })).toHaveValue("4");
        await expect(issues.getByLabel("Auto-refill", { exact: true })).toBeChecked();
        await expect(issues.getByLabel("Refill limit", { exact: true })).toHaveValue("5");
        await expect(issues.getByLabel("Allowed labels", { exact: true })).toHaveValue("auto-merge, bug");
        await expect(issues.locator('[data-action="monitor-toggle"]')).toHaveText("Start");
        await openSettings(page, "agents");
        await expect(agents.locator("fieldset[data-agent-id]").first()).toHaveAttribute("data-agent-id", "claude");
        await expect(agents.getByLabel("Model codex", { exact: true })).toHaveValue("gpt-6-sol");
        await expect(agents.getByLabel("Prefer for codex", { exact: true })).toHaveValue("type:fix, kind:spec");
        await expect(agents.getByLabel("Session mode codex", { exact: true })).toHaveValue("Continue");
        await expect(agents.getByLabel("Runtime codex", { exact: true })).toHaveValue("Docker");
        await expect(agents.getByLabel("Docker service codex", { exact: true })).toHaveValue("fixture-service");
        await openSettings(pageB, "issues");
        await expect(autonomous(pageB)).toHaveAttribute("aria-checked", "false");
        await expect(issuesB.getByLabel("Max active", { exact: true })).toHaveValue("2");
        await expect(issuesB.getByLabel("Allowed labels", { exact: true })).toHaveValue("project-b");
        const finalA = await readPreferences(a), finalB = await readPreferences(b);
        for (const key of ["enabled", "autonomous_mode", "max_active_agents", "max_active_agents_mode", "terminal_queue_auto_refill", "terminal_queue_auto_refill_limit", "allowed_labels", "launch_profiles"]) {
          expect(finalB[key], `Project B preserves ${key}`).toEqual(originalB[key]);
        }
        expect(finalA.enabled).toBe(false);
        expect(finalB.enabled).toBe(false);
        for (const section of ["issues", "agents", "system"]) await screenshot(page, section, info);
        expect(errors).toEqual([]);
      } finally {
        await info.attach(`settings-focus-${themeFor(info)}`, { body: JSON.stringify(await page.evaluate(() => ({
          activeElement: document.activeElement?.outerHTML.slice(0, 700), hasFocus: document.hasFocus(),
          openDialogs: [...document.querySelectorAll('[role="dialog"]')].filter(el => el.getClientRects().length).map(el => el.id),
        })), null, 2), contentType: "application/json" });
        await info.attach(`settings-errors-${themeFor(info)}`, { body: JSON.stringify(errors, null, 2), contentType: "application/json" });
        await info.attach(`settings-workspace-frames-${themeFor(info)}`, { body: JSON.stringify(workspaceFrames, null, 2), contentType: "application/json" });
        if (process.env.GWT_L3_SCREENSHOT_DIR) {
          await writeFile(join(process.env.GWT_L3_SCREENSHOT_DIR, `settings-errors-${themeFor(info)}.json`), JSON.stringify(errors, null, 2));
          await writeFile(join(process.env.GWT_L3_SCREENSHOT_DIR, `settings-workspace-frames-${themeFor(info)}.json`), JSON.stringify(workspaceFrames, null, 2));
        }
        await pageB.close();
      }
    });
  });
});
