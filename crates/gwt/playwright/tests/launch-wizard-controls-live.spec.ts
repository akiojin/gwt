/**
 * SPEC-2014 2026-05-29 amendment (SC-065) — Launch Agent setting controls
 * live E2E. Runs against a real gwt browser-server backend and exercises the
 * operation-appropriate controls introduced by the UI/UX overhaul:
 *
 *   - Target renders as a segmented radiogroup and switching to Shell hides
 *     the agent-specific settings (deterministic: Target is always Agent/Shell).
 *   - Reasoning renders as a snapped slider with the Claude "Auto" lifted into
 *     a separate toggle; moving the slider updates the launch summary and
 *     enabling Auto suspends the slider and reports "auto".
 *
 * Like the Fast mode live spec, the whole suite is gated on
 * GWT_PLAYWRIGHT_BASE_URL and stops before any real launch.
 */
import { randomUUID } from "node:crypto";
import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  acquireLiveGwtBackendLock,
  clearLiveLaunchWizard,
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test.describe.serial("Launch Wizard setting controls (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");
  test.setTimeout(120_000);

  let releaseBackendLock: (() => Promise<void>) | undefined;
  let browserErrors: string[] = [];
  let pmEditorRequests: any[] = [];

  test.beforeEach(async ({ page }, testInfo) => {
    browserErrors = [];
    page.on("pageerror", error => browserErrors.push(error.message));
    page.on("console", message => { if (message.type() === "error") browserErrors.push(message.text()); });
    releaseBackendLock = await acquireLiveGwtBackendLock(BASE, testInfo);
    pmEditorRequests = [];
    page.on("websocket", socket => socket.on("framesent", ({ payload }) => {
      try {
        const request = JSON.parse(String(payload));
        if (["agent_roster", "set_pm_launch_profile", "open_pm_agent", "restart_pm_agent"].includes(request.kind)) pmEditorRequests.push(request);
      } catch { /* Unrelated frames are not retained. */ }
    }));
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await keepLaunchWizardModalVisible(page);
    await openLiveGwtProject(page);
    await clearLiveLaunchWizard(page);
  });

  test.afterEach(async ({ page }) => {
    if (!releaseBackendLock) return;
    try {
      await clearLiveLaunchWizard(page);
    } finally {
      await releaseBackendLock();
      releaseBackendLock = undefined;
    }
    expect(browserErrors, "console and page errors").toEqual([]);
  });

  test("Target is a segmented radiogroup that toggles agent settings", async ({
    page,
  }) => {
    await openLaunchWizardForBranch(page);
    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toBeVisible();
    await enterLaunchSettings(page);

    const target = wizard.getByRole("radiogroup", { name: "Target" });
    await expect(target).toBeVisible();
    const shell = target.locator('.launch-segmented__option[data-value="shell"]');
    const agent = target.locator('.launch-segmented__option[data-value="agent"]');
    await expect(agent).toHaveAttribute("aria-checked", "true");

    await selectLaunchTarget(page, shell, "Shell");
    // Agent-only controls disappear when Shell is the launch target.
    await expect(wizard.getByRole("radiogroup", { name: "Agent" })).toHaveCount(0, {
      timeout: 15_000,
    });

    await selectLaunchTarget(page, agent, "Agent");
  });

  test("Grok Build exposes free-text Model and common Effort controls", async ({
    page,
  }) => {
    const pageErrors: string[] = [];
    const consoleErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });

    await openLaunchWizardForBranch(page);
    const wizard = page.locator("#wizard-modal");
    await enterLaunchSettings(page);

    const agentField = wizard.getByLabel("Agent", { exact: true });
    const tag = await agentField.evaluate((node) => node.tagName.toLowerCase());
    // The real backend lists detected agents only (SPEC-1921) and retires
    // Gemini (Issue #4660), regardless of the count-adaptive control layout.
    const agentOptions = agentField.locator(
      tag === "select" ? "option" : ".launch-segmented__option",
    );
    const agentIds = await agentOptions.evaluateAll((options) =>
      options.map((option) =>
        option.getAttribute("value") ?? option.getAttribute("data-value"),
      ),
    );
    expect(agentIds).not.toContain("gemini");
    expect(agentIds).toEqual(expect.arrayContaining(["claude", "codex", "grok"]));
    if (tag === "select") {
      await expect(agentField.locator('option[value="grok"]')).toHaveText(
        "Grok Build",
      );
    } else {
      await expect(
        wizard.locator('.launch-segmented__option[data-value="grok"]'),
      ).toContainText("Grok Build");
    }

    await selectWizardAgent(page, "grok");
    await expect(agentSummaryValue(page)).toHaveText("Grok Build");

    const model = wizard.getByLabel("Model", { exact: true });
    await expect(model).toHaveAttribute("type", "text");
    await expect(model).toHaveAttribute(
      "placeholder",
      "Grok model id (blank = config)",
    );
    await model.fill("DefaultXL");
    await model.blur();
    await expect(summaryValue(page, "Model")).toHaveText("DefaultXL");

    const effort = wizard.getByLabel("Effort", { exact: true });
    const effortValues = await wizard
      .locator(".launch-range__tick")
      .evaluateAll((ticks) => ticks.map((tick) => tick.getAttribute("data-value")));
    expect(effortValues).toEqual([
      "none",
      "minimal",
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
    ]);

    const auto = wizard.locator('[data-reasoning-auto] input[type="checkbox"]');
    await expect(auto).toBeChecked();
    await expect(effort).toBeDisabled();
    await expect(summaryValue(page, "Effort")).toHaveCount(0);

    await auto.setChecked(false);
    await auto.blur();
    await expect(effort).toBeEnabled();
    await expect(summaryValue(page, "Effort")).toHaveText("medium");
    await effort.press("ArrowRight");
    await effort.blur();
    await expect(summaryValue(page, "Effort")).toHaveText("high");

    await auto.setChecked(true);
    await auto.blur();
    await expect(effort).toBeDisabled();
    await expect(summaryValue(page, "Effort")).toHaveCount(0);

    await model.fill("");
    await model.blur();
    await expect(summaryValue(page, "Model")).toHaveCount(0);
    await page.evaluate(() => new Promise(requestAnimationFrame));
    expect(pageErrors).toEqual([]);
    expect(consoleErrors).toEqual([]);
  });

  test("PM settings persist a Grok launch profile and render pending restart state", async ({ page }, testInfo) => {
    const pageErrors: string[] = [], consoleErrors: string[] = [];
    page.on("pageerror", error => pageErrors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") consoleErrors.push(message.text()); });
    const currentPmStatus = async (target: Page) => target.evaluate(() => ((window as any).__gwtPlaywrightMessages || [])
      .filter(({ payload }: any) => payload.kind === "pm_status").at(-1)?.payload);
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    const surface = page.locator(".agents-surface:not(.agents-surface--preview)");
    const detail = surface.locator(".roster-detail");
    const projectKey = new URL(page.url()).pathname.slice(3);
    expect(projectKey).toMatch(/^[a-f0-9]+$/);
    await expect.poll(() => currentPmStatus(page)).toMatchObject({ project_key: projectKey, auto_start: false, is_running: false });
    const rosterMutation = async (kind: string, submit: () => Promise<void>) => {
      const start = pmEditorRequests.length;
      const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
      await submit();
      await expect.poll(() => pmEditorRequests.slice(start).find(request => request.kind === "agent_roster" && request.action?.kind === kind)?.request_id).toBeTruthy();
      const request = pmEditorRequests.slice(start).find(request => request.kind === "agent_roster" && request.action?.kind === kind);
      expect(request.project_key).toBe(projectKey);
      const reply = await page.waitForFunction(({ after, requestId }) => ((window as any).__gwtPlaywrightMessages || []).find((entry: any) =>
        entry.sequence > after && entry.payload.request_id === requestId && ["agent_roster", "agent_roster_error"].includes(entry.payload.kind))?.payload,
      { after, requestId: request.request_id }).then(handle => handle.jsonValue());
      expect(reply.kind, reply.message || reply.roster?.error).toBe("agent_roster");
      expect(reply.roster.project_key).toBe(projectKey); expect(reply.roster.error).toBeNull();
      return reply.roster;
    };
    const memberName = `PM editor Grok ${testInfo.project.name}-${randomUUID().slice(0, 8)}`;
    await surface.getByRole("textbox", { name: "Name for new member", exact: true }).fill(memberName);
    await surface.getByRole("combobox", { name: "Tool for new member", exact: true }).selectOption("claude");
    await surface.getByRole("checkbox", { name: "PM eligible for new member", exact: true }).check();
    const registered = await rosterMutation("add", () => surface.locator('.roster-capacity [data-action="add"]').click());
    const member = registered.members.find((value: any) => value.name === memberName);
    expect(member).toMatchObject({ agent_id: "claude", enabled: true, pm_eligible: true, assignment: null });
    const memberId = member.agent_member_id;
    expect(memberId).toMatch(/^[0-9a-f-]{36}$/i);
    await surface.locator(`.roster-row[data-member-id="${memberId}"]`).click();
    const selected = await rosterMutation("select_pm", () => detail.locator('[data-action="select-pm"]').click());
    expect(selected.members.find((value: any) => value.agent_member_id === memberId)?.pm_project_key).toBe(projectKey);
    await expect.poll(() => currentPmStatus(page)).toMatchObject({ project_key: projectKey, configured_member_id: memberId, is_running: false });
    let profileRevision = (await currentPmStatus(page)).profile_revision;
    expect(profileRevision).toBeTruthy();
    await expect(detail.locator('[data-member-tuning-status]')).toHaveText("Saved");
    const saveProfile = async (field: string, value: string) => {
      const start = pmEditorRequests.length;
      const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
      const input = detail.locator(`[data-member-tuning-field="${field}"]`);
      const wireField = field;
      const expected = value || null;
      const before = await currentPmStatus(page);
      expect(before).toMatchObject({ project_key: projectKey, configured_member_id: memberId, profile_revision: profileRevision, is_running: false });
      if ((before[`configured_${wireField}`] ?? null) === expected) {
        await expect(input).toHaveValue(value);
        await expect(detail.locator('[data-member-tuning-status]')).toHaveText("Saved");
        await expect(detail.locator('[data-member-tuning-revision]')).toHaveText(`Revision: ${profileRevision}`);
        expect(pmEditorRequests.slice(start).filter(request => request.kind === "set_pm_launch_profile"), "an unchanged saved field needs no write").toEqual([]);
        return;
      }
      await input.fill(value);
      await expect.poll(() => pmEditorRequests.slice(start).find(request => request.kind === "set_pm_launch_profile")?.request_id).toBeTruthy();
      const request = pmEditorRequests.slice(start).find(request => request.kind === "set_pm_launch_profile");
      expect(request).toMatchObject({ project_key: projectKey, member_id: memberId, expected_profile_revision: profileRevision });
      expect(request[wireField] ?? null).toBe(expected);
      const reply = await page.waitForFunction(({ after, requestId }) => ((window as any).__gwtPlaywrightMessages || []).find((entry: any) =>
        entry.sequence > after && entry.payload.kind === "pm_launch_profile_result" && entry.payload.request_id === requestId)?.payload,
      { after, requestId: request.request_id }).then(handle => handle.jsonValue());
      expect(reply, reply.message).toMatchObject({ success: true, project_key: projectKey, member_id: memberId });
      expect(reply.profile[wireField] ?? null).toBe(expected);
      expect(reply.profile_revision).toBeTruthy(); profileRevision = reply.profile_revision;
      await expect.poll(async () => {
        const status = await currentPmStatus(page);
        return { project: status?.project_key, member: status?.configured_member_id, revision: status?.profile_revision,
          value: status?.[`configured_${wireField}`] ?? null, running: status?.is_running };
      }).toEqual({ project: projectKey, member: memberId, revision: profileRevision, value: expected, running: false });
      await expect(detail.locator('[data-member-tuning-status]')).toHaveText("Saved");
      await expect(detail.locator('[data-member-tuning-revision]')).toHaveText(`Revision: ${profileRevision}`);
    };
    const agent = detail.locator('[data-member-tuning-field="agent_id"]');
    const model = detail.locator('[data-member-tuning-field="model"]');
    const effort = detail.locator('[data-member-tuning-field="reasoning"]');
    await saveProfile("agent_id", "claude");
    const claudeStatus = await currentPmStatus(page);
    const claudeCatalog = await page.evaluate(project => ((window as any).__gwtPlaywrightMessages || [])
      .filter(({ payload }: any) => payload.kind === "agent_roster" && payload.roster.project_key === project).at(-1)?.payload.roster.tuning_choices
      ?.find((choice: any) => choice.id === "claude"), projectKey);
    expect(claudeCatalog, "the shared roster supplies Claude's current catalog").toBeTruthy();
    const claudeModel = claudeCatalog.models.find((choice: any) => choice.value === claudeStatus.configured_model);
    const claudeEfforts = (claudeModel?.reasoning_options || claudeCatalog.reasoning_options).map((choice: any) => choice.value);
    expect(claudeEfforts).toEqual(expect.arrayContaining(["auto", "low", "medium", "high"]));
    for (const foreign of ["none", "minimal", "ultra"]) expect(claudeEfforts).not.toContain(foreign);
    await expect.poll(() => effort.locator("..").locator("datalist option").evaluateAll(options => options.map(option => (option as HTMLOptionElement).value)))
      .toEqual(claudeEfforts);
    await saveProfile("agent_id", "grok");
    await expect(agent).toHaveValue("grok");
    expect((await currentPmStatus(page)).agent_options).toContainEqual({ id: "grok", name: "Grok Build" });
    await expect(agent.locator("..").locator('datalist option[value="grok"]')).toHaveCount(1);
    await expect.poll(() => effort.locator("..").locator("datalist option").evaluateAll(options => options.map(option => (option as HTMLOptionElement).value)))
      .toEqual(["auto", "none", "minimal", "low", "medium", "high", "xhigh", "max"]);
    await saveProfile("model", "team/grok-code-fast");
    await saveProfile("reasoning", "high");
    await expect(model).toHaveValue("team/grok-code-fast");
    await expect(effort).toHaveValue("high");
    await page.reload();
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    // Navigation needs the current project's hydrated PM identity.
    await expect.poll(() => currentPmStatus(page)).toMatchObject({ project_key: projectKey, configured_member_id: memberId,
      profile_revision: profileRevision, configured_agent_id: "grok", configured_model: "team/grok-code-fast", configured_reasoning: "high", is_running: false });
    await page.locator(".pm-launcher-shell").hover();
    await page.getByRole("button", { name: "Project Manager settings" }).click();
    await expect(surface).toBeVisible();
    await expect(surface.locator(`.roster-row[data-member-id="${memberId}"]`)).toHaveAttribute("aria-selected", "true");
    await expect(agent).toHaveValue("grok");
    await expect(model).toHaveValue("team/grok-code-fast");
    await expect(effort).toHaveValue("high");
    await expect(detail.locator('[data-member-tuning-status]')).toHaveText("Saved");
    await expect(detail.locator('[data-member-tuning-revision]')).toHaveText(`Revision: ${profileRevision}`);
    const saved = await currentPmStatus(page);
    // Only running facts are synthetic: the saved identity/profile stays the
    // real backend readback, and dismissing Restart performs no paid launch.
    // Isolate this presentation fixture from real stopped PM snapshots. Drain
    // already-queued messages first; the final reload restores live delivery.
    const liveSockets = await page.evaluate(project => {
      const sockets: WebSocket[] = ((window as any).__gwtPlaywrightSockets || [])
        .filter((socket: WebSocket) => socket.readyState === WebSocket.OPEN);
      for (const socket of sockets) socket.addEventListener("message", event => {
        let payload;
        try { payload = JSON.parse(String(event.data)); } catch { return; }
        if (payload.kind === "pm_status" && payload.project_key === project) event.stopImmediatePropagation();
      }, { capture: true });
      return sockets.length;
    }, projectKey);
    expect(liveSockets).toBeGreaterThan(0);
    await expect.poll(() => page.evaluate(() => (window as any).__gwtTerminalTestApi.metrics("pm-presentation-probe").outputPending)).toBe(false);
    await page.evaluate(status => window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
      ...status, running_agent_id: "claude", running_model: "sonnet", running_reasoning: "medium", is_running: true,
    } })), saved);
    const operations = surface.locator('[data-role="pm-project-operations"]');
    await expect(operations.locator('[data-role="pm-running-as"]')).toContainText("Running as: Claude Code · Model: sonnet · Effort: medium");
    await expect(operations.locator('[data-role="pm-pending-chip"]')).toBeVisible();
    let restartPrompt = "";
    page.once("dialog", async dialog => { restartPrompt = dialog.message(); await dialog.dismiss(); });
    await operations.locator('[data-role="pm-restart"]').click();
    await expect.poll(() => restartPrompt).toContain("new conversation");
    await page.reload();
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await expect.poll(() => currentPmStatus(page)).toMatchObject({ configured_member_id: memberId, configured_agent_id: "grok",
      configured_model: "team/grok-code-fast", configured_reasoning: "high", is_running: false });
    expect(pmEditorRequests.filter(request => ["open_pm_agent", "restart_pm_agent"].includes(request.kind))).toEqual([]);
    expect(pageErrors).toEqual([]); expect(consoleErrors).toEqual([]);
  });

  test("Claude model rows preserve versionless labels and alias selections", async ({
    page,
  }) => {
    const pageErrors: string[] = [];
    const consoleErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });

    await openLaunchWizardForBranch(page);
    const wizard = page.locator("#wizard-modal");
    await enterLaunchSettings(page);
    await selectWizardAgent(page, "claude");
    await expect(agentSummaryValue(page)).toHaveText("Claude Code");

    const model = wizard.getByLabel("Model", { exact: true });
    await expect(model.locator("option")).toHaveText([
      "Default", "Opus", "Fable", "Sonnet", "Haiku",
    ]);
    expect(await model.locator("option").evaluateAll((options) =>
      options.map((option) => (option as HTMLOptionElement).value),
    )).toEqual(["", "opus", "fable", "sonnet", "haiku"]);

    const range = wizard.locator(".launch-range__input");
    const auto = wizard.locator('[data-reasoning-auto] input[type="checkbox"]');
    let opusEffortValues: (string | null)[] = [];
    for (const alias of ["opus", "fable", "sonnet", "haiku"]) {
      await model.selectOption(alias);
      await model.blur();
      // Wait for backend state after the interaction guard releases on blur.
      await expect(summaryValue(page, "Model")).toHaveText(alias);
      if (alias === "opus" || alias === "fable") {
        await expect(range).toBeVisible();
        const effortValues = await wizard.locator(".launch-range__tick")
          .evaluateAll((ticks) => ticks.map((tick) => tick.getAttribute("data-value")));
        if (alias === "opus") {
          opusEffortValues = effortValues;
          expect(effortValues).toEqual(expect.arrayContaining(["xhigh", "max"]));
        } else {
          expect(effortValues).toEqual(opusEffortValues);
        }
      }
    }
    await expect(range).toHaveCount(0);
    await expect(auto).toHaveCount(0);

    await model.selectOption("");
    await model.blur();
    await expect(summaryValue(page, "Model")).toHaveCount(0);
    await expect(model).toHaveValue("");
    await expect(range).toBeVisible();
    await expect(auto).toHaveCount(1);
    await page.evaluate(() => new Promise(requestAnimationFrame));
    expect(pageErrors).toEqual([]);
    expect(consoleErrors).toEqual([]);
  });

  test("Reasoning renders as a slider with a separate Auto toggle", async ({
    page,
  }) => {
    await openLaunchWizardForBranch(page);
    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toBeVisible();
    await enterLaunchSettings(page);

    await selectWizardAgent(page, "claude");
    // Pin an effort-capable model so the reasoning control is shown
    // deterministically (Sonnet exposes Auto / Low / Medium / High).
    const model = wizard.getByLabel("Model", { exact: true });
    if ((await model.evaluate((n) => n.tagName.toLowerCase())) === "select") {
      await model.selectOption("haiku");
      await model.blur();
      await expect(summaryValue(page, "Model")).toHaveText("haiku");
      await model.selectOption("sonnet");
      await model.blur();
      // selectOption() only waits for the DOM change event. Wait for the real
      // backend round-trip (released on blur by the interaction guard) before
      // touching reasoning so a stale slider cannot commit its parked value.
      await expect(summaryValue(page, "Model")).toHaveText("sonnet");
    }

    // Auto is the default: the slider starts suspended and the effort is
    // delegated to Claude Code's own per-model default.
    const range = wizard.locator(".launch-range__input");
    await expect(range).toBeVisible();
    await expect(range).toBeDisabled();
    await expect(effortSummaryValue(page)).toHaveText("auto");

    const auto = wizard.locator('[data-reasoning-auto] input[type="checkbox"]');
    await expect(auto).toHaveCount(1);
    await expect(auto).toBeChecked();

    // Turning Auto off re-enables the slider parked at the middle ordinal
    // stop (Medium for Sonnet's Low / Medium / High scale).
    await auto.setChecked(false);
    await blurActiveElement(page);
    await expect(range).toBeEnabled();

    // Last-used effort is intentionally persistent. Drive the control to the
    // middle ordinal explicitly so retries and prior live sessions cannot
    // change the assertion's starting point.
    await range.press("Home");
    await range.press("ArrowRight");
    await range.blur();
    await expect(effortSummaryValue(page)).toHaveText("medium");

    // ArrowRight snaps from Medium to High and reports the stored value.
    // While the slider keeps focus, the wizardInteractionGuard (SPEC-2014
    // 2026-05-29) defers backend re-renders so the drag/keyboard interaction
    // is not destroyed mid-step; the coalesced state flushes on focusout.
    // Blur the slider to release the guard before asserting the summary.
    await range.press("ArrowRight");
    await range.blur();
    await expect(effortSummaryValue(page)).toHaveText("high");

    // Auto is a separate toggle, not a slider stop: re-enabling it suspends
    // the slider and reports "auto" again.
    await auto.setChecked(true);
    await blurActiveElement(page);
    await expect(range).toBeDisabled();
    await expect(effortSummaryValue(page)).toHaveText("auto");
  });
});

