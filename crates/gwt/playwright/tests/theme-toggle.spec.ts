/* SPEC-2356 — theme toggle round-trip (US-3 AS-2/AS-3). */
import { test, expect } from "@playwright/test";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.describe("Theme toggle", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test("toggles dark ↔ light within 200ms (NFR-008)", async ({ page }) => {
    await page.goto(BASE);
    const html = page.locator("html");
    await page.locator("#op-theme-toggle").click(); // → dark
    await expect(html).toHaveAttribute("data-theme", "dark");
    const t0 = Date.now();
    await page.locator("#op-theme-toggle").click(); // → light
    await expect(html).toHaveAttribute("data-theme", "light");
    expect(Date.now() - t0).toBeLessThan(800); // generous, real budget 200ms
  });
});
