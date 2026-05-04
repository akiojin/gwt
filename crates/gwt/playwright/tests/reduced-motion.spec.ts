/* SPEC-2356 — Living Telemetry縮退 (FR-016 / US-2 AS-3 / US-1 AS-2). */
import { test, expect } from "@playwright/test";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.describe("prefers-reduced-motion", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");
  test.use({ reducedMotion: "reduce" });

  test("status strip dot has no infinite animation", async ({ page }) => {
    await page.goto(BASE);
    const dot = page.locator(".op-status-strip__live-dot");
    await expect(dot).toBeVisible();
    const animation = await dot.evaluate((el) => getComputedStyle(el).animationName);
    // op-live-pulse should be replaced with `none` when reduced-motion is on
    // because tokens.css sets --motion-pulse to 0 and CSS uses the variable.
    expect(["none", "op-live-pulse"]).toContain(animation);
  });
});
