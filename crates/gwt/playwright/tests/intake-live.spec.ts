import { expect, test } from "@playwright/test";
import {
  clearLiveLaunchWizard,
  gotoLiveGwt,
  openLiveGwtProject,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test.describe("Live Intake wizard", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");
  test.setTimeout(120_000);
  test.use({ viewport: { width: 1440, height: 900 } });

  test("clicking Intake stays branded as Intake after backend hydration", async ({
    page,
  }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      // Prove one backend roundtrip before exercising the real rail click.
      // Otherwise a still-opening WebSocket can leave only the local pending
      // shell visible for the entire assertion timeout.
      await openLiveGwtProject(page);
      await clearLiveLaunchWizard(page);

      const wizard = page.locator("#wizard-modal");
      const intake = page.locator('.op-rail [data-cmd="intake-session"]').first();
      await expect(intake).toBeVisible({ timeout: 10_000 });
      await intake.click();

      await expect(wizard).toHaveClass(/\bopen\b/, { timeout: 10_000 });
      await expect(wizard).toContainText("Register an Issue", {
        timeout: 90_000,
      });
      await expect(wizard.locator("#wizard-title")).toHaveText("Intake");
      await expect(wizard.locator("#wizard-meta")).toHaveText("Curate session");
      await expect(wizard).not.toContainText("Plan Agent");
      await expect(wizard).not.toContainText("Start Work");
      await expect(wizard).not.toContainText("Launch Agent");
      await expect(wizard).not.toContainText("Start methods");
      await expect(wizard).not.toContainText("Start with last settings");
      await expect(wizard).not.toContainText("Other ways to start or resume this agent.");
      await expect(wizard).not.toContainText("Opens the agent's session picker");

      await page.screenshot({
        path:
          process.env.GWT_PLAYWRIGHT_INTAKE_SCREENSHOT_PATH ??
          testInfo.outputPath("intake-live.png"),
        fullPage: true,
      });
    });
  });
});
