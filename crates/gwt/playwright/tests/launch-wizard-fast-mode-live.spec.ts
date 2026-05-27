/**
 * SPEC-2014 2026-05-27 follow-up — Launch Wizard Fast mode live E2E.
 *
 * Runs against a real `gwt serve` backend and exercises the user-facing path
 * that regressed: Start Work -> Configure and start -> Claude Code -> Fast mode
 * -> runtime context resolution. The test stops before the final launch so it
 * does not create a branch or start a real Claude Code process.
 */
import { expect, test, type Page } from "@playwright/test";
import {
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test.describe.serial("Launch Wizard Claude Code Fast mode (live backend)", () => {
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

  test("Claude Code Fast mode stays on after runtime context resolution", async ({
    page,
  }) => {
    await sendLiveGwtEvent(page, { kind: "open_start_work" });

    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toBeVisible();
    await wizard.getByRole("button", { name: "Configure and start" }).click();

    const agentSelect = wizard.getByLabel("Agent", { exact: true });
    await expect(agentSelect).toBeVisible();
    await agentSelect.selectOption("claude");
    await expect(agentSelect).toHaveValue("claude");

    const fastMode = wizard.getByLabel("Use the agent's Fast mode", {
      exact: true,
    });
    await expect(fastMode).toBeVisible();
    await fastMode.setChecked(false);
    await expect(fastModeSummaryValue(page)).toHaveText("off");
    await fastMode.setChecked(true);
    await expect(fastModeSummaryValue(page)).toHaveText("on");

    const submit = page.locator("#wizard-submit-button");
    await expect(submit).toHaveText("Continue");
    await submit.click();

    await expect(submit).toHaveText(/^(Launch|Create and launch)$/);
    await expect(fastModeSummaryValue(page)).toHaveText("on");
  });
});

async function keepLaunchWizardModalVisible(page: Page): Promise<void> {
  await page.addStyleTag({
    content: `
      #wizard-modal[aria-hidden="false"] {
        display: flex !important;
        pointer-events: auto !important;
      }
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

function fastModeSummaryValue(page: Page) {
  return page
    .locator(".wizard-summary-item", { hasText: "Fast mode" })
    .locator(".wizard-summary-value");
}
