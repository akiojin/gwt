/**
 * SPEC-2014 2026-05-27 follow-up — Launch Wizard Fast mode live E2E.
 *
 * Runs against a real gwt browser-server backend and exercises the user-facing path
 * that regressed: Work branch -> Configure launch -> Claude Code -> Fast mode
 * -> runtime context resolution. The test stops before the final launch so it
 * does not create a branch or start a real Claude Code process.
 */
import { expect, test, type Page } from "@playwright/test";
import {
  acquireLiveGwtBackendLock,
  clearLiveLaunchWizard,
  gotoLiveGwt,
  openLiveLaunchWizardForBranch,
  openLiveGwtProject,
  sendLiveGwtEvent,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const REAL_CLAUDE_LAUNCH = process.env.GWT_PLAYWRIGHT_LAUNCH_REAL_CLAUDE === "1";

test.describe.serial("Launch Wizard Claude Code Fast mode (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");
  test.setTimeout(120_000);

  let releaseBackendLock: (() => Promise<void>) | undefined;
  let cleanupLaunchFixture: (() => Promise<void>) | undefined;

  test.beforeEach(async ({ page }, testInfo) => {
    cleanupLaunchFixture = undefined;
    test.skip(
      testInfo.project.name !== "chromium-dark",
      "live Launch Wizard E2E runs once against the shared backend",
    );
    releaseBackendLock = await acquireLiveGwtBackendLock(BASE, testInfo);
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await keepLaunchWizardModalVisible(page);
    await openLiveGwtProject(page);
    await clearLiveLaunchWizard(page);
  });

  test.afterEach(async ({ page }) => {
    if (!releaseBackendLock) return;
    const cleanup = cleanupLaunchFixture;
    cleanupLaunchFixture = undefined;
    try {
      try {
        await clearLiveLaunchWizard(page);
      } finally {
        await cleanup?.();
      }
    } finally {
      await releaseBackendLock();
      releaseBackendLock = undefined;
    }
  });

  test("Claude Code Fast mode stays on after runtime context resolution", async ({
    page,
  }) => {
    cleanupLaunchFixture = (await openLiveLaunchWizardForBranch(page)).cleanup;

    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toBeVisible();
    await chooseConfigureAndStart(page);

    await selectWizardAgent(page, "claude");

    const fastMode = wizard.getByLabel("Use the agent's Fast mode", {
      exact: true,
    });
    await expect(fastMode).toBeVisible();
    await fastMode.setChecked(false);
    await blurActiveElement(page);
    await expect(fastModeSummaryValue(page)).toHaveText("off");
    await fastMode.setChecked(true);
    await blurActiveElement(page);
    await expect(fastModeSummaryValue(page)).toHaveText("on");

    const submit = page.locator("#wizard-submit-button");
    await expect(submit).toHaveText("Continue");
    await submit.click();

    // Runtime resolution and launch materialization surface transient
    // "Preparing..." / "Launching..." labels; on a loaded host they outlast the
    // default expect timeout, so wait for the settled label explicitly.
    await expect(submit).toHaveText(/^(Continue|Launch)$/, { timeout: 30_000 });
    await expect(fastModeSummaryValue(page)).toHaveText("on");
    if ((await submit.textContent())?.trim() === "Continue") {
      await submit.click();
    }

    await expect(submit).toHaveText(/^(Launch|Create and launch)$/, {
      timeout: 30_000,
    });
    await expect(fastModeSummaryValue(page)).toHaveText("on");
  });

  test("Claude Code launches with the Fast mode indicator visible", async ({
    page,
  }) => {
    test.setTimeout(120_000);
    test.skip(
      !REAL_CLAUDE_LAUNCH,
      "set GWT_PLAYWRIGHT_LAUNCH_REAL_CLAUDE=1 to launch the real Claude Code process",
    );

    const beforeIds = await claudeWindowIds(page);
    let launchedWindowId: string | null = null;
    try {
      cleanupLaunchFixture = (await openLiveLaunchWizardForBranch(page)).cleanup;

      const wizard = page.locator("#wizard-modal");
      await expect(wizard).toBeVisible();
      await chooseConfigureAndStart(page);

      await selectWizardAgent(page, "claude");
      await wizard
        .getByLabel("Use the agent's Fast mode", { exact: true })
        .setChecked(true);
      await expect(fastModeSummaryValue(page)).toHaveText("on");

      const submit = page.locator("#wizard-submit-button");
      await expect(submit).toHaveText("Continue");
      await submit.click();
      await expect(submit).toHaveText("Continue");
      await expect(fastModeSummaryValue(page)).toHaveText("on");
      await submit.click();
      await expect(submit).toHaveText("Launch");
      await submit.click();

      const agentWindow = await waitForNewClaudeWindow(page, beforeIds);
      launchedWindowId = await agentWindow.getAttribute("data-id");
      await expect(agentWindow.locator(".title-text")).toHaveText("Claude Code");
      await expect(agentWindow.locator(".status-chip")).toBeVisible();
      await expect(async () => {
        const text = await agentWindow.locator(".terminal-root").textContent();
        expect(text ?? "").toMatch(/[⚡↯]/);
      }).toPass({ timeout: 45_000 });
    } finally {
      if (launchedWindowId) {
        const launchedWindow = page.locator(
          `.workspace-window[data-id="${launchedWindowId}"]`,
        );
        if (await launchedWindow.count()) {
          // SPEC-3038 US-3 Close Guard: the titlebar × always opens a confirm
          // modal; the close only happens after the destructive confirm.
          await launchedWindow.getByLabel("Close window").click();
          const closeConfirm = page.locator(
            '#window-close-confirm-modal [data-role="window-close-confirm"]',
          );
          await expect(closeConfirm).toBeVisible();
          await closeConfirm.click();
          await expect(launchedWindow).toHaveCount(0, { timeout: 15_000 });
        }
      }
    }
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

function fastModeSummaryValue(page: Page) {
  return page
    .locator("#wizard-summary .wizard-summary-item", { hasText: "Fast mode" })
    .locator(".wizard-summary-value");
}

async function chooseConfigureAndStart(page: Page): Promise<void> {
  const wizard = page.locator("#wizard-modal");
  const agentSelect = wizard.getByLabel("Agent", { exact: true });
  if (await agentSelect.isVisible().catch(() => false)) {
    return;
  }

  await sendLiveGwtEvent(page, {
    kind: "launch_wizard_action",
    action: { kind: "set_launch_path", path: "manual_setup" },
    bounds: null,
  });
  await agentSelect.waitFor({ state: "visible", timeout: 10_000 });
}

async function blurActiveElement(page: Page): Promise<void> {
  await page.evaluate(() => {
    const active = document.activeElement;
    if (active instanceof HTMLElement) active.blur();
  });
}

async function claudeWindowIds(page: Page): Promise<string[]> {
  return page
    .locator(".workspace-window", { hasText: "Claude Code" })
    .evaluateAll((nodes) =>
      nodes
        .map((node) => (node as HTMLElement).dataset.id || "")
        .filter(Boolean),
    );
}

async function waitForNewClaudeWindow(page: Page, beforeIds: string[]) {
  const id = await page
    .waitForFunction(
      ({ beforeIds }) => {
        const seen = new Set(beforeIds);
        const node = Array.from(document.querySelectorAll(".workspace-window"))
          .find((candidate) => {
            const element = candidate as HTMLElement;
            const title = element.querySelector(".title-text")?.textContent?.trim();
            return title === "Claude Code" && !seen.has(element.dataset.id || "");
          });
        return node ? (node as HTMLElement).dataset.id || "" : "";
      },
      { beforeIds },
      { timeout: 60_000 },
    )
    .then((handle) => handle.jsonValue());
  return page.locator(`.workspace-window[data-id="${id}"]`);
}
