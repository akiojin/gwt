/* SPEC-2356 Operator Design System — Playwright chrome smoke (Phase 4/9 baseline).
 *
 * Loads the embedded gwt frontend and asserts the Operator chrome scaffold
 * renders in both Dark and Light themes. The gwt binary must be running on the
 * URL provided via GWT_PLAYWRIGHT_BASE_URL (the visual-regression CI workflow
 * boots it before invoking Playwright).
 *
 * Phase 9 (FR-021/FR-022/FR-031/FR-032): chrome visibility runs through the
 * hover-reveal state machine. Sidebar and Window controls auto-hide by default
 * and are summoned via the peek 帯 (`.op-sidebar-peek` / `.op-window-controls-peek`).
 */
import { test, expect } from "@playwright/test";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.describe("Operator chrome smoke", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test("project bar, status strip, peek 帯, and theme toggle render in auto-hide state", async ({ page }) => {
    await page.goto(BASE);
    await expect(page.locator(".project-bar")).toBeVisible();
    await expect(page.locator(".op-status-strip")).toBeVisible();
    await expect(page.locator("#op-theme-toggle")).toBeVisible();
    await expect(page.locator("#op-strip-clock")).toHaveText(/\d{2}:\d{2}:\d{2}/);
    // Sidebar exists in the DOM but is not the visible chrome until revealed.
    await expect(page.locator(".op-sidebar")).toHaveCount(1);
    await expect(page.locator(".op-sidebar-peek")).toBeVisible();
    await expect(page.locator(".op-window-controls-peek")).toBeVisible();
    await expect(page.locator("html")).not.toHaveAttribute("data-op-sidebar", /.+/);
    await expect(page.locator("html")).not.toHaveAttribute("data-op-window-controls", /.+/);
  });

  test("hover on sidebar peek 帯 reveals the sidebar and collapses ~250ms after pointerleave", async ({ page }) => {
    await page.goto(BASE);
    await page.locator(".op-sidebar-peek").hover();
    await expect(page.locator("html")).toHaveAttribute("data-op-sidebar", "revealed");
    await page.mouse.move(2000, 2000);
    await expect(page.locator("html")).toHaveAttribute("data-op-sidebar", "revealed");
    await page.waitForTimeout(400);
    await expect(page.locator("html")).not.toHaveAttribute("data-op-sidebar", /.+/);
  });

  test("hover on window controls peek 帯 reveals only the collapsible groups", async ({ page }) => {
    await page.goto(BASE);
    await expect(page.locator("#tile-button")).toBeHidden();
    await page.locator(".op-window-controls-peek").hover();
    await expect(page.locator("html")).toHaveAttribute("data-op-window-controls", "revealed");
    await expect(page.locator("#tile-button")).toBeVisible();
    await expect(page.locator("#add-button")).toBeVisible();
    // Palette + Zoom controls remain in the toolbar regardless of reveal state.
    await expect(page.locator("#op-palette-button")).toBeVisible();
    await expect(page.locator("#zoom-reset-button")).toBeVisible();
  });

  test("keyboard focus on sidebar peek 帯 triggers reveal as a hover alternative", async ({ page }) => {
    await page.goto(BASE);
    await page.locator(".op-sidebar-peek").focus();
    await expect(page.locator("html")).toHaveAttribute("data-op-sidebar", "revealed");
  });

  test("⌘K opens the command palette", async ({ page }) => {
    await page.goto(BASE);
    await page.keyboard.press("Meta+K");
    await expect(page.locator("#op-palette-backdrop")).toHaveAttribute("data-open", "true");
    await page.keyboard.press("Escape");
    await expect(page.locator("#op-palette-backdrop")).not.toHaveAttribute("data-open", "true");
  });
});

test.describe("Operator chrome reduced-motion", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");
  test.use({ reducedMotion: "reduce" });

  test("prefers-reduced-motion collapses the panel without the close delay", async ({ page }) => {
    await page.goto(BASE);
    await page.locator(".op-sidebar-peek").hover();
    await expect(page.locator("html")).toHaveAttribute("data-op-sidebar", "revealed");
    await page.mouse.move(2000, 2000);
    await page.waitForTimeout(40);
    await expect(page.locator("html")).not.toHaveAttribute("data-op-sidebar", /.+/);
  });
});
