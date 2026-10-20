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

test.afterEach(async ({ page }) => {
  if (!process.env.GWT_PLAYWRIGHT_BASE_URL || page.isClosed()) return;
  // Release the fixed surface before closing its disposable backing window.
  await page.locator('button[data-surface="agents"]').click();
  const ids = await settings(page).evaluateAll(windows => windows.map(window => (window as HTMLElement).dataset.id));
  const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
  for (const id of ids) await sendLiveGwtEvent(page, { kind: "close_window", id });
  await sendLiveGwtEvent(page, { kind: "list_windows" });
  // Wait for the backend's close/readback sequence before checking its DOM projection.
  await page.waitForFunction(({ cursor, ids }) => ((window as any).__gwtPlaywrightMessages ?? [])
    .some(({ sequence, payload }: any) => sequence > cursor && payload.kind === "window_list"
      && Array.isArray(payload.windows) && !payload.windows.some((window: any) => ids.includes(window.id))), { cursor, ids });
  await expect(settings(page)).toHaveCount(0);
});

test("Issue #5787 candidate choices preserve saved values and persist dependent selections", async ({ page }, info) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  test.skip(!base, "requires browser-check's isolated checkout");
  test.setTimeout(180_000);
  await withLiveGwtBackendLock(base!, info, async () => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await gotoLiveGwt(page, base!, { enableTestBridge: true });
    await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .some(({ payload }: any) => payload.kind === "workspace_state"));
    await sendLiveGwtEvent(page, { kind: "issue_monitor_profiles_set", request_id: 578701,
      profiles: [{ agent_id: "codex", model: "saved-unlisted-model", reasoning: "high", docker_service: "saved-service", runtime_target: "Host" }] });
    const seeded = await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .find(({ payload }: any) => ["issue_monitor_profiles", "issue_monitor_profiles_write_failed"].includes(payload.kind)
        && payload.request_id === 578701)?.payload).then(handle => handle.jsonValue());
    expect(seeded.kind, seeded.message).toBe("issue_monitor_profiles");
    const open = async () => {
      // Restore the saved window before the disclosure can create another one.
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
      await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "agents" } })));
      await expect(panel(page, "agents")).toBeVisible();
    };
    await open();
    const candidates = panel(page, "agents");
    const control = (name: string) => candidates.getByLabel(`${name} codex`, { exact: true });
    for (const name of ["Agent", "Model", "Reasoning", "Docker service"]) await expect(control(name)).toHaveJSProperty("tagName", "SELECT");
    await expect(control("Model")).toHaveValue("saved-unlisted-model");
    await expect(control("Reasoning")).toHaveValue("high");
    await expect(control("Docker service")).toHaveValue("saved-service");
    await expect(control("Docker service")).toBeDisabled();
    const save = async (action: () => Promise<unknown>) => {
      const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
      await action();
      const result = await page.waitForFunction(cursor => ((window as any).__gwtPlaywrightMessages ?? [])
        .find(({ sequence, payload }: any) => sequence > cursor
          && ["issue_monitor_profiles", "issue_monitor_profiles_write_failed"].includes(payload.kind)
          && Number.isInteger(payload.request_id))?.payload, cursor).then(handle => handle.jsonValue());
      expect(result.kind, result.message).toBe("issue_monitor_profiles");
      await expect(candidates).toHaveAttribute("data-saving", "false");
    };
    const select = (field: Locator, value: string) => save(async () => {
      await field.focus();
      await field.selectOption(value);
    });
    await select(control("Model"), "gpt-6-astra");
    await expect(control("Reasoning")).toHaveValue("medium");
    await select(control("Reasoning"), "high");
    await select(control("Runtime"), "Docker");
    await expect(control("Docker service")).toBeEnabled();
    // The checkout has a Compose service; use the detected option rather than
    // injecting a choice or contacting Docker.
    const service = await control("Docker service").locator("option").evaluateAll(options =>
      options.map(option => (option as HTMLOptionElement).value).find(value => value && value !== "saved-service"));
    expect(service).toBeTruthy();
    await select(control("Docker service"), service!);
    await gotoLiveGwt(page, base!, { enableTestBridge: true });
    await open();
    await expect(control("Model")).toHaveValue("gpt-6-astra");
    await expect(control("Reasoning")).toHaveValue("high");
    await expect(control("Docker service")).toHaveValue(service!);
    await expect(control("Windows shell")).toBeDisabled();
    await info.attach(`candidate-selects-${themeFor(info)}`, { body: await candidates.screenshot(), contentType: "image/png" });
    await select(control("Agent"), "openclaw");
    await candidates.getByRole("heading", { name: "Candidates", exact: true }).click();
    await expect(candidates.getByLabel("Reasoning openclaw", { exact: true })).toBeDisabled();
    await expect(candidates.getByLabel("Fast mode openclaw", { exact: true })).toBeDisabled();
    await select(candidates.getByLabel("Agent openclaw", { exact: true }), "hermes");
    await candidates.getByRole("heading", { name: "Candidates", exact: true }).click();
    const hermesModel = candidates.getByLabel("Model hermes", { exact: true });
    await expect(hermesModel).toHaveJSProperty("tagName", "INPUT");
    // A catalog reply may rerender the row between separate locator calls.
    const suggestions = await hermesModel.evaluate((input: HTMLInputElement) => ({
      id: input.getAttribute("list"), values: Array.from(input.list?.options ?? [], option => option.value),
    }));
    expect(suggestions.id).toBeTruthy();
    expect(suggestions.values).toEqual(["fixture-model", "fixture-alternative"]);
    await save(async () => { await hermesModel.fill("custom-hermes-model"); await hermesModel.press("Tab"); });
    await gotoLiveGwt(page, base!, { enableTestBridge: true });
    await open();
    await expect(candidates.getByLabel("Model hermes", { exact: true })).toHaveValue("custom-hermes-model");
    await expect(page.locator("html")).toHaveAttribute("data-theme", themeFor(info));
    await info.attach(`candidate-choices-${themeFor(info)}`, { body: await candidates.screenshot(), contentType: "image/png" });
    expect(errors).toEqual([]);
  });
});

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

