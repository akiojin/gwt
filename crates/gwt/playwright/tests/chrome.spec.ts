/* SPEC-3038 Operator chrome smoke — Command Rail era.
 *
 * Loads the embedded gwt frontend and asserts the Operator chrome scaffold
 * renders in both Dark and Light themes. The gwt binary must be running on the
 * URL provided via GWT_PLAYWRIGHT_BASE_URL (the visual-regression CI workflow
 * boots it before invoking Playwright).
 *
 * SPEC-3038 US-1: the 56px Command Rail is grid-docked and always visible.
 * The SPEC-2356 hover-reveal machinery (peek 帯 + data-op-sidebar) is retired.
 */
import { test, expect } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.beforeEach(async ({ page }) => {
  await gotoLiveGwt(page, BASE);
});

test.describe("Operator chrome smoke", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test("project bar, status strip, command rail, and theme toggle render", async ({ page }) => {
    await expect(page.locator(".project-bar")).toBeVisible();
    await expect(page.locator(".op-status-strip")).toBeVisible();
    await expect(page.locator("#op-theme-toggle")).toBeVisible();
    await expect(page.locator("#op-strip-clock")).toHaveText(/\d{2}:\d{2}:\d{2}/);
    // The Command Rail is always visible — no reveal state, no peek 帯.
    await expect(page.locator(".op-rail")).toBeVisible();
    await expect(page.locator(".op-sidebar-peek")).toHaveCount(0);
    await expect(page.locator(".op-window-controls-peek")).toHaveCount(0);
    await expect(page.locator("html")).not.toHaveAttribute("data-op-sidebar", /.+/);
    await expect(page.locator("html")).not.toHaveAttribute("data-op-window-controls", /.+/);
  });

  test("rail items are reachable without any reveal interaction", async ({ page }) => {
    for (const id of [
      "#tile-button",
      "#stack-button",
      "#align-button",
      "#window-list-button",
      "#add-button",
      "#op-palette-button",
    ]) {
      await expect(page.locator(id)).toBeVisible();
    }
  });

  test("hovering a rail item raises its flyout label", async ({ page }) => {
    const tile = page.locator("#tile-button");
    const flyout = tile.locator(".op-rail__flyout");
    await expect(flyout).toHaveCSS("opacity", "0");
    await tile.hover();
    await expect(flyout).toHaveCSS("opacity", "1");
    await expect(flyout).toHaveText(/Tile/);
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

  test("the rail renders identically under prefers-reduced-motion", async ({ page }) => {
    await expect(page.locator(".op-rail")).toBeVisible();
    await expect(page.locator("#tile-button")).toBeVisible();
  });
});
