/* SPEC-2356 Operator Design System — Playwright chrome smoke (Phase 4/9 baseline).
 *
 * Loads the embedded gwt frontend and asserts the Operator chrome scaffold
 * renders in both Dark and Light themes. The gwt binary must be running on the
 * URL provided via GWT_PLAYWRIGHT_BASE_URL (the visual-regression CI workflow
 * boots it before invoking Playwright).
 *
 * Phase 9 (FR-021/FR-031/FR-032): chrome visibility runs through the
 * hover-reveal state machine. The sidebar auto-hides by default and is
 * summoned via the peek 帯 (`.op-sidebar-peek`). Window controls live inside
 * the sidebar; the separate window-controls peek was retired.
 */
import { test, expect } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.beforeEach(async ({ page }) => {
  await gotoLiveGwt(page, BASE);
});

test.describe("Operator chrome smoke", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test("project bar, status strip, sidebar peek 帯, and theme toggle render in auto-hide state", async ({ page }) => {
    await expect(page.locator(".project-bar")).toBeVisible();
    await expect(page.locator(".op-status-strip")).toBeVisible();
    await expect(page.locator("#op-theme-toggle")).toBeVisible();
    await expect(page.locator("#op-strip-clock")).toHaveText(/\d{2}:\d{2}:\d{2}/);
    // Sidebar exists in the DOM but is not the visible chrome until revealed.
    await expect(page.locator(".op-sidebar")).toHaveCount(1);
    await expect(page.locator(".op-sidebar-peek")).toBeVisible();
    await expect(page.locator(".op-window-controls-peek")).toHaveCount(0);
    await expect(page.locator("html")).not.toHaveAttribute("data-op-sidebar", /.+/);
    await expect(page.locator("html")).not.toHaveAttribute("data-op-window-controls", /.+/);
  });

  test("hover on sidebar peek 帯 reveals the sidebar and collapses ~250ms after pointerleave", async ({ page }) => {
    await page.locator(".op-sidebar-peek").hover();
    await expect(page.locator("html")).toHaveAttribute("data-op-sidebar", "revealed");
    await page.mouse.move(2000, 2000);
    await expect(page.locator("html")).toHaveAttribute("data-op-sidebar", "revealed");
    await page.waitForTimeout(400);
    await expect(page.locator("html")).not.toHaveAttribute("data-op-sidebar", /.+/);
  });

  test("window controls live in the sidebar and reveal with the sidebar peek 帯", async ({ page }) => {
    await expect(page.locator(".op-window-controls-peek")).toHaveCount(0);
    await expect(page.locator("html")).not.toHaveAttribute("data-op-window-controls", /.+/);
    const sidebar = page.locator("#op-sidebar");
    await expect(sidebar.locator("#tile-button")).toHaveCount(1);
    await expect(sidebar.locator("#stack-button")).toHaveCount(1);
    await expect(sidebar.locator("#align-button")).toHaveCount(1);
    await expect(sidebar.locator("#window-list-button")).toHaveCount(1);
    await expect(sidebar.locator("#add-button")).toHaveCount(1);
    await page.locator(".op-sidebar-peek").hover();
    await expect(page.locator("html")).toHaveAttribute("data-op-sidebar", "revealed");
    await expect(sidebar.locator("#tile-button")).toBeVisible();
    await expect(sidebar.locator("#add-button")).toBeVisible();
    // Palette + Zoom controls remain reachable from their dedicated chrome locations.
    await expect(sidebar.locator("#op-palette-button")).toBeVisible();
    await expect(page.locator("#zoom-reset-button")).toBeVisible();
  });

  test("keyboard focus on sidebar peek 帯 triggers reveal as a hover alternative", async ({ page }) => {
    await page.locator(".op-sidebar-peek").focus();
    await expect(page.locator("html")).toHaveAttribute("data-op-sidebar", "revealed");
  });

  test("⌘K opens the command palette", async ({ page }) => {
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
    await page.locator(".op-sidebar-peek").hover();
    await expect(page.locator("html")).toHaveAttribute("data-op-sidebar", "revealed");
    await page.mouse.move(2000, 2000);
    await page.waitForTimeout(40);
    await expect(page.locator("html")).not.toHaveAttribute("data-op-sidebar", /.+/);
  });
});
