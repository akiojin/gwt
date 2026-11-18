/** Issue #7030: explicit speed selection with real backend state, without starting paid inference. */
import { randomUUID } from "node:crypto";
import { expect, test, type Page } from "@playwright/test";
import {
  acquireLiveGwtBackendLock, clearLiveLaunchWizard, gotoLiveGwt,
  openLiveGwtProject, sendLiveGwtEvent,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test.describe.serial("Codex and Claude speed settings (live backend)", () => {
  test.skip(!BASE, "requires an isolated checkout backend");
  test.setTimeout(180_000);
  let release: (() => Promise<void>) | undefined;
  let errors: string[];
  let pmEditorRequests: any[] = [];

  test.beforeEach(async ({ page }, testInfo) => {
    errors = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    release = await acquireLiveGwtBackendLock(BASE, testInfo);
    pmEditorRequests = [];
    page.on("websocket", socket => socket.on("framesent", ({ payload }) => {
      try {
        const request = JSON.parse(String(payload));
        if (["agent_roster", "set_pm_launch_profile", "open_pm_agent", "restart_pm_agent"].includes(request.kind)) pmEditorRequests.push(request);
      } catch { /* Unrelated frames are not retained. */ }
    }));
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    const theme = testInfo.project.use.colorScheme === "light" ? "light" : "dark";
    await page.locator(`#op-theme-toggle [data-theme-value="${theme}"]`).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await openLiveGwtProject(page);
    await clearLiveLaunchWizard(page);
  });
  test.afterEach(async ({ page }, testInfo) => {
    if (testInfo.status !== testInfo.expectedStatus) {
      await testInfo.attach("speed-settings-failure", {
        body: JSON.stringify({ url: page.url(), wizard: await latestWizardView(page),
          body: await page.locator("body").innerText() }, null, 2),
        contentType: "application/json",
      });
      await testInfo.attach("speed-settings-failure-screen", { body: await page.screenshot(), contentType: "image/png" });
    }
    try { await clearLiveLaunchWizard(page); }
    finally { await release?.(); release = undefined; }
    expect(errors, "console and page errors").toEqual([]);
  });

  test("Launch Wizard exposes Codex speed and keeps Claude controls separate", async ({ page }) => {
    await sendLiveGwtEvent(page, { kind: "open_active_work_launch_wizard", branch_name: process.env.GWT_PLAYWRIGHT_BRANCH_NAME ?? "develop" });
    await page.locator("#wizard-modal").getByRole("button", { name: /^Configure and start/ }).click({ timeout: 30_000 });
    await expect(page.locator("#wizard-modal").getByLabel("Agent", { exact: true })).toBeVisible({ timeout: 30_000 });
    await selectWizardAgent(page, "codex");
    const modal = page.locator("#wizard-modal");
    const speed = modal.getByRole("radiogroup", { name: "Speed", exact: true });
    await expect(speed).toBeVisible();
    for (const value of ["off", "fast", "ultrafast"]) {
      await speed.locator(`[data-value="${value}"]`).click();
      await blurActiveElement(page);
      await expect.poll(() => latestWizardView(page)).toMatchObject({ codex_speed: value });
      await expect(speed.locator(`[data-value="${value}"]`)).toHaveAttribute("aria-checked", "true");
    }
    await expect(modal).toContainText("Pro $500");
    await expect(modal).toContainText("Codex warns and omits unsupported tiers");
    await expect(modal.getByLabel("Use Claude Code Fast mode", { exact: true })).toHaveCount(0);
    await selectWizardAgent(page, "claude");
    await expect(speed).toHaveCount(0);
    await expect(modal.getByText("Ultrafast", { exact: true })).toHaveCount(0);
    const fast = modal.getByLabel("Use Claude Code Fast mode", { exact: true });
    await expect(fast).toBeVisible();
    await fast.check();
    await expect.poll(() => latestWizardView(page)).toMatchObject({ fast_mode: true });
    await selectWizardAgent(page, "codex");
    await expect(speed).toBeVisible();
    await expect(modal.getByLabel("Skip permission prompts", { exact: true })).toHaveCount(0);
  });

  test("Agent Settings saves Codex Ultrafast with model and reasoning and restores after reload", async ({ page }) => {
    await sendLiveGwtEvent(page, { kind: "issue_monitor_profiles_set", request_id: 703001,
      profiles: [{ agent_id: "codex", model: "gpt-6-astra", reasoning: "high", codex_speed: "off" }] });
    const seeded = await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .find(({ payload }: any) => ["issue_monitor_profiles", "issue_monitor_profiles_write_failed"].includes(payload.kind)
        && payload.request_id === 703001)?.payload).then(handle => handle.jsonValue());
    expect(seeded.kind, seeded.message).toBe("issue_monitor_profiles");
    await sendLiveGwtEvent(page, { kind: "issue_monitor_configure_profile" });
    const modal = page.locator("#wizard-modal");
    await expect(modal).toHaveClass(/open/, { timeout: 30_000 });
    const speed = modal.getByRole("radiogroup", { name: "Speed", exact: true });
    await speed.locator('[data-value="ultrafast"]').click();
    await blurActiveElement(page);
    await expect.poll(() => latestWizardView(page)).toMatchObject({
      selected_model: "gpt-6-astra", selected_reasoning: "high", codex_speed: "ultrafast",
    });
    const submit = page.locator("#wizard-submit-button");
    await submit.click();
    await expect.poll(() => latestWizardView(page)).toMatchObject({ show_runtime_confirmation: true });
    await submit.click();
    await expect.poll(() => latestWizardView(page)).toMatchObject({ show_confirm: true });
    await expect(submit).toHaveText(/Save/);
    await submit.click();
    await expect(modal).not.toHaveClass(/open/, { timeout: 30_000 });
    await page.reload();
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await sendLiveGwtEvent(page, { kind: "get_issue_monitor_profiles", request_id: 703002 });
    const saved = await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .filter(({ payload }: any) => payload.kind === "issue_monitor_profiles").at(-1)?.payload.profiles)
      .then(handle => handle.jsonValue());
    expect(saved[0]).toMatchObject({ agent_id: "codex", model: "gpt-6-astra", reasoning: "high", codex_speed: "ultrafast" });
    await sendLiveGwtEvent(page, { kind: "issue_monitor_configure_profile" });
    await expect(modal).toHaveClass(/open/, { timeout: 30_000 });
    await expect(speed.locator('[data-value="ultrafast"]')).toHaveAttribute("aria-checked", "true");
    await clearLiveLaunchWizard(page);
    await sendLiveGwtEvent(page, { kind: "open_active_work_launch_wizard", branch_name: process.env.GWT_PLAYWRIGHT_BRANCH_NAME ?? "develop" });
    await page.locator("#wizard-modal").getByRole("button", { name: /^Configure and start/ }).click({ timeout: 30_000 });
    await expect(page.locator("#wizard-modal").getByLabel("Agent", { exact: true })).toBeVisible({ timeout: 30_000 });
    await selectWizardAgent(page, "codex");
    await expect.poll(() => latestWizardView(page)).toMatchObject({
      selected_agent_id: "codex", selected_model: "gpt-6-astra",
      selected_reasoning: "high", codex_speed: "ultrafast",
    });
    await expect(speed.locator('[data-value="ultrafast"]')).toHaveAttribute("aria-checked", "true");
  });

  test("PM settings save and restore explicit Codex speed without starting PM", async ({ page }, testInfo) => {
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    const surface = page.locator(".agents-surface:not(.agents-surface--preview)");
    const detail = surface.locator(".roster-detail");
    const projectKey = new URL(page.url()).pathname.slice(3);
    expect(projectKey).toMatch(/^[a-f0-9]+$/);
    await expect.poll(() => latestPmStatus(page)).toMatchObject({ project_key: projectKey, auto_start: false, is_running: false });
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
    const memberName = `PM editor speed ${testInfo.project.name}-${randomUUID().slice(0, 8)}`;
    await surface.getByRole("textbox", { name: "Name for new member", exact: true }).fill(memberName);
    await surface.getByRole("combobox", { name: "Tool for new member", exact: true }).selectOption("codex");
    await surface.getByRole("checkbox", { name: "PM eligible for new member", exact: true }).check();
    const registered = await rosterMutation("add", () => surface.locator('.roster-capacity [data-action="add"]').click());
    const member = registered.members.find((value: any) => value.name === memberName);
    expect(member).toMatchObject({ agent_id: "codex", enabled: true, pm_eligible: true, assignment: null });
    const memberId = member.agent_member_id;
    expect(memberId).toMatch(/^[0-9a-f-]{36}$/i);
    await surface.locator(`.roster-row[data-member-id="${memberId}"]`).click();
    const selected = await rosterMutation("select_pm", () => detail.locator('[data-action="select-pm"]').click());
    expect(selected.members.find((value: any) => value.agent_member_id === memberId)?.pm_project_key).toBe(projectKey);
    await expect.poll(() => latestPmStatus(page)).toMatchObject({ project_key: projectKey, configured_member_id: memberId, is_running: false });
    let profileRevision = (await latestPmStatus(page)).profile_revision;
    expect(profileRevision).toBeTruthy();
    await expect(detail.locator('[data-member-tuning-status]')).toHaveText("Saved");
    const saveProfile = async (field: string, value: string) => {
      const start = pmEditorRequests.length;
      const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
      const input = detail.locator(`[data-member-tuning-field="${field}"]`);
      const wireField = field === "speed" ? "codex_speed" : field;
      const expected = value || null;
      const before = await latestPmStatus(page);
      expect(before).toMatchObject({ project_key: projectKey, configured_member_id: memberId, profile_revision: profileRevision, is_running: false });
      if ((before[`configured_${wireField}`] ?? null) === expected) {
        await expect(input).toHaveValue(value);
        await expect(detail.locator('[data-member-tuning-status]')).toHaveText("Saved");
        await expect(detail.locator('[data-member-tuning-revision]')).toHaveText(`Revision: ${profileRevision}`);
        expect(pmEditorRequests.slice(start).filter(request => request.kind === "set_pm_launch_profile"), "an unchanged saved field needs no write").toEqual([]);
        return;
      }
      if (field === "speed") await input.selectOption(value); else await input.fill(value);
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
        const status = await latestPmStatus(page);
        return { project: status?.project_key, member: status?.configured_member_id, revision: status?.profile_revision,
          value: status?.[`configured_${wireField}`] ?? null, running: status?.is_running };
      }).toEqual({ project: projectKey, member: memberId, revision: profileRevision, value: expected, running: false });
      await expect(detail.locator('[data-member-tuning-status]')).toHaveText("Saved");
      await expect(detail.locator('[data-member-tuning-revision]')).toHaveText(`Revision: ${profileRevision}`);
    };
    for (const [field, value] of [["agent_id", "codex"], ["model", "gpt-6-astra"], ["reasoning", "high"]]) await saveProfile(field, value);
    const speed = detail.locator('[data-member-tuning-field="speed"]');
    await expect(speed).toBeVisible();
    await expect(speed.locator("option")).toHaveText(["Agent Settings default", "Off", "Fast", "Ultrafast"]);
    for (const value of ["off", "fast", "", "off", "ultrafast"]) {
      await saveProfile("speed", value);
      await expect(speed).toHaveValue(value);
      await expect.poll(() => latestPmStatus(page)).toMatchObject({ configured_agent_id: "codex", configured_model: "gpt-6-astra", configured_reasoning: "high", is_running: false });
    }
    const openSettings = async () => {
      await page.locator(".pm-launcher-shell").hover();
      await page.locator("#op-pm-settings-button").click();
      await expect(surface).toBeVisible();
      await expect(surface.locator('[data-tab="team"]')).toHaveAttribute("aria-selected", "true");
      await expect(surface.locator(`.roster-row[data-member-id="${memberId}"]`)).toHaveAttribute("aria-selected", "true");
      await expect(page.locator('[data-settings-tab="project-manager"]')).toHaveCount(0);
    };
    await page.reload();
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    // Navigation needs the current project's hydrated PM identity.
    await expect.poll(() => latestPmStatus(page)).toMatchObject({ project_key: projectKey, configured_member_id: memberId,
      profile_revision: profileRevision, configured_agent_id: "codex", configured_model: "gpt-6-astra", configured_reasoning: "high", configured_codex_speed: "ultrafast", is_running: false });
    await openSettings();
    await expect(speed).toHaveValue("ultrafast");
    await expect(detail.locator('[data-member-tuning-field="agent_id"]')).toHaveValue("codex");
    await expect(detail.locator('[data-member-tuning-field="model"]')).toHaveValue("gpt-6-astra");
    await expect(detail.locator('[data-member-tuning-field="reasoning"]')).toHaveValue("high");
    await expect(detail.locator('[data-member-tuning-status]')).toHaveText("Saved");
    // The same shared editor keeps the Codex-only tier out of Claude's choices.
    await saveProfile("agent_id", "claude");
    await expect(speed.locator('option[value="ultrafast"]')).toHaveCount(0);
    await expect(speed.locator("option")).toHaveText(["Agent Settings default", "Off", "Fast"]);
    expect((await latestPmStatus(page)).configured_codex_speed ?? null).toBeNull();
    expect(pmEditorRequests.filter(request => ["open_pm_agent", "restart_pm_agent"].includes(request.kind))).toEqual([]);
  });

});

async function latestPmStatus(page: Page) {
  return page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
    .filter(({ payload }: any) => payload.kind === "pm_status").at(-1)?.payload);
}

async function latestWizardView(page: Page) {
  return page.evaluate(() => {
    const messages = (window as any).__gwtPlaywrightMessages || [];
    return [...messages].reverse().find((entry: any) =>
      entry.payload?.kind === "launch_wizard_state" && entry.payload.wizard
    )?.payload.wizard;
  });
}

// SPEC-2014 2026-05-29 — Agent renders as a segmented radiogroup when the
// detected-agent count is small, and falls back to a <select> when custom
// agents push the count past the budget. Select control-agnostically.
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
  const option = wizard.locator(
    `.launch-segmented__option[data-value="${agentId}"]`,
  );
  await option.click();
  await expect(option).toHaveAttribute("aria-checked", "true");
  await blurActiveElement(page);
}

async function blurActiveElement(page: Page): Promise<void> {
  await page.evaluate(() => {
    const active = document.activeElement;
    if (active instanceof HTMLElement) active.blur();
  });
}
