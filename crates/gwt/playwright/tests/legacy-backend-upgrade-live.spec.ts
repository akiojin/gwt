/**
 * #4825 AC-3: a retired custom backend is preserved and its launch explains
 * the supported manual recovery. Uses the real backend, with no fake replies.
 * Before browser-check launch, copy fixtures/legacy-backend-config.toml to
 * CHECK_HOME/.gwt/config.toml. Set GWT_PLAYWRIGHT_CHECK_HOME to that fresh HOME,
 * plus BASE_URL, PROJECT_ROOT and BRANCH_NAME as in the other live tests.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import {
  acquireLiveGwtBackendLock,
  clearLiveLaunchWizard,
  gotoLiveGwt,
  openLiveGwtProject,
  openLiveLaunchWizardForBranch,
  sendLiveGwtEvent,
  type LiveLaunchWizardFixture,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const MESSAGE = "旧 backend 設定は自動移行されません。Settings で provider を再登録してください";

test.describe("Retired backend upgrade (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set");
  test.setTimeout(240_000);

  test("launch explains Settings recovery without migrating or deleting the old config", async ({ page }, testInfo) => {
    expect(CHECK_HOME, "Set GWT_PLAYWRIGHT_CHECK_HOME to the seeded isolated HOME").not.toBe("");
    const release = await acquireLiveGwtBackendLock(BASE, testInfo);
    let fixture: LiveLaunchWizardFixture | undefined;
    let failed = false;
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    try {
      const configPath = join(CHECK_HOME, ".gwt", "config.toml");
      const seed = await readFile(join(__dirname, "../fixtures/legacy-backend-config.toml"), "utf8");
      expect(await readFile(configPath, "utf8")).toContain(seed.trim());
      await gotoLiveGwt(page, BASE, { enableTestBridge: true, hub: true });
      await openLiveGwtProject(page);
      const theme = testInfo.project.use.colorScheme === "light" ? "light" : "dark";
      await page.locator(`#op-theme-toggle [data-theme-value="${theme}"]`).click();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await clearLiveLaunchWizard(page);
      fixture = await openLiveLaunchWizardForBranch(page);
      const wizard = page.locator("#wizard-modal");
      // Cold Windows HOME measured 101–127s Frontend stalls before the first
      // wizard. Bound the real DOM wait; do not retry or warm it up.
      await expect(wizard).toBeVisible({ timeout: 150_000 });
      await sendLiveGwtEvent(page, {
        kind: "launch_wizard_action",
        action: { kind: "set_launch_path", path: "manual_setup" },
        bounds: null,
      });
      const agent = wizard.getByLabel("Agent", { exact: true });
      await expect(agent).toBeVisible();
      await agent.selectOption("legacy-cc");
      await agent.blur();
      await expect(wizard.locator(".wizard-summary-item", {
        has: page.locator(".wizard-summary-label", { hasText: /^Agent$/ }),
      })).toContainText("Legacy Claude Code");
      await wizard.locator("#wizard-submit-button").click();
      await expect(page.locator("#wizard-error")).toHaveText(MESSAGE);
      await expect(page.locator("#wizard-error")).toBeVisible();
      expect(await readFile(configPath, "utf8")).toContain(seed.trim());
      await testInfo.attach(`legacy-backend-${theme}`, {
        body: await wizard.screenshot(),
        contentType: "image/png",
      });
      expect(errors).toEqual([]);
    } catch (error) {
      failed = true;
      throw error;
    } finally {
      try {
        if (fixture) {
          await clearLiveLaunchWizard(page);
          await fixture.cleanup();
        }
      } catch (error) {
        if (!failed) throw error;
        await testInfo.attach("cleanup-error", {
          body: String(error),
          contentType: "text/plain",
        });
      } finally {
        await release();
      }
    }
  });
});
