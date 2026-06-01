/**
 * SPEC-2012/SPEC-2014 2026-06-01 - Launch Wizard reconnect recovery.
 *
 * Runs against a real gwt browser-server backend. The test injects a stale
 * Launch Wizard state into the frontend, then sends `frontend_ready` over the
 * live WebSocket. The backend must reply with the authoritative
 * `launch_wizard_state` tombstone (`wizard: null`) so the stale modal closes.
 */
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

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

    await sendLiveGwtEvent(page, { kind: "frontend_ready" });

    await expect(wizard).toHaveAttribute("aria-hidden", "true");
    await expect(wizard).toBeHidden();
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

async function suppressInitialFrontendReady(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const originalSend = WebSocket.prototype.send;
    WebSocket.prototype.send = function sendWithInitialReadySuppressed(data) {
      try {
        const payload = typeof data === "string" ? JSON.parse(data) : null;
        if (
          payload?.kind === "frontend_ready" &&
          (window as any).__gwtDropInitialFrontendReady !== false
        ) {
          (window as any).__gwtDropInitialFrontendReady = false;
          return;
        }
      } catch {
        /* no-op */
      }
      return originalSend.call(this, data);
    };
  });
}

async function clearBackendLaunchWizard(page: Page): Promise<void> {
  const wizard = page.locator("#wizard-modal");
  await sendLiveGwtEvent(page, {
    kind: "launch_wizard_action",
    action: { kind: "cancel" },
    bounds: null,
  });
  await expect(wizard).toBeHidden();
  await expect(wizard.locator(".wizard-summary-item")).toHaveCount(0);
  await expect(wizard).not.toContainText("Work launch");
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
