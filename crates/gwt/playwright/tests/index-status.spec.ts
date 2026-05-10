/* SPEC-1939 Phase 12 / T-IDX-109 + T-IDX-110 — Playwright e2e for the
 * Project Index status badge.
 *
 * CI orchestration:
 *   1. Boot gwt (release build) under xvfb with the env vars
 *      `GWT_INDEX_TEST_FIXTURE=<fixture-json>` (aggregator seam) and
 *      `GWT_BROWSER_URL_FILE=<tmp-file>` (URL handoff seam).
 *   2. Wait for `<tmp-file>` to materialise; read its content.
 *   3. Export it as `GWT_PLAYWRIGHT_BASE_URL=<url>` and run
 *      `npm run test:visual`.
 *   4. Tear gwt down after the suite completes.
 *
 * Both seams ship with `gwt` (`crates/gwt/src/main.rs` writes the URL,
 * `crates/gwt/src/index_worker.rs` reads the fixture) so this spec only
 * needs the env vars to be set externally.
 *
 * The CI fixtures cover the two scenarios:
 *
 *   crates/gwt/playwright/fixtures/index-status-repair-required.json
 *     — initial `repair_required` so we can observe the auto-rebuild
 *       transition badge UX (badge label, click → settings:open).
 *
 *   crates/gwt/playwright/fixtures/index-status-error.json
 *     — terminal `error` so we can observe the manual-retry UX.
 *
 * Both specs skip when `GWT_PLAYWRIGHT_BASE_URL` is unset (matches existing
 * specs); CI sets the URL once gwt is up.
 */
import { test, expect } from "@playwright/test";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.describe("Project Index status badge (SPEC-1939 Phase 12)", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test("renders a clickable button with the formatted label (T-IDX-109 happy path)", async ({
    page,
  }) => {
    await page.goto(BASE);

    // The badge starts hidden and is revealed after the bootstrap reports a
    // visible state. With GWT_INDEX_TEST_FIXTURE set, the backend emits a
    // `repair_required` view immediately so the badge becomes visible
    // without waiting on the real Python runner.
    const badge = page.locator("#index-status");
    // CI cold-start (xvfb + gwt build + bootstrap + WebSocket round-trip)
    // can take longer than 10s on shared runners. 30s gives the bootstrap
    // path plenty of headroom while still failing fast on real regressions.
    await expect(badge).toBeVisible({ timeout: 30_000 });
    await expect(badge).toHaveAttribute("type", "button");
    await expect(badge).toHaveAttribute("aria-label", /index/i);
    await expect(badge).toContainText(/Index:\s+(repair|repairing|ready)/);
  });

  test("dispatches settings:open on click (T-IDX-109 click path)", async ({ page }) => {
    await page.goto(BASE);

    // Wait for the badge to be visible before clicking — without this the
    // click can fire on a hidden element and `settings:open` never gets
    // dispatched, masking real regressions as inconsistent test failures.
    await expect(page.locator("#index-status")).toBeVisible({ timeout: 30_000 });

    // Wire a temporary listener so the test can observe the dispatched
    // CustomEvent without depending on the Settings.Index window opening
    // (which requires backend create_window plumbing).
    const dispatched = await page.evaluate(async () => {
      return await new Promise<{ target: string } | null>((resolve) => {
        const handler = (event: Event) => {
          const detail = (event as CustomEvent).detail || {};
          document.removeEventListener("settings:open", handler);
          resolve({ target: detail.target ?? "" });
        };
        document.addEventListener("settings:open", handler, { once: true });
        const badge = document.getElementById("index-status");
        if (!badge) {
          resolve(null);
          return;
        }
        badge.click();
        // Fail-fast if no event in 2s.
        setTimeout(() => resolve(null), 2_000);
      });
    });
    expect(dispatched).toEqual({ target: "index" });
  });

  test("error state surfaces a red badge and the click still routes to settings (T-IDX-110)", async ({
    page,
  }) => {
    test.skip(
      process.env.GWT_INDEX_FIXTURE_KIND !== "error",
      "this case requires the error fixture (GWT_INDEX_FIXTURE_KIND=error)",
    );

    await page.goto(BASE);
    const badge = page.locator("#index-status");
    // CI cold-start (xvfb + gwt build + bootstrap + WebSocket round-trip)
    // can take longer than 10s on shared runners. 30s gives the bootstrap
    // path plenty of headroom while still failing fast on real regressions.
    await expect(badge).toBeVisible({ timeout: 30_000 });
    await expect(badge).toHaveClass(/error/);
    await expect(badge).toContainText(/Index:\s+error/);

    const dispatched = await page.evaluate(async () => {
      return await new Promise<{ target: string } | null>((resolve) => {
        const handler = (event: Event) => {
          const detail = (event as CustomEvent).detail || {};
          document.removeEventListener("settings:open", handler);
          resolve({ target: detail.target ?? "" });
        };
        document.addEventListener("settings:open", handler, { once: true });
        const badge = document.getElementById("index-status");
        if (!badge) {
          resolve(null);
          return;
        }
        badge.click();
        setTimeout(() => resolve(null), 2_000);
      });
    });
    expect(dispatched).toEqual({ target: "index" });
  });
});
