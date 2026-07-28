/**
 * Issue #2796 — Mission Briefing must never block a live gwt client.
 *
 * These tests deliberately suppress `frontend_ready` so the workspace
 * fail-open path cannot hide the overlay for us. That leaves the briefing's
 * own timeout, early-dismiss, and sessionStorage contracts under test.
 */
import { expect, test, type Page } from "@playwright/test";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test.describe("Mission Briefing dismissal (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test.beforeEach(async ({ page }) => {
    await suppressFrontendReady(page);
  });

  test("auto-dismisses without workspace state assistance", async ({ page }) => {
    await page.goto(BASE);

    const briefing = page.locator("#op-briefing");
    await expect(briefing).toBeHidden({ timeout: 3_000 });
  });

  test("a click dismisses the overlay immediately", async ({ page }) => {
    await page.goto(BASE);

    const briefing = page.locator("#op-briefing");
    await expect(briefing).toBeVisible();
    await briefing.click();
    await expect(briefing).toBeHidden({ timeout: 1_000 });
  });

  test("a previously shown briefing stays hidden", async ({ page }) => {
    await page.addInitScript(() => {
      window.sessionStorage.setItem("gwt:ui:briefing", "1");
    });
    await page.goto(BASE);

    await expect(page.locator("#op-briefing")).toBeHidden({ timeout: 1_000 });
  });
});

async function suppressFrontendReady(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const originalSend = WebSocket.prototype.send;
    WebSocket.prototype.send = function sendWithoutFrontendReady(
      data: string | ArrayBufferLike | Blob | ArrayBufferView,
    ) {
      try {
        const payload = typeof data === "string" ? JSON.parse(data) : null;
        if (payload?.kind === "frontend_ready") {
          return;
        }
      } catch {
        /* Forward malformed or binary frames unchanged. */
      }
      return originalSend.call(this, data);
    };
  });
}
