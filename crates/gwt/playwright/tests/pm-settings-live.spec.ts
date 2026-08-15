/* SPEC-3431 FR-132 — live Project Manager Settings verification. */
import { expect, test } from "@playwright/test";
import { basename, join } from "node:path";
import {
  gotoLiveGwt,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.describe("Project Manager Settings", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");
  test.use({ viewport: { width: 1440, height: 900 } });
  test.setTimeout(60_000);

  test("rail routing, validation, persistence, and theme remain live", async ({
    page,
  }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      const consoleErrors: string[] = [];
      const pageErrors: string[] = [];
      const failedResources: string[] = [];
      page.on("console", (message) => {
        if (message.type() === "error") consoleErrors.push(message.text());
      });
      page.on("pageerror", (error) => pageErrors.push(error.message));
      page.on("response", (response) => {
        if (response.status() >= 400) {
          failedResources.push(`${response.status()} ${response.url()}`);
        }
      });

      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expectActiveProject(page);
      await openProjectManagerSettings(page);
      const sharedMount = await mountSharedPmSettings(page);

      const settingsWindow = page.locator(
        '.workspace-window[data-preset="settings"]',
      );
      const panel = settingsWindow.locator(
        '[data-settings-panel="project-manager"]',
      );
      const interval = panel.locator('[data-role="pm-loop-interval"]');
      const intervalError = panel.locator(
        '[data-role="pm-loop-interval-error"]',
      );

      await expect(settingsWindow).toBeVisible();
      await expect(
        settingsWindow.locator('[data-settings-tab="project-manager"]'),
      ).toHaveAttribute("aria-selected", "true");
      await expect(panel).toBeVisible();
      await expect(interval).toHaveValue("60");
      await expect(sharedMount).toHaveValue("60");
      const originalInterval = await interval.inputValue();

      try {
        await injectPmStatus(page, { available: false });
        await expect(interval).toHaveValue("");
        await expect(interval).toBeDisabled();
        await expect(sharedMount).toHaveValue("");
        await expect(sharedMount).toBeDisabled();

        const refreshCursor = await messageCursor(page);
        await page.locator('.project-tab.active[aria-current="page"]').click({
          position: { x: 20, y: 20 },
        });
        await waitForPmInterval(page, refreshCursor, originalInterval);
        await expect(interval).toHaveValue(originalInterval);
        await expect(interval).toBeEnabled();
        await expect(sharedMount).toHaveValue(originalInterval);
        await expect(sharedMount).toBeEnabled();

        await interval.fill("9");
        await interval.press("Tab");
        await expect(intervalError).toContainText("at least 10 seconds");
        await expect(interval).toHaveAttribute("aria-invalid", "true");

        const tenCursor = await messageCursor(page);
        await interval.fill("10");
        await interval.press("Tab");
        await waitForPmInterval(page, tenCursor, "10");
        await expect(sharedMount).toHaveValue("10");
        await expect(intervalError).toBeHidden();
        await expect(interval).toHaveAttribute("aria-invalid", "false");

        await page.reload();
        await suppressStartupOverlays(page);
        await expectActiveProject(page);
        await openProjectManagerSettings(page);
        const reloadedInterval = pmInterval(page);
        await expect(reloadedInterval).toHaveValue("10");

        const expectedTheme = testInfo.project.name.endsWith("light")
          ? "light"
          : "dark";
        await expect(page.locator("html")).toHaveAttribute(
          "data-theme",
          expectedTheme,
        );

        await testInfo.attach(`pm-settings-${expectedTheme}`, {
          body: await settingsWindow.screenshot(),
          contentType: "image/png",
        });
        const screenshotDir = process.env.GWT_PM_SETTINGS_SCREENSHOT_DIR;
        if (screenshotDir) {
          await settingsWindow.screenshot({
            path: join(screenshotDir, `pm-settings-${expectedTheme}.png`),
          });
        }

        await reloadedInterval.scrollIntoViewIfNeeded();
        await expect(reloadedInterval).toBeVisible();
        await testInfo.attach(`pm-settings-${expectedTheme}-interval`, {
          body: await settingsWindow.screenshot(),
          contentType: "image/png",
        });
        if (screenshotDir) {
          await settingsWindow.screenshot({
            path: join(
              screenshotDir,
              `pm-settings-${expectedTheme}-interval.png`,
            ),
          });
        }

        expect(failedResources).toEqual([]);
        expect(consoleErrors).toEqual([]);
        expect(pageErrors).toEqual([]);
      } finally {
        await restoreInterval(page, originalInterval);
      }
    });
  });
});

