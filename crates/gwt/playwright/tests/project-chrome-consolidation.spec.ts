// Issue #4539: the former in-page Projects dropdown is retired.
// Hub intake coverage lives in project-hub.spec.ts; close and keyboard
// replacement coverage lives in project-close-detach.spec.ts.
import { expect, test } from "@playwright/test";
import { HUB_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test("Hub has project intake without in-page project switching", async ({ page }) => {
  await installEmbeddedRoutes(page);
  await page.goto(HUB_URL);
  await expect(page.locator("[data-hub-action='open-folder']")).toBeVisible();
  await expect(page.locator("[data-hub-action='clone']")).toBeVisible();
  await expect(page.locator("#project-tabs")).toHaveCount(0);
  await expect(page.locator("#project-switcher-button")).toHaveCount(0);
});
