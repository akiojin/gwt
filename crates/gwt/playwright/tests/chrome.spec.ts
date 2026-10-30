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
  await gotoLiveGwt(page, BASE, { suppressProjectSurfaces: true });
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
      '[data-surface="issues"]',
      '[data-surface="works"]',
      '[data-cmd="open-logs"]',
      '[data-cmd="open-index"]',
      '[data-cmd="open-profile"]',
      "#op-palette-button",
    ]) {
      await expect(page.locator(id)).toBeVisible();
    }
  });

  test("hovering and focusing a rail item exposes its explanation outside the scroll area", async ({ page }) => {
    const control = page.locator("#op-palette-button");
    const explanation = page.locator("#rail-control-explanation");
    await control.hover();
    await expect(explanation).toBeVisible();
    await expect(explanation).toContainText("Command palette");
    await control.focus();
    await expect(explanation).toBeVisible();
  });

  test("⌘K opens the command palette", async ({ page }) => {
    // Headed Chromium reserves the physical Meta+K chord for browser chrome
    // before the page sees it. Dispatch the same trusted-shape DOM event so
    // this browser E2E exercises gwt's global hotkey registry deterministically.
    await page.locator("body").dispatchEvent("keydown", {
      key: "k",
      code: "KeyK",
      metaKey: true,
      bubbles: true,
    });
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
    await expect(page.locator('[data-surface="works"]')).toBeVisible();
  });
});