const PROJECT_ROOT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";
const PROJECT_NAME = basename(PROJECT_ROOT);

async function expectActiveProject(page: any): Promise<void> {
  expect(PROJECT_ROOT).not.toBe("");
  const active = page.locator('.project-tab.active[aria-current="page"]');
  await expect(active).toHaveCount(1, { timeout: 10_000 });
  await expect(active).toHaveAttribute("data-project-root", PROJECT_ROOT);
  await expect(active.locator(".project-tab-label")).toHaveText(PROJECT_NAME);
}

async function mountSharedPmSettings(page: any): Promise<any> {
  await page.evaluate(() => {
    const mount = document.createElement("section");
    mount.id = "pm-settings-shared-test-mount";
    mount.hidden = true;
    document.body.appendChild(mount);
    (window as any).__gwtPmSettingsTestApi.mount(mount);
  });
  return page.locator(
    '#pm-settings-shared-test-mount [data-role="pm-loop-interval"]',
  );
}

async function injectPmStatus(page: any, detail: unknown): Promise<void> {
  await page.evaluate((status) => {
    window.dispatchEvent(
      new CustomEvent("__gwt_test_inject", {
        detail: { kind: "pm_status", ...(status as object) },
      }),
    );
  }, detail);
}

async function openProjectManagerSettings(page: any): Promise<void> {
  const launcher = page.locator(".pm-launcher-shell");
  await expect(launcher).toBeVisible({ timeout: 10_000 });
  await launcher.hover();
  await page.locator("#op-pm-settings-button").click();
  await expect(
    page.locator('.workspace-window[data-preset="settings"]'),
  ).toBeVisible({ timeout: 10_000 });
}

function pmInterval(page: any): any {
  return page.locator(
    '.workspace-window[data-preset="settings"] '
      + '[data-settings-panel="project-manager"] '
      + '[data-role="pm-loop-interval"]',
  );
}

async function suppressStartupOverlays(page: any): Promise<void> {
  await page.addStyleTag({
    content: `
      #op-briefing,
      #project-picker,
      #project-onboarding,
      #preset-modal {
        display: none !important;
        pointer-events: none !important;
      }
    `,
  });
}

async function restoreInterval(page: any, original: string): Promise<void> {
  if (!/^\d+$/.test(original)) return;
  await suppressStartupOverlays(page);
  await expectActiveProject(page);
  await openProjectManagerSettings(page);
  const interval = pmInterval(page);
  await expect(interval).toHaveCount(1);
  const current = await interval.inputValue();
  if (current === original) return;
  const cursor = await messageCursor(page);
  await interval.fill(original);
  await interval.press("Tab");
  await waitForPmInterval(page, cursor, original);
}

async function messageCursor(page: any): Promise<number> {
  return page.evaluate(() =>
    Number((window as any).__gwtPlaywrightMessageSequence) || 0
  );
}

async function waitForPmInterval(
  page: any,
  cursor: number,
  expected: string,
): Promise<void> {
  await page.waitForFunction(
    ({ cursor, expected }) => {
      const messages = (window as any).__gwtPlaywrightMessages;
      return Array.isArray(messages) && messages.some((entry: any) =>
        entry?.sequence > cursor
        && entry?.payload?.kind === "pm_status"
        && entry.payload.loop_interval_secs_decimal === expected
      );
    },
    { cursor, expected },
    { timeout: 10_000 },
  );
}
