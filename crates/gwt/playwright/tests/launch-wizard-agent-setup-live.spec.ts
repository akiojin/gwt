/**
 * SPEC-1921 Phase L1a / SPEC-3864 — detection drives the Launch Wizard.
 *
 * Runs against this checkout's real backend with a fresh isolated HOME,
 * Claude, Codex and Hermes detected, and `agy` plus GWT_E2E_NPM_AGENT absent
 * from the effective PATH:
 *
 *   - Detected agents show their version in the launch summary without a
 *     Version picker. Claude and Codex retain their Update affordance.
 *   - Undetected built-ins are absent regardless of distribution route.
 *   - Installed Hermes without first-time config retains its Configure action.
 *
 * Like the other live specs, the suite is gated on GWT_PLAYWRIGHT_BASE_URL and
 * never submits a launch. Since SPEC-3245 Stage E removed the deprecated Intake
 * route, the wizard is opened through the normal branch Launch Wizard fixture.
 */
import { expect, test, type Page } from "@playwright/test";
import {
  acquireLiveGwtBackendLock,
  clearLiveLaunchWizard,
  gotoLiveGwt,
  openLiveGwtProject,
  openLiveLaunchWizardForBranch,
  type LiveLaunchWizardFixture,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
// npm-routed built-in that must be absent from the backend's PATH. Defaults to
// OpenClaw (AC-12); on hosts where gwt's macOS PATH hydration re-adds a
// Homebrew-installed openclaw, point this at another npm-routed agent that is
// genuinely missing (e.g. `opencode`) — the wizard path under test is the same.
const NPM_AGENT = process.env.GWT_E2E_NPM_AGENT ?? "openclaw";

test.describe.serial("Launch Wizard agent setup affordance (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");
  test.setTimeout(120_000);

  let releaseBackendLock: (() => Promise<void>) | undefined;
  let wizardFixture: LiveLaunchWizardFixture | undefined;
  let errors: ReturnType<typeof collectErrors>;

  test.beforeEach(async ({ page }, testInfo) => {
    errors = collectErrors(page);
    releaseBackendLock = await acquireLiveGwtBackendLock(BASE, testInfo);
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await keepLaunchWizardModalVisible(page);
    await openLiveGwtProject(page);
    await clearLiveLaunchWizard(page);
  });

  test.afterEach(async ({ page }) => {
    if (!releaseBackendLock) return;
    try {
      try {
        await clearLiveLaunchWizard(page);
      } finally {
        await wizardFixture?.cleanup();
      }
    } finally {
      wizardFixture = undefined;
      await releaseBackendLock();
      releaseBackendLock = undefined;
    }
    expect(errors.pageErrors, "page errors across the live flow").toEqual([]);
    expect(errors.consoleErrors, "console errors across the live flow").toEqual([]);
  });

  for (const [agentId, displayName] of [["claude", "Claude Code"], ["codex", "Codex"]]) {
    test(`${displayName} shows detected version and Update without a Version picker`, async ({ page }, testInfo) => {
      wizardFixture = await openLiveLaunchWizardForBranch(page);
      await enterManualSetupSettings(page);
      await selectWizardAgent(page, agentId);
      const wizard = page.locator("#wizard-modal");
      await expect(wizard.getByLabel("Version", { exact: true })).toHaveCount(0);
      await expect(summaryValue(page, "Agent")).toHaveText(displayName);
      const version = summaryValue(page, "Version");
      await expect(version).toBeVisible();
      await expect(version).toHaveText(/\d+\.\d+/);
      const setup = wizard.locator(`.launch-agent-setup[data-agent-id="${agentId}"]`);
      await expect(setup).toBeVisible();
      await expect(setup).toHaveAttribute("data-setup-kind", "update");
      await expect(setup).toContainText("Launch uses the detected CLI directly");
      await expect(setup).not.toContainText("package runner");
      await expect(setup.getByRole("button", { name: `Update ${displayName}`, exact: true })).toBeVisible();
      await page.evaluate(() => new Promise(requestAnimationFrame));
      await setup.scrollIntoViewIfNeeded();
      await page.screenshot({ path: testInfo.outputPath(`${agentId}-setup.png`), fullPage: true });
      await version.scrollIntoViewIfNeeded();
      await page.screenshot({ path: testInfo.outputPath(`${agentId}-detected-version.png`), fullPage: true });
    });
  }

  test("uninstalled built-ins are absent for installer and npm distribution routes", async ({
    page,
  }) => {
    wizardFixture = await openLiveLaunchWizardForBranch(page);
    const wizard = page.locator("#wizard-modal");
    await enterManualSetupSettings(page);

    const agents = wizard.getByLabel("Agent", { exact: true });
    await expect(agents).toBeVisible();
    for (const agentId of ["agy", NPM_AGENT]) {
      await expect(agents.locator(`option[value="${agentId}"], .launch-segmented__option[data-value="${agentId}"]`)).toHaveCount(0);
      await expect(wizard.locator(`.launch-agent-setup[data-agent-id="${agentId}"]`)).toHaveCount(0);
    }
    await expect(wizard.getByLabel("Version", { exact: true })).toHaveCount(0);
    await expect(wizard.locator('option[value="installed"], option[value="latest"]')).toHaveCount(0);
  });

  test("installed Hermes retains first-time Configure without a Version picker", async ({
    page,
  }, testInfo) => {
    wizardFixture = await openLiveLaunchWizardForBranch(page);
    const wizard = page.locator("#wizard-modal");
    await enterManualSetupSettings(page);

    await selectWizardAgent(page, "hermes");
    await expect(wizard.getByLabel("Version", { exact: true })).toHaveCount(0);
    await expect(summaryValue(page, "Agent")).toHaveText("Hermes Agent");
    const setup = wizard.locator('.launch-agent-setup[data-agent-id="hermes"]');
    await expect(setup).toBeVisible();
    await expect(setup).toHaveAttribute("data-setup-kind", "configure");
    await expect(setup.locator(".launch-agent-setup__detail")).toContainText("hermes setup");
    await expect(setup.getByRole("button", { name: "Run Hermes Agent setup", exact: true })).toBeVisible();
    await page.evaluate(() => new Promise(requestAnimationFrame));
    await setup.scrollIntoViewIfNeeded();
    await page.screenshot({ path: testInfo.outputPath("hermes-configure.png"), fullPage: true });
  });
});

function summaryValue(page: Page, label: string) {
  return page.locator("#wizard-summary .wizard-summary-item")
    .filter({ has: page.locator(".wizard-summary-label", { hasText: new RegExp(`^${label}$`) }) })
    .locator(".wizard-summary-value");
}

function collectErrors(page: Page) {
  const pageErrors: string[] = [];
  const consoleErrors: string[] = [];
  page.on("pageerror", (error) => pageErrors.push(String(error)));
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
  return { pageErrors, consoleErrors };
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
  await page.evaluate(() => {
    const active = document.activeElement;
    if (active instanceof HTMLElement) active.blur();
  });
}

async function enterManualSetupSettings(page: Page): Promise<void> {
  const wizard = page.locator("#wizard-modal");
  const target = wizard.getByRole("radiogroup", { name: "Target" });
  if (await target.isVisible().catch(() => false)) {
    return;
  }
  // #4963: cold Issue Monitor cache preparation can delay the first wizard.
  // Let only this readiness fence use the existing 120s outer test deadline.
  const configure = wizard.getByRole("button", { name: /^Configure and start/ });
  await configure.waitFor({ state: "visible", timeout: 0 });
  await expect(configure).toBeEnabled();
  await configure.click();
  await expect(target).toBeVisible({ timeout: 10_000 });
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