async function selectLaunchTarget(
  page: Page,
  option: Locator,
  expectedSummary: string,
): Promise<void> {
  await expect(async () => {
    await option.click();
    await expect(option).toHaveAttribute("aria-checked", "true", {
      timeout: 2_000,
    });
    await blurActiveElement(page);
    await expect(targetSummaryValue(page)).toHaveText(expectedSummary, {
      timeout: 5_000,
    });
  }).toPass({ timeout: 30_000 });
}

async function selectWizardAgent(page: Page, agentId: string): Promise<void> {
  const wizard = page.locator("#wizard-modal");
  const agentField = wizard.getByLabel("Agent", { exact: true });
  await expect(agentField).toBeVisible();
  const tag = await agentField.evaluate((node) => node.tagName.toLowerCase());
  if (tag === "select") {
    await agentField.selectOption(agentId);
    await expect(agentField).toHaveValue(agentId);
    await agentField.blur();
    return;
  }
  const option = wizard.locator(`.launch-segmented__option[data-value="${agentId}"]`);
  await option.click();
  await expect(option).toHaveAttribute("aria-checked", "true");
  await blurActiveElement(page);
}

async function openLaunchWizardForBranch(page: Page): Promise<void> {
  await sendLiveGwtEvent(page, {
    kind: "open_active_work_launch_wizard",
    branch_name: process.env.GWT_PLAYWRIGHT_BRANCH_NAME ?? "develop",
  });
}

