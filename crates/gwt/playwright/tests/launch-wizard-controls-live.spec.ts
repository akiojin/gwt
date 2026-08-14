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
import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  acquireLiveGwtBackendLock,
  clearLiveGwtLaunchWizard,
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
  suppressInitialFrontendReady,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test.describe.serial("Launch Wizard setting controls (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");
  test.setTimeout(120_000);

  let releaseBackendLock: (() => Promise<void>) | undefined;

  test.beforeEach(async ({ page }, testInfo) => {
    releaseBackendLock = await acquireLiveGwtBackendLock(BASE, testInfo);
    await suppressInitialFrontendReady(page);
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await keepLaunchWizardModalVisible(page);
    await clearLiveGwtLaunchWizard(page);
    await openLiveGwtProject(page);
  });

  test.afterEach(async ({ page }) => {
    if (!releaseBackendLock) return;
    try {
      await clearLiveGwtLaunchWizard(page);
    } finally {
      await releaseBackendLock();
      releaseBackendLock = undefined;
    }
  });

  test("Target is a segmented radiogroup that toggles agent settings", async ({
    page,
  }) => {
    await sendLiveGwtEvent(page, { kind: "open_intake_session" });
    const wizard = page.locator("#wizard-modal");
    await chooseConfigureIntake(page);

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

    await sendLiveGwtEvent(page, { kind: "open_intake_session" });
    const wizard = page.locator("#wizard-modal");
    await chooseConfigureIntake(page);

    const agentField = wizard.getByLabel("Agent", { exact: true });
    const tag = await agentField.evaluate((node) => node.tagName.toLowerCase());
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

  test("PM settings persist a Grok launch profile and render pending restart state", async ({
    page,
  }) => {
    const pageErrors: string[] = [];
    const consoleErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });

    const panel = page.locator("#pm-settings-panel");
    await page.getByRole("button", { name: "Project Manager settings" }).click();
    await expect(panel).toBeVisible();

    const agent = panel.locator('[data-role="pm-agent-select"]');
    const model = panel.locator('[data-role="pm-model-input"]');
    const effort = panel.locator('[data-role="pm-effort-select"]');
    await expect(agent.locator('option[value="grok"]')).toHaveText("Grok Build");

    await agent.selectOption("claude");
    await expect(effort.locator('option[value="ultracode"]')).toHaveText(
      "Ultracode",
    );
    await agent.selectOption("grok");
    await expect(agent).toHaveValue("grok");
    // The select updates locally before the backend's pm_status round-trip.
    // Wait for the Grok-specific effort catalog so a fast model edit cannot
    // be overwritten by the preceding profile snapshot.
    await expect(effort.locator('option[value="none"]')).toHaveText("None");
    await model.fill("team/grok-code-fast");
    await model.blur();
    await expect(async () => {
      await expect(model).toHaveValue("team/grok-code-fast");
      await page.waitForTimeout(250);
      await expect(model).toHaveValue("team/grok-code-fast");
    }).toPass({ timeout: 5_000 });
    await effort.selectOption("high");
    await expect(async () => {
      await expect(effort).toHaveValue("high");
      await expect(model).toHaveValue("team/grok-code-fast");
      await page.waitForTimeout(250);
      await expect(effort).toHaveValue("high");
      await expect(model).toHaveValue("team/grok-code-fast");
    }).toPass({ timeout: 5_000 });
    await expect(effort.locator("option")).toHaveText([
      "Auto",
      "None",
      "Minimal",
      "Low",
      "Medium",
      "High",
      "Extra high",
      "Max",
    ]);

    // Restore the backend profile before using a synthetic running snapshot
    // to exercise the otherwise destructive pending/restart presentation.
    await model.fill("");
    await model.blur();
    await effort.selectOption("");
    await agent.selectOption("claude");
    await expect(agent).toHaveValue("claude");
    await expect(effort.locator('option[value="ultracode"]')).toHaveText(
      "Ultracode",
    );

    await page.evaluate(() => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
        detail: {
          kind: "pm_status",
          auto_start: false,
          agent_options: [
            { id: "claude", name: "Claude Code" },
            { id: "codex", name: "Codex" },
            { id: "grok", name: "Grok Build" },
          ],
          configured_agent_id: "grok",
          configured_model: "team/grok-code-fast",
          configured_reasoning: "high",
          running_agent_id: "claude",
          running_model: "sonnet",
          running_reasoning: "medium",
          is_running: true,
        },
      }));
    });
    await expect(panel.locator('[data-role="pm-running-as"]')).toContainText(
      "Running as: Claude Code · Model: sonnet · Effort: medium",
    );
    await expect(panel.locator('[data-role="pm-pending-chip"]')).toBeVisible();

    let restartPrompt = "";
    page.once("dialog", async (dialog) => {
      restartPrompt = dialog.message();
      await dialog.dismiss();
    });
    await panel.locator('[data-role="pm-restart"]').click();
    await expect.poll(() => restartPrompt).toContain("new conversation");
    await page.evaluate(() => new Promise(requestAnimationFrame));
    expect(pageErrors).toEqual([]);
    expect(consoleErrors).toEqual([]);
  });

  test("Reasoning renders as a slider with a separate Auto toggle", async ({
    page,
  }) => {
    await sendLiveGwtEvent(page, { kind: "open_intake_session" });
    const wizard = page.locator("#wizard-modal");
    await chooseConfigureIntake(page);

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
    await auto.blur();
    await expect(range).toBeEnabled();
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
    await auto.blur();
    await expect(range).toBeDisabled();
    await expect(effortSummaryValue(page)).toHaveText("auto");
  });
});

async function chooseConfigureIntake(page: Page): Promise<void> {
  const wizard = page.locator("#wizard-modal");
  const target = wizard.getByRole("radiogroup", { name: "Target" });
  const configure = wizard.getByRole("button", { name: "Configure intake" });
  await expect(configure).toBeVisible({ timeout: 90_000 });
  await configure.click();
  await expect(target).toBeVisible({ timeout: 90_000 });
}

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
    return;
  }
  const option = wizard.locator(`.launch-segmented__option[data-value="${agentId}"]`);
  await option.click();
  await expect(option).toHaveAttribute("aria-checked", "true");
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
