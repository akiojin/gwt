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
import { test, expect, type Page } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";
const pageErrors = new WeakMap<Page, string[]>();

test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  pageErrors.set(page, errors);
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await gotoLiveGwt(page, BASE, { suppressProjectSurfaces: true, enableTestBridge: true });
});

test.describe("mobile project chrome", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "requires browser-check isolated checkout server");
  test.use({ hasTouch: true });

  test("top actions fit narrow screens with touch targets and retain desktop layout", async ({ page }, testInfo) => {
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(page.locator("#app-version")).toBeVisible();
    for (const width of [320, 390]) {
      await page.setViewportSize({ width, height: 844 });
      const controls = page.locator(".project-actions > button:visible, .project-actions > [role=button]:visible, .op-theme-toggle__option");
      await expect(controls).toHaveCount(6);
      for (const control of await controls.all()) {
        await expect(control).toBeInViewport({ ratio: 1 });
        const bounds = await control.boundingBox();
        expect(bounds!.width).toBeGreaterThanOrEqual(44);
        expect(bounds!.height).toBeGreaterThanOrEqual(44);
        expect(await control.evaluate(element => {
          const box = element.getBoundingClientRect();
          return element.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2));
        })).toBe(true);
      }
      const bar = await page.locator(".project-bar").boundingBox();
      const rail = await page.locator(".op-rail").boundingBox();
      const surface = await page.locator(".surface-area").boundingBox();
      expect(bar!.x + bar!.width).toBeLessThanOrEqual(width);
      expect(rail!.y).toBeGreaterThanOrEqual(bar!.y + bar!.height);
      expect(surface!.y).toBeGreaterThanOrEqual(rail!.y + rail!.height);
      expect(surface!.height).toBeGreaterThan(0);
      const other = theme === "dark" ? "light" : "dark";
      await page.locator(`[data-theme-value="${other}"]`).tap();
      await expect(page.locator("html")).toHaveAttribute("data-theme", other);
      await page.locator(`[data-theme-value="${theme}"]`).tap();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await page.screenshot({ path: testInfo.outputPath(`mobile-chrome-${width}-${theme}.png`) });
    }
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
      detail: { kind: "workspace_state_notice", notice: {
        kind: "load_error", path: "/fixture/workspace.json", message: "Workspace state needs attention",
      } },
    })));
    const notice = page.locator(".workspace-state-notice");
    await expect(notice).toBeVisible();
    const noticeBounds = await notice.boundingBox();
    for (const control of await page.locator(".project-actions > button, #app-version, .op-theme-toggle__option").all()) {
      const bounds = await control.boundingBox();
      expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(noticeBounds!.y);
    }
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
      detail: { kind: "workspace_state_notice", notice: null },
    })));
    await expect(notice).toHaveCount(0);
    await page.setViewportSize({ width: 1360, height: 900 });
    await expect(page.locator(".project-bar")).toHaveCSS("height", "44px");
    await expect(page.locator(".op-theme-toggle")).toHaveCSS("height", "22px");
    expect(pageErrors.get(page)).toEqual([]);
  });
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
