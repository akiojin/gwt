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

  test("shared progress renders the saved snapshot on mount and subsequent updates", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("pageerror", error => errors.push(error.message));
    await gotoLiveGwt(page, BASE, {
      enableTestBridge: true,
      suppressProjectSurfaces: true,
      suppressUpdateApplyStart: true,
    });
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const inject = (detail: Record<string, unknown>) => page.evaluate(payload => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
    }, detail);
    await inject({ kind: "update_state", state: "available", current: "9.25.0", latest: "9.26.0" });
    await inject({ kind: "update_progress", downloaded: 1024 * 1024, total: 4 * 1024 * 1024 });
    const cta = page.locator("#update-cta");
    const modal = page.locator("#update-modal");
    const progress = page.locator("[data-update-modal-progress]");
    const counter = page.locator("[data-update-modal-byte-counter]");
    await expect(modal).toHaveCount(0);
    await cta.click();
    await expect(progress).toHaveAttribute("aria-valuenow", "25");
    await expect(counter).toHaveText("1.0 MB / 4.0 MB");
    await inject({ kind: "update_progress", downloaded: 3 * 1024 * 1024, total: 4 * 1024 * 1024 });
    await expect(progress).toHaveAttribute("aria-valuenow", "75");
    await expect(counter).toHaveText("3.0 MB / 4.0 MB");
    // Closing the failure panel preserves the latest progress for a remount.
    await inject({ kind: "update_apply_error", reason: "Fixture failure" });
    await page.locator("[data-update-modal-close]").click();
    await expect(modal).toHaveCount(0);
    await cta.click();
    await expect(progress).toHaveAttribute("aria-valuenow", "75");
    await expect(counter).toHaveText("3.0 MB / 4.0 MB");
    await inject({ kind: "update_progress", downloaded: 2 * 1024 * 1024, total: 4 * 1024 * 1024 });
    await expect(progress).toHaveAttribute("aria-valuenow", "50");
    await expect(counter).toHaveText("2.0 MB / 4.0 MB");
    await expect.poll(() => progress.evaluate(node => Math.round(
      node.querySelector(".update-modal__progress-fill")!.getBoundingClientRect().width
      / node.getBoundingClientRect().width * 100,
    ))).toBe(50);
    await page.screenshot({ path: testInfo.outputPath(`update-progress-${theme}.png`) });
    await page.locator("[data-update-modal-cancel]").click();
    await expect(modal).toHaveCount(0);
    await cta.click();
    await expect(progress).toHaveAttribute("aria-valuenow", "0");
    await expect(counter).toHaveText("0.0 MB / 0.0 MB");
    expect(errors).toEqual([]);
  });

  test("a newer release replaces the ready prompt without reopening a deferred same-version modal", async ({ page }) => {
    const errors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    page.on("pageerror", (error) => errors.push(error.message));
    // The two releases are injected fixtures, so Later must not try to persist
    // a nonexistent manifest on the isolated backend.
    await page.addInitScript(() => {
      const originalSend = WebSocket.prototype.send;
      WebSocket.prototype.send = function (data) {
        if (typeof data === "string" && JSON.parse(data).kind === "apply_update_later") return;
        return originalSend.call(this, data);
      };
    });
    await gotoLiveGwt(page, BASE, {
      enableTestBridge: true,
      suppressProjectSurfaces: true,
      suppressUpdateApplyStart: true,
    });
    const inject = (detail: Record<string, unknown>) =>
      page.evaluate((payload) => {
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
      }, detail);
    const available = { kind: "update_state", state: "available", current: "9.25.0" };
    const cta = page.locator("#update-cta");
    const modal = page.locator("#update-modal");

    await inject({ ...available, latest: "9.26.0" });
    await cta.click();
    await inject({ kind: "update_ready", version: "9.26.0", asset_path: "/tmp/fake-9.26.0" });
    await expect(modal).toHaveAttribute("data-version", "9.26.0");
    await page.locator("[data-update-modal-later]").click();
    await inject({ ...available, latest: "9.26.0" });
    await expect(modal).toHaveCount(0);
    await expect(cta).toHaveAttribute("data-status", "ready");

    // Reopen the staged release so supersession must remove its Restart action.
    await cta.click();
    await expect(modal).toHaveAttribute("data-state", "ready");
    await inject({ ...available, latest: "9.27.0" });
    await expect(modal).toHaveCount(0);
    await expect(cta).toHaveText("Update available: v9.27.0 - Click to update");
    await cta.click();
    await expect(modal).toHaveAttribute("data-state", "downloading");
    await expect(modal).toHaveAttribute("data-version", "9.27.0");
    await expect(page.locator("[data-update-modal-restart-now]")).toHaveCount(0);
    expect(errors).toEqual([]);
  });

  test("CTA -> downloading -> ready -> Later morphs CTA to ready", async ({ page }) => {
    await gotoLiveGwt(page, BASE, {
      enableTestBridge: true,
      suppressProjectSurfaces: true,
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

  test("update_apply_error renders failed state with stage / reason / log", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("pageerror", error => errors.push(error.message));
    await gotoLiveGwt(page, BASE, {
      enableTestBridge: true,
      suppressProjectSurfaces: true,
      suppressUpdateApplyStart: true,
    });
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

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
    await expect(page.locator("#update-cta")).toHaveAttribute("data-status", "error");
    await expect(page.locator("#update-cta")).toContainText("Update failed: HTTP 503");
    await expect(modal.getByRole("dialog")).toHaveAttribute("aria-modal", "true");
    await expect(page.locator("[data-update-modal-open-log]")).toBeFocused();
    await page.screenshot({ path: testInfo.outputPath(`update-error-${theme}.png`) });
    // The injected failure has no real download or log to act on. Retry is
    // suppressed by the bridge; Open log is covered by the controller tests.
    await page.locator("[data-update-modal-retry]").click();
    await expect(modal).toHaveAttribute("data-state", "downloading");
    await expect(page.locator("#update-cta")).toHaveAttribute("data-status", "applying");
    await expect(page.locator("[data-update-modal-progress]")).toHaveAttribute("aria-valuenow", "0");
    await page.evaluate(() => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
        detail: { kind: "update_apply_error", message: "Legacy fixture failure" },
      }));
    });
    await expect(modal).toHaveAttribute("data-state", "failed");
    await expect(page.locator("[data-update-modal-reason]")).toHaveText("Legacy fixture failure");
    await expect(page.locator("#update-cta")).toContainText("Update failed: Legacy fixture failure");
    await page.locator("[data-update-modal-close]").click();
    await expect(modal).toHaveCount(0);
    await expect(page.locator("#update-cta")).toHaveAttribute("data-status", "available");
    expect(errors).toEqual([]);
  });
});