async function setAutoApplyUpdates(page: Page, enabled: boolean): Promise<void> {
  const control = panel(page, "system").getByLabel("Auto-apply updates", { exact: true });
  const before = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
  // Status broadcasts may restore the old authoritative value between the
  // native click and the save acknowledgement. Wait for that new response.
  await control.click();
  await page.waitForFunction(({ before, enabled }) => ((window as any).__gwtPlaywrightMessages ?? [])
    .some(({ sequence, payload }: any) => sequence > before
      && payload.kind === "update_settings" && payload.status.auto_apply_updates === enabled),
  { before, enabled });
  if (enabled) await expect(control).toBeChecked();
  else await expect(control).not.toBeChecked();
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
  if (section === "issues") await panel(page, section).locator("[data-settings-issues-controls]").evaluate(element => element.scrollIntoView({ block: "start" }));
  const name = `settings-${section}-${themeFor(info)}`;
  await info.attach(name, { body: await settings(page).screenshot(), contentType: "image/png" });
  if (process.env.GWT_L3_SCREENSHOT_DIR) {
    await settings(page).screenshot({ path: join(process.env.GWT_L3_SCREENSHOT_DIR, `${name}.png`) });
  }
}

async function assertIssuesLayout(page: Page): Promise<void> {
  const controls = panel(page, "issues").locator("[data-settings-issues-controls]");
  await controls.scrollIntoViewIfNeeded();
  await expect(controls.locator("legend")).toHaveText(["Launch control", "Refill", "Target"]);
  await expect(controls.locator(":scope > :first-child")).toContainText("Project settings");
  const rows = controls.locator(".settings-field");
  await expect(rows).toHaveCount(6);
  const bounds = await rows.evaluateAll(elements => elements.map(element => {
    const row = element.getBoundingClientRect();
    const label = element.firstElementChild!.getBoundingClientRect();
    const value = element.lastElementChild!.getBoundingClientRect();
    return { top: row.top, bottom: row.bottom, label: label.width, value: value.left, right: row.right };
  }));
  const visible = await panel(page, "issues").locator(".settings-issues-options").boundingBox();
  expect(visible).not.toBeNull();
  expect(new Set(bounds.map(row => Math.round(row.label))).size).toBe(1);
  expect(new Set(bounds.map(row => Math.round(row.value))).size).toBe(1);
  for (const row of bounds) {
    expect(row.top).toBeGreaterThanOrEqual(0);
    expect(row.bottom).toBeLessThanOrEqual(page.viewportSize()!.height);
    expect(row.right).toBeLessThanOrEqual(page.viewportSize()!.width);
    expect(row.top).toBeGreaterThanOrEqual(visible!.y);
    expect(row.bottom).toBeLessThanOrEqual(visible!.y + visible!.height);
  }
  for (const name of ["Autonomous mode", "Max active", "Max active mode", "Auto-refill", "Refill limit", "Allowed labels"]) {
    await expect(controls.getByLabel(name, { exact: true })).toBeInViewport({ ratio: 1 });
  }
  await expect(controls.locator('[data-action="monitor-toggle"]')).toBeInViewport({ ratio: 1 });
}

