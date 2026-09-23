/* SPEC-3431 FR-132 — live Project Manager Settings verification. */
import { expect, test } from "@playwright/test";
import { dirname, join } from "node:path";
import { execFileSync } from "node:child_process";
import { realpathSync } from "node:fs";
import {
  sendLiveGwtEvent,
  gotoLiveGwt,
  openLiveGwtProject,
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
      await openLiveGwtProject(page, PROJECT_ROOT);
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
        await expect(interval).toHaveValue("60");
        await expect(interval).toBeDisabled();
        await expect(sharedMount).toHaveValue("60");
        await expect(sharedMount).toBeDisabled();

        const refreshCursor = await messageCursor(page);
        // Issue #4538: a Project tab re-hydrates through its own scope
        // (re-selecting the bound tab is a no-op in a per-project URL).
        await sendLiveGwtEvent(page, { kind: "frontend_ready" });
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
async function expectActiveProject(page: any): Promise<void> {
  expect(PROJECT_ROOT).not.toBe("");
  const projectKey = new URL(page.url()).pathname.match(/^\/p\/([0-9a-f]{16})$/)?.[1];
  expect(projectKey, "PM Settings must stay bound to a Project route").toBeTruthy();
  await expect(page.locator("#close-project-button")).toBeVisible();
  const projectRoot = await page.waitForFunction((key: string) => {
    const state = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
      entry.payload?.kind === "workspace_state"
      && entry.payload.workspace.tabs.length === 1
      && entry.payload.workspace.tabs[0].project_key === key);
    return state?.payload.workspace.tabs[0].project_root;
  }, projectKey).then((handle: any) => handle.jsonValue());
  // Recent reopen canonicalizes a worktree to its repository root. Accept
  // those two identities, never the helper's unrelated Recent fallback.
  const commonDirectory = execFileSync("git", ["-C", PROJECT_ROOT, "rev-parse",
    "--path-format=absolute", "--git-common-dir"], { encoding: "utf8" }).trim();
  expect([realpathSync(PROJECT_ROOT), dirname(realpathSync(commonDirectory))])
    .toContain(realpathSync(projectRoot));
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
