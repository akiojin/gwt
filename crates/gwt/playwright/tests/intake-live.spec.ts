import { expect, test } from "@playwright/test";
import {
  gotoLiveGwt,
  openLiveGwtProject,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test.describe("Deprecated Intake launch surfaces (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");
  test.setTimeout(120_000);
  test.use({ viewport: { width: 1440, height: 900 } });

  test("Intake launch surfaces are absent while normal canvas actions remain", async ({
    page,
  }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await openLiveGwtProject(page);
      await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });
      await expect(page.locator('.op-rail [data-cmd="intake-session"]')).toHaveCount(0);
      await expect(page.locator("#canvas-empty-intake")).toHaveCount(0);
      await expect(page.locator("#op-workspace-overview-entry")).toBeVisible();
      await expect(page.locator("#canvas-empty-open-workspace")).toBeAttached();
      await expect(page.locator("#canvas-empty-add-window")).toBeAttached();
    });
  });
});
