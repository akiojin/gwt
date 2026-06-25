/**
 * SPEC-2012/SPEC-2014 2026-06-01 - Launch Wizard reconnect recovery.
 *
 * Runs against a real gwt browser-server backend. The test injects a stale
 * Launch Wizard state into the frontend, then sends `frontend_ready` over the
 * live WebSocket. The backend must reply with the authoritative
 * `launch_wizard_state` tombstone (`wizard: null`) so the stale modal closes.
 */
import { expect, test, type Page } from "@playwright/test";
import {
  gotoLiveGwt,
  sendLiveGwtEvent,
  suppressInitialFrontendReady,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const WIZARD_TOMBSTONE_TIMEOUT_MS = 15_000;

test.describe.serial("Launch Wizard reconnect recovery (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test.beforeEach(async ({ page }, testInfo) => {
    test.skip(
      testInfo.project.name !== "chromium-dark",
      "live Launch Wizard reconnect E2E runs once against the shared backend",
    );
    await suppressInitialFrontendReady(page);
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await keepLaunchWizardModalVisibilityDeterministic(page);
    await clearBackendLaunchWizard(page);
  });

  test("FrontendReady tombstone closes a stale Launch Wizard after reconnect", async ({
    page,
  }) => {
    await injectStaleLaunchWizard(page);

    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toBeVisible();
    await expect(wizard).toContainText("Stale Launch Wizard");

    await page.evaluate(() => {
      (window as any).__gwtDropInitialFrontendReady = false;
    });
    await sendLiveGwtEvent(page, { kind: "frontend_ready" });

    await expect(wizard).toHaveAttribute("aria-hidden", "true", {
      timeout: WIZARD_TOMBSTONE_TIMEOUT_MS,
    });
    await expect(wizard).toBeHidden({ timeout: WIZARD_TOMBSTONE_TIMEOUT_MS });
  });
});

async function keepLaunchWizardModalVisibilityDeterministic(page: Page): Promise<void> {
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

async function clearBackendLaunchWizard(page: Page): Promise<void> {
  const wizard = page.locator("#wizard-modal");
  await expect(async () => {
    await sendLiveGwtEvent(page, {
      kind: "launch_wizard_action",
      action: { kind: "cancel" },
      bounds: null,
    });
    await page.waitForTimeout(250);
    await page.evaluate(() => {
      (window as any).__gwtDropInitialFrontendReady = false;
    });
    await sendLiveGwtEvent(page, { kind: "frontend_ready" });
    await page.waitForTimeout(500);
    await expect(wizard).toBeHidden({ timeout: 1_000 });
    await expect(wizard.locator(".wizard-summary-item")).toHaveCount(0, {
      timeout: 1_000,
    });
    await expect(wizard).not.toContainText("Work launch", { timeout: 1_000 });
  }).toPass({ timeout: 10_000 });
}

async function injectStaleLaunchWizard(page: Page): Promise<void> {
  await page.evaluate((wizard) => {
    window.dispatchEvent(
      new CustomEvent("__gwt_test_inject", {
        detail: {
          kind: "launch_wizard_state",
          wizard,
        },
      }),
    );
  }, {
    title: "Launch Agent",
    branch_name: "work/reconnect-stale",
    selected_branch_name: "work/reconnect-stale",
    show_branch_controls: false,
    show_start_methods: false,
    show_manual_setup: false,
    show_runtime_confirmation: false,
    runtime_context_resolved: true,
    runtime_resolution_pending: false,
    is_hydrating: false,
    show_back_button: false,
    primary_action_label: "Launch",
    primary_action_enabled: true,
    launch_summary: [
      { label: "Recovery", value: "Stale Launch Wizard" },
      { label: "Target", value: "Agent" },
    ],
    progress_steps: [
      { label: "Launch requested", state: "done" },
      {
        label: "Reconnect sync",
        state: "current",
        detail: "Waiting for authoritative backend state",
      },
    ],
    start_methods: [],
  });
}
