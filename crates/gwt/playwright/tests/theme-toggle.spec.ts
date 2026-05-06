/* SPEC-2356 — theme toggle round-trip (US-3 AS-2/AS-3/AS-6/AS-7). */
import { test, expect } from "@playwright/test";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.describe("Theme toggle", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test("segmented control commits preference and reflects via data-theme + aria-checked", async ({ page }) => {
    await page.goto(BASE);
    const html = page.locator("html");
    const auto = page.locator('#op-theme-toggle [data-theme-value="auto"]');
    const dark = page.locator('#op-theme-toggle [data-theme-value="dark"]');
    const light = page.locator('#op-theme-toggle [data-theme-value="light"]');

    // FR-024 — radiogroup container with three exclusive options.
    await expect(page.locator("#op-theme-toggle")).toHaveAttribute("role", "radiogroup");
    await expect(auto).toHaveAttribute("role", "radio");

    await dark.click();
    await expect(html).toHaveAttribute("data-theme", "dark");
    await expect(dark).toHaveAttribute("aria-checked", "true");
    await expect(auto).toHaveAttribute("aria-checked", "false");
    await expect(light).toHaveAttribute("aria-checked", "false");

    const t0 = Date.now();
    await light.click();
    await expect(html).toHaveAttribute("data-theme", "light");
    expect(Date.now() - t0).toBeLessThan(800); // generous, real budget 200ms (NFR-008)
    await expect(light).toHaveAttribute("aria-checked", "true");

    // US3-AS7 — AUTO must be reachable directly from any state.
    await auto.click();
    await expect(auto).toHaveAttribute("aria-checked", "true");
  });
});
