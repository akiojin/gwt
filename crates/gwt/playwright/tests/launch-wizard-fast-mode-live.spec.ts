/** Issue #7030: explicit speed selection with real backend state, without starting paid inference. */
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

  test.beforeEach(async ({ page }, testInfo) => {
    errors = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    release = await acquireLiveGwtBackendLock(BASE, testInfo);
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

  test("PM settings save and restore explicit Codex speed without starting PM", async ({ page }) => {
    await sendLiveGwtEvent(page, {
      kind: "set_pm_launch_profile", agent_id: "codex", model: "gpt-6-astra",
      reasoning: "high", codex_speed: "off",
    });
    await expect.poll(() => latestPmStatus(page)).toMatchObject({
      configured_agent_id: "codex", configured_codex_speed: "off", is_running: false,
    });
    const openSettings = async () => {
      await page.locator(".pm-launcher-shell").hover();
      await page.locator("#op-pm-settings-button").click();
      await expect(page.locator('.workspace-window[data-preset="settings"]')).toBeVisible();
    };
    await openSettings();
    const panel = page.locator('.workspace-window[data-preset="settings"] [data-settings-panel="project-manager"]');
    const speed = panel.locator('[data-role="pm-speed-select"]');
    await expect(speed).toBeVisible();
    await expect(speed.locator("option")).toHaveText(["Agent Settings default", "Off", "Fast", "Ultrafast"]);
    for (const value of ["fast", "off", "ultrafast"]) {
      await speed.selectOption(value);
      await expect.poll(() => latestPmStatus(page)).toMatchObject({
        configured_agent_id: "codex", configured_model: "gpt-6-astra",
        configured_reasoning: "high", configured_codex_speed: value, is_running: false,
      });
      await expect(speed).toHaveValue(value);
    }
    await page.reload();
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await openSettings();
    await expect(speed).toHaveValue("ultrafast");
    await expect.poll(() => latestPmStatus(page)).toMatchObject({
      configured_agent_id: "codex", configured_model: "gpt-6-astra",
      configured_reasoning: "high", configured_codex_speed: "ultrafast", is_running: false,
    });
    // Selecting Claude must never offer the Codex-only tier.
    await panel.locator('[data-role="pm-agent-select"]').selectOption("claude");
    await expect(speed.locator('option[value="ultrafast"]')).toHaveCount(0);
    await expect(speed.locator("option")).toHaveText(["Agent Settings default", "Off", "Fast"]);
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
