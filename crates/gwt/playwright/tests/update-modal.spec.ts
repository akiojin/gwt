/* SPEC-2041 Phase 19 — post-click update modal flow (T-121).
 *
 * Driven by injecting `update_state` / `update_progress` / `update_ready` /
 * `update_apply_error` payloads through the WebSocket message handler the
 * page exposes on `window.__gwtTestApi`. The test does not require a real
 * GitHub release to be reachable; it only exercises the rendering pipeline
 * inside the WebView. Skipped when `GWT_PLAYWRIGHT_BASE_URL` is unset so it
 * matches the existing Playwright suite's skip-by-default contract.
 */
import { test, expect } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.describe("Update modal", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test("CTA -> downloading -> ready -> Later morphs CTA to ready", async ({ page }) => {
    await gotoLiveGwt(page, BASE, {
      enableTestBridge: true,
      suppressUpdateApplyStart: true,
    });

    // Simulate `update_state available` arriving from backend.
    await page.evaluate(() => {
      const ev = new CustomEvent("__gwt_test_inject", {
        detail: {
          kind: "update_state",
          state: "available",
          current: "9.25.0",
          latest: "9.26.0",
        },
      });
      window.dispatchEvent(ev);
    });

    const cta = page.locator("#update-cta");
    await expect(cta).toBeVisible();
    await expect(cta).toHaveText(/Update available: v9\.26\.0 - Click to update/);

    await cta.click();
    const modal = page.locator("#update-modal");
    await expect(modal).toBeVisible();
    await expect(modal).toHaveAttribute("data-state", "downloading");
    await expect(page.locator("[data-update-modal-cancel]")).toBeVisible();

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "update_progress",
            downloaded: 1024 * 1024,
            total: 4 * 1024 * 1024,
            asset: "gwt-macos-arm64.tar.gz",
            version: "9.26.0",
          },
        }),
      );
    });
    const progress = page.locator("[data-update-modal-progress]");
    await expect(progress).toHaveAttribute("aria-valuenow", "25");

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "update_ready",
            version: "9.26.0",
            asset_path: "/tmp/pending-update/9.26.0/gwt",
          },
        }),
      );
    });
    await expect(modal).toHaveAttribute("data-state", "ready");
    await page.locator("[data-update-modal-later]").click();
    await expect(modal).toHaveCount(0);
    await expect(cta).toHaveText(/Update v9\.26\.0 ready.*Restart now/);
  });

  test("update_apply_error renders failed state with stage / reason / log", async ({ page }) => {
    await gotoLiveGwt(page, BASE, {
      enableTestBridge: true,
      suppressUpdateApplyStart: true,
    });

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "update_state",
            state: "available",
            current: "9.25.0",
            latest: "9.26.0",
          },
        }),
      );
    });

    await page.locator("#update-cta").click();
    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "update_apply_error",
            stage: "Download asset",
            reason: "HTTP 503",
            log_path: "/Users/x/.gwt/logs/update-2026-05-11.log",
          },
        }),
      );
    });

    const modal = page.locator("#update-modal");
    await expect(modal).toHaveAttribute("data-state", "failed");
    await expect(page.locator("[data-update-modal-stage]")).toContainText("Download asset");
    await expect(page.locator("[data-update-modal-reason]")).toContainText("HTTP 503");
    await expect(page.locator("[data-update-modal-log]")).toContainText("update-2026-05-11");
    await expect(page.locator("[data-update-modal-open-log]")).toBeVisible();
    await expect(page.locator("[data-update-modal-retry]")).toBeVisible();
    await expect(page.locator("[data-update-modal-close]")).toBeVisible();
  });
});