test.describe("Issue #5322 Settings", () => {
  test.skip(Object.keys(BACKENDS).length === 0, "requires verify-issue-5322.mjs isolated backends");
  test.setTimeout(180_000);
  test.use({ viewport: { width: 1280, height: 900 }, actionTimeout: 15_000 });

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
          await sendLiveGwtEvent(current, { kind: "update_window_geometry",
            id: await settings(current).getAttribute("data-id"),
            geometry: { x: 20, y: 20, width: 1200, height: 820 }, cols: 0, rows: 0 });
          await expect(settings(current)).toHaveCSS("height", "820px");
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
        await page.locator('button[data-surface="issues"]').click();
        await expect(page.locator('.workspace-window[data-preset="issue"], .workspace-window[data-preset="issue_monitor"]')).toHaveCount(0);
        await expect(settings(page)).toHaveClass(/focused/);
        await openSettings(page, "issues");
        await expect(settings(page)).toHaveCount(1);
        expect(await settings(page).getAttribute("data-id")).toBe(originalWindow);
        await expect(settings(page)).toHaveClass(/focused/);
        await expect(issues).toBeFocused();
        await expect(autonomous(page)).toBeInViewport();
        await page.locator('button[data-surface="issues"]').click();
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
        await screenshot(page, "issues", info);
        await assertIssuesLayout(page);

        // AC-1/5: a no-remote project with Autonomous and PM disabled can
        // persist Start/Stop without dispatching implementation work.
        await expect(issues.locator('[data-action="monitor-toggle"]')).toHaveText("Start");
        await expect(issues.locator('[data-action="monitor-toggle"]')).toBeEnabled();
        await toggleMonitor(page, true);
        await expect(issues.locator('[data-action="monitor-toggle"]')).toBeFocused();
        await expect.poll(async () => (await readPreferences(a)).enabled).toBe(true);
        await expect(issues.locator('[data-action="monitor-toggle"]')).toHaveText("Stop");
        await toggleMonitor(page, false);
        await expect.poll(async () => (await readPreferences(a)).enabled).toBe(false);
        await expect(issues.locator('[data-action="monitor-toggle"]')).toHaveText("Start");

        // Edit the remaining project-local controls while Monitor is stopped.
        await autonomous(page).click();
        await expect.poll(async () => (await readPreferences(a)).autonomous_mode).toBe(true);
        await expect(autonomous(page)).toBeFocused();
        const maxMode = issues.getByLabel("Max active mode", { exact: true });
        await selectCapacityMode(page, "manual");
        await editNumber(issues.getByLabel("Max active", { exact: true }), "6");
        await issues.getByRole("button", { name: "Apply Max active", exact: true }).click();
        await expect.poll(async () => (await readPreferences(a)).max_active_agents).toBe(6);
        await expect(maxMode).toHaveValue("manual");
        await selectCapacityMode(page, "auto");
        await expect.poll(async () => (await readPreferences(a)).max_active_agents_mode).toBe("auto");
        await expect(issues.getByLabel("Max active", { exact: true })).toBeDisabled();
        await selectCapacityMode(page, "manual");
        await expect(issues.getByLabel("Max active", { exact: true })).toBeEnabled();
        await editNumber(issues.getByLabel("Max active", { exact: true }), "4");
        await issues.getByRole("button", { name: "Apply Max active", exact: true }).click();
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
        const saveCandidate = async (action: () => Promise<unknown>) => {
          const before = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
          await action();
          await page.waitForFunction(({ before, root }) => ((window as any).__gwtPlaywrightMessages ?? [])
            .some(({ sequence, payload }: any) => sequence > before
              && payload.kind === "issue_monitor_profiles" && payload.project_root === root
              && Number.isInteger(payload.request_id)), { before, root: a.root });
          await expect(agents).toHaveAttribute("data-saving", "false");
        };
        const editCandidate = async (name: string, value: string) => saveCandidate(async () => {
          const control = agents.getByLabel(name, { exact: true });
          if (await control.evaluate(node => node.tagName === "SELECT")) await control.selectOption(value);
          else await control.fill(value);
          await agents.getByLabel(name, { exact: true }).press("Tab");
        });
        await expect(agents.getByRole("button", { name: "Save profiles", exact: true })).toHaveCount(0);
        await sendLiveGwtEvent(page, { kind: "get_issue_monitor_profiles" });
        await page.waitForFunction(root => ((window as any).__gwtPlaywrightMessages ?? []).some(({ payload }: any) =>
          payload.kind === "issue_monitor_profiles" && payload.project_root === root
          && payload.profiles[0]?.version === "1.2.3"), a.root);
        await expect(agents.getByLabel("Model codex", { exact: true })).toHaveValue("gpt-6-astra");
        await expect(agents.getByLabel("Reasoning codex", { exact: true })).toHaveValue("high");
        await expect(agents.getByLabel("Prefer for codex", { exact: true })).toHaveValue("type:fix");
        await editCandidate("Model codex", "gpt-6-sol");
        await editCandidate("Reasoning codex", "medium");
        await agents.getByLabel("Prefer for codex", { exact: true }).fill("bug");
        await agents.getByLabel("Prefer for codex", { exact: true }).press("Tab");
        await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? []).some(({ payload }: any) =>
          payload.kind === "issue_monitor_toast" && payload.level === "error" && /prefer_for/.test(payload.message)));
        await sendLiveGwtEvent(page, { kind: "get_issue_monitor_profiles" });
        await expect(agents.getByLabel("Model codex", { exact: true })).toHaveValue("gpt-6-sol");
        await expect(agents.getByLabel("Prefer for codex", { exact: true })).toHaveValue("bug");
        expect((await readPreferences(a)).launch_profiles[0]).toMatchObject({ model: "gpt-6-sol", prefer_for: ["type:fix"] });
        await editCandidate("Prefer for codex", "type:fix, kind:spec");
        await saveCandidate(() => agents.getByLabel("Session mode codex", { exact: true }).selectOption("Continue"));
        await saveCandidate(() => agents.getByLabel("Runtime codex", { exact: true }).selectOption("Docker"));
        await editCandidate("Docker service codex", "fixture-service");
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
        await agents.getByLabel("Add candidate candidate", { exact: true }).selectOption("claude");
        await agents.getByRole("button", { name: "Add candidate", exact: true }).click();
        await expect(agents.locator("fieldset[data-agent-id]")).toHaveCount(2);
        await editCandidate("Model claude", "sonnet");
        await editCandidate("Reasoning claude", "medium");
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
        await setAutoApplyUpdates(page, true);
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
        await setAutoApplyUpdates(page, false);
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
        await setAutoApplyUpdates(page, true);
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
