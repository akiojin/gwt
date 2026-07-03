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
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, openLiveGwtProject, sendLiveGwtEvent } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test.describe.serial("Launch Wizard setting controls (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test.beforeEach(async ({ page }, testInfo) => {
    test.skip(
      testInfo.project.name !== "chromium-dark",
      "live Launch Wizard E2E runs once against the shared backend",
    );
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await keepLaunchWizardModalVisible(page);
    await openLiveGwtProject(page);
  });

  test("Target is a segmented radiogroup that toggles agent settings", async ({
    page,
  }) => {
    await sendLiveGwtEvent(page, { kind: "open_intake" });
    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toBeVisible();
    await wizard.getByRole("button", { name: "Configure and start" }).click();

    const target = wizard.getByRole("radiogroup", { name: "Target" });
    await expect(target).toBeVisible();
    const shell = target.locator('.launch-segmented__option[data-value="shell"]');
    const agent = target.locator('.launch-segmented__option[data-value="agent"]');
    await expect(agent).toHaveAttribute("aria-checked", "true");

    await shell.click();
    await expect(shell).toHaveAttribute("aria-checked", "true");
    await expect(targetSummaryValue(page)).toHaveText("Shell");
    // Agent-only controls disappear when Shell is the launch target.
    await expect(wizard.getByRole("radiogroup", { name: "Agent" })).toHaveCount(0);

    await agent.click();
    await expect(agent).toHaveAttribute("aria-checked", "true");
    await expect(targetSummaryValue(page)).toHaveText("Agent");
  });

  test("Reasoning renders as a slider with a separate Auto toggle", async ({
    page,
  }) => {
    await sendLiveGwtEvent(page, { kind: "open_intake" });
    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toBeVisible();
    await wizard.getByRole("button", { name: "Configure and start" }).click();

    await selectWizardAgent(page, "claude");
    // Pin an effort-capable model so the reasoning control is shown
    // deterministically (Sonnet exposes Auto / Low / Medium / High).
    const model = wizard.getByLabel("Model", { exact: true });
    if ((await model.evaluate((n) => n.tagName.toLowerCase())) === "select") {
      await model.selectOption("sonnet");
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
    await expect(range).toBeDisabled();
    await expect(effortSummaryValue(page)).toHaveText("auto");
  });
});

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

function effortSummaryValue(page: Page) {
  return page
    .locator(".wizard-summary-item", { hasText: "Effort" })
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
