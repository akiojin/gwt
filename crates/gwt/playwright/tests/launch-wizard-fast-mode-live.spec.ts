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
const REAL_CLAUDE_LAUNCH = process.env.GWT_PLAYWRIGHT_LAUNCH_REAL_CLAUDE === "1";
const BRANCH_NAME =
  process.env.GWT_PLAYWRIGHT_BRANCH_NAME ?? "work/20260527-0745";

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

    await selectWizardAgent(page, "claude");

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
      await openLaunchWizardForCurrentBranch(page);

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
          await launchedWindow.getByLabel("Close window").click();
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
    return;
  }
  const option = wizard.locator(
    `.launch-segmented__option[data-value="${agentId}"]`,
  );
  await option.click();
  await expect(option).toHaveAttribute("aria-checked", "true");
}

function fastModeSummaryValue(page: Page) {
  return page
    .locator(".wizard-summary-item", { hasText: "Fast mode" })
    .locator(".wizard-summary-value");
}

async function openLaunchWizardForCurrentBranch(page: Page): Promise<void> {
  const workWindow = await createWorkWindow(page);
  const workWindowId = await workWindow.getAttribute("data-id");
  expect(workWindowId).toBeTruthy();
  await sendLiveGwtEvent(page, {
    kind: "open_launch_wizard",
    id: workWindowId,
    branch_name: BRANCH_NAME,
  });
}

async function chooseConfigureAndStart(page: Page): Promise<void> {
  const wizard = page.locator("#wizard-modal");
  const agentSelect = wizard.getByLabel("Agent", { exact: true });
  if (await agentSelect.isVisible().catch(() => false)) {
    return;
  }

  for (let attempt = 0; attempt < 3; attempt += 1) {
    const configureButton = wizard
      .getByRole("button", { name: /Configure and start/ })
      .first();
    if (!(await configureButton.count())) {
      break;
    }
    await expect(configureButton).toBeEnabled({ timeout: 10_000 });
    await configureButton.click();

    try {
      await agentSelect.waitFor({ state: "visible", timeout: 2_000 });
      return;
    } catch {
      // Some start-method layouts require the footer submit after card selection.
    }

    const submit = page.locator("#wizard-submit-button");
    if (await submit.isVisible()) {
      const label = (await submit.textContent())?.trim() ?? "";
      if (label === "Choose start method" && !(await submit.isDisabled())) {
        await submit.click();
        await agentSelect.waitFor({ state: "visible", timeout: 10_000 });
        return;
      }
    }
    await page.waitForTimeout(500);
  }
  await agentSelect.waitFor({ state: "visible", timeout: 10_000 });
}

async function createWorkWindow(page: Page) {
  const beforeIds = await page
    .locator(".workspace-window")
    .evaluateAll((nodes) =>
      nodes.map((node) => (node as HTMLElement).dataset.id || ""),
    );
  await sendLiveGwtEvent(page, {
    kind: "create_window",
    preset: "work",
    bounds: { x: 96, y: 96, width: 880, height: 520 },
  });
  const id = await page
    .waitForFunction(
      ({ beforeIds }) => {
        const seen = new Set(beforeIds);
        const node = Array.from(document.querySelectorAll(".workspace-window"))
          .find((candidate) => !seen.has((candidate as HTMLElement).dataset.id || ""));
        return node ? (node as HTMLElement).dataset.id || "" : "";
      },
      { beforeIds },
    )
    .then((handle) => handle.jsonValue());
  return page.locator(`.workspace-window[data-id="${id}"]`);
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
