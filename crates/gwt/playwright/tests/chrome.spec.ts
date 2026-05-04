/* SPEC-2356 Operator Design System — Playwright chrome smoke (Phase 4 baseline).
 *
 * Loads the embedded gwt frontend and asserts the Operator chrome scaffold
 * renders in both Dark and Light themes. The gwt binary must be running on the
 * URL provided via GWT_PLAYWRIGHT_BASE_URL (the visual-regression CI workflow
 * boots it before invoking Playwright).
 */
import { test, expect } from "@playwright/test";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.describe("Operator chrome smoke", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test("project bar, status strip, sidebar, and theme toggle render", async ({ page }) => {
    await page.goto(BASE);
    await expect(page.locator(".project-bar")).toBeVisible();
    await expect(page.locator(".op-sidebar")).toBeVisible();
    await expect(page.locator(".op-status-strip")).toBeVisible();
    await expect(page.locator("#op-theme-toggle")).toBeVisible();
    await expect(page.locator("#op-strip-clock")).toHaveText(/\d{2}:\d{2}:\d{2}/);
  });

  test("⌘K opens the command palette", async ({ page }) => {
    await page.goto(BASE);
    await page.keyboard.press("Meta+K");
    await expect(page.locator("#op-palette-backdrop")).toHaveAttribute("data-open", "true");
    await page.keyboard.press("Escape");
    await expect(page.locator("#op-palette-backdrop")).not.toHaveAttribute("data-open", "true");
  });
});
