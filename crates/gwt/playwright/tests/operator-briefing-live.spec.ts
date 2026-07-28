/**
 * Issue #2796 — Mission Briefing must never block a live gwt client.
 *
 * The live contract is outcome-based: whether hydration, the briefing timer,
 * or prior session state wins the race, the overlay must become hidden and
 * stop intercepting pointer input.
 */
import { expect, test } from "@playwright/test";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test.describe("Mission Briefing dismissal (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test("dismisses within the live startup deadline", async ({ page }) => {
    await page.goto(BASE);

    const briefing = page.locator("#op-briefing");
    await expect(briefing).toBeHidden({ timeout: 3_000 });
  });

  test("does not intercept pointer input after hydration", async ({ page }) => {
    await page.goto(BASE);

    const briefing = page.locator("#op-briefing");
    await expect(briefing).toBeHidden({ timeout: 1_000 });
    await expect(briefing).toHaveCSS("pointer-events", "auto");
    expect(await briefing.evaluate((element) => element.hidden)).toBe(true);
  });

  test("a previously shown briefing stays hidden", async ({ page }) => {
    await page.addInitScript(() => {
      window.sessionStorage.setItem("gwt:ui:briefing", "1");
    });
    await page.goto(BASE);

    await expect(page.locator("#op-briefing")).toBeHidden({ timeout: 1_000 });
  });
});
