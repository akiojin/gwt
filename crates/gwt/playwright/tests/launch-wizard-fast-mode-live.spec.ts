/**
 * SPEC-1921 Phase L2 — fixed Launch Wizard permissions and Fast mode.
 *
 * Use browser-check's checkout backend and fresh HOME, with Claude and Codex
 * installed. Before starting the backend, seed CHECK_HOME/.gwt/sessions/
 * launch-wizard-l2-{claude,codex}.toml with skip_permissions=false and
 * fast_mode=true and model="opus" / "gpt-6-astra", using the fixture
 * project's worktree and branch. Set
 * GWT_PLAYWRIGHT_CHECK_HOME to that HOME. The tests only read these Sessions
 * and stop at launch confirmation without starting an agent process.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
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
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";

test.describe.serial("Launch Wizard fixed permissions and Fast mode (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");
  test.setTimeout(120_000);

  let releaseBackendLock: (() => Promise<void>) | undefined;
  let cleanupLaunchFixture: (() => Promise<void>) | undefined;
  let errors: string[];

  test.beforeEach(async ({ page }, testInfo) => {
    cleanupLaunchFixture = undefined;
    errors = [];
    page.on("pageerror", (error) => errors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    releaseBackendLock = await acquireLiveGwtBackendLock(BASE, testInfo);
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    const theme = testInfo.project.use.colorScheme === "light" ? "light" : "dark";
    await page.locator(`#op-theme-toggle [data-theme-value="${theme}"]`).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
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
    expect(errors, "console and page errors across the live flow").toEqual([]);
  });

  for (const [agentId, displayName, savedModel] of [
    ["claude", "Claude Code", "opus"],
    ["codex", "Codex", "gpt-6-astra"],
  ]) {
    test(`${displayName} fixes saved permissions and Fast mode through runtime resolution and reload`, async ({ page }, testInfo) => {
      expect(CHECK_HOME, "Set GWT_PLAYWRIGHT_CHECK_HOME to the seeded isolated HOME").not.toBe("");
      const seedPath = join(CHECK_HOME, ".gwt", "sessions", `launch-wizard-l2-${agentId}.toml`);
      const seed = await readFile(seedPath, "utf8");
      expect(seed).toMatch(/^skip_permissions\s*=\s*false\s*$/m);
      expect(seed).toMatch(/^(?:fast_mode|codex_fast_mode)\s*=\s*true\s*$/m);
      cleanupLaunchFixture = (await openLiveLaunchWizardForBranch(page)).cleanup;
      await expect(page.locator("#wizard-modal")).toBeVisible({ timeout: 30_000 });
      await chooseConfigureAndStart(page);
      await selectWizardAgent(page, agentId);
      // A non-default model proves the backend actually restored the saved
      // profile, rather than merely launching with unrelated fixed defaults.
      await expect.poll(async () => (await latestWizardView(page))?.selected_model, {
        timeout: 30_000,
      }).toBe(savedModel);
      await expectFixedLaunchSettings(page, agentId);

      // Older clients can still send the saved choices. Wait for each real
      // backend response so the existing fixed View cannot satisfy the check.
      for (const action of [
        { kind: "set_skip_permissions", enabled: false },
        { kind: "set_fast_mode", enabled: true },
        { kind: "set_codex_fast_mode", enabled: true },
      ]) {
        const cursor = await page.evaluate(() => Number((window as any).__gwtPlaywrightMessageSequence) || 0);
        await sendLiveGwtEvent(page, { kind: "launch_wizard_action", action, bounds: null });
        await page.waitForFunction((cursor) =>
          ((window as any).__gwtPlaywrightMessages || []).some((entry: any) =>
            entry.sequence > cursor && entry.payload?.kind === "launch_wizard_state" && entry.payload.wizard
          ), cursor, { timeout: 30_000 });
        await expectFixedLaunchSettings(page, agentId);
      }

      const submit = page.locator("#wizard-submit-button");
      await expect(submit).toHaveText("Continue");
      await submit.click();
      await expect.poll(() => latestWizardView(page), { timeout: 30_000 }).toMatchObject({
        runtime_context_resolved: true,
        show_runtime_confirmation: true,
        show_confirm: false,
      });
      await expect(submit).toHaveText("Continue");
      await expectFixedLaunchSettings(page, agentId);
      await submit.click();
      await expect.poll(async () => (await latestWizardView(page))?.show_confirm, {
        timeout: 30_000,
      }).toBe(true);
      await expectFixedLaunchSettings(page, agentId);
      await expect(submit).toHaveText(/^(Launch|Create and launch)$/, { timeout: 30_000 });

      // A stale frontend View must never recreate choices, even when it
      // advertises both the current and the legacy Fast mode field names.
      const current = await latestWizardView(page);
      await page.evaluate((wizard) => {
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "launch_wizard_state",
            wizard: {
              ...wizard,
              show_manual_setup: true,
              show_confirm: false,
              show_runtime_confirmation: false,
              show_skip_permissions: true,
              skip_permissions: false,
              show_fast_mode: true,
              fast_mode: true,
              show_codex_fast_mode: true,
              codex_fast_mode: true,
            },
          },
        }));
      }, current);
      await expectLaunchChoicesAbsent(page);
      await page.reload();
      await expect(page.locator("#wizard-modal")).toBeVisible({ timeout: 30_000 });
      // gotoLiveGwt reinstalls capture on reload; this is a fresh backend View,
      // rather than the page-local legacy payload injected above.
      await expectFixedLaunchSettings(page, agentId);
      const theme = testInfo.project.use.colorScheme === "light" ? "light" : "dark";
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      expect(await readFile(seedPath, "utf8"), "saved legacy preferences remain unchanged").toBe(seed);
      await testInfo.attach(`launch-wizard-l2-${agentId}-${theme}`, {
        body: await page.locator("#wizard-modal").screenshot(),
        contentType: "image/png",
      });
    });
  }
});

async function latestWizardView(page: Page) {
  return page.evaluate(() => {
    const messages = (window as any).__gwtPlaywrightMessages || [];
    return [...messages].reverse().find((entry: any) =>
      entry.payload?.kind === "launch_wizard_state" && entry.payload.wizard
    )?.payload.wizard;
  });
}

async function expectLaunchChoicesAbsent(page: Page): Promise<void> {
  const wizard = page.locator("#wizard-modal");
  await expect(wizard.getByText("Launch settings", { exact: true })).toHaveCount(0);
  await expect(wizard.getByLabel("Skip permission prompts", { exact: true })).toHaveCount(0);
  await expect(wizard.getByLabel("Use the agent's Fast mode", { exact: true })).toHaveCount(0);
  await expect(wizard.locator(".wizard-progress-label").filter({
    hasText: /^(Launch settings|Skip Permissions|Fast mode)$/i,
  })).toHaveCount(0);
}

async function expectFixedLaunchSettings(page: Page, agentId: string): Promise<void> {
  await expect.poll(() => latestWizardView(page), { timeout: 30_000 }).toMatchObject({
    selected_agent_id: agentId,
    skip_permissions: true,
    fast_mode: false,
    show_skip_permissions: false,
    show_fast_mode: false,
  });
  await expectLaunchChoicesAbsent(page);
  await expect(fastModeSummaryValue(page)).toHaveText("off");
  await expect(page.locator("#wizard-summary .wizard-summary-item", {
    has: page.locator(".wizard-summary-label", { hasText: /^Permissions$/ }),
  }).locator(".wizard-summary-value")).toHaveText("skip");
}

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

  const configure = wizard.getByRole("button", { name: /^Configure and start/ });
  await configure.waitFor({ state: "visible", timeout: 0 });
  await expect(configure).toBeEnabled();
  await configure.click();
  await expect(agentSelect).toBeVisible({ timeout: 10_000 });
}

async function blurActiveElement(page: Page): Promise<void> {
  await page.evaluate(() => {
    const active = document.activeElement;
    if (active instanceof HTMLElement) active.blur();
  });
}