async function enterLaunchSettings(page: Page): Promise<void> {
  const wizard = page.locator("#wizard-modal");
  const target = wizard.getByRole("radiogroup", { name: "Target" });
  if (await target.isVisible().catch(() => false)) {
    return;
  }
  await wizard.getByRole("button", { name: /^Configure and start/ }).click();
  await expect(target).toBeVisible({ timeout: 10_000 });
}

async function blurActiveElement(page: Page): Promise<void> {
  await page.evaluate(() => {
    const active = document.activeElement;
    if (active instanceof HTMLElement) active.blur();
  });
}

function targetSummaryValue(page: Page) {
  return page
    .locator(".wizard-summary-item", { hasText: "Target" })
    .locator(".wizard-summary-value");
}

function agentSummaryValue(page: Page) {
  return page
    .locator(".wizard-summary-item")
    .filter({
      has: page.locator(".wizard-summary-label", { hasText: /^Agent$/ }),
    })
    .locator(".wizard-summary-value");
}

function effortSummaryValue(page: Page) {
  return summaryValue(page, "Effort");
}

function summaryValue(page: Page, label: string) {
  return page
    .locator(".wizard-summary-item")
    .filter({
      has: page.locator(".wizard-summary-label", {
        hasText: new RegExp(`^${label}$`),
      }),
    })
    .locator(".wizard-summary-value");
}

async function keepLaunchWizardModalVisible(page: Page): Promise<void> {
  await page.addStyleTag({
    content: `
      #wizard-modal[aria-hidden="false"],
      #wizard-modal.open {
        display: flex !important;
        pointer-events: auto !important;
      }
      #wizard-modal[aria-hidden="true"] {
        display: none !important;
        pointer-events: none !important;
      }
    `,
  });
}
