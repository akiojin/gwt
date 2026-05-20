/**
 * SPEC #2780 — Release Notes window live E2E.
 *
 * Unlike the other release-notes related tests (the gwt-core parser unit
 * tests and the linkedom DOM tests in `crates/gwt/web/__tests__/`), this
 * spec runs against a **live** gwt backend over a real WebSocket. It does
 * NOT use `installEmbeddedRoutes`, because the whole point is to exercise
 * the `open_release_notes` -> `release_notes_payload` round-trip that the
 * embedded-route tests skip.
 *
 * Required environment:
 *   GWT_PLAYWRIGHT_BASE_URL=http://127.0.0.1:<port>   # a running gwt server
 *
 * The spec auto-skips when the env var is missing so CI runs without
 * a live backend stay green; the live coverage runs explicitly via
 * the gwt-verify --mode pre-pr flow once a real server is wired up.
 */
import { test, expect } from "@playwright/test";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

// `serial` because every test in this file talks to the same live backend.
// In parallel mode the chromium-dark and chromium-light projects race against
// each other for the modal-backdrop state and the WebSocket session, which
// flaps the click interception logic. Sequential execution keeps the spec
// stable while leaving the rest of `pnpm test:visual` parallel.
test.describe.serial("Release Notes window (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test.beforeEach(async ({ page }) => {
    // The Operator splash should auto-dismiss after ~1.45s and persists a
    // sessionStorage flag so subsequent loads skip it entirely. We pre-set
    // the flag here so the splash is skipped from the first load.
    //
    // KNOWN ISSUE (filed separately): in live mode the splash currently
    // refuses to dismiss even when the sessionStorage flag is set. While
    // that is open, force-hide the overlay client-side so the rest of the
    // E2E can still exercise the Release Notes flow. Remove the force-hide
    // once the splash dismiss bug is closed.
    await page.addInitScript(() => {
      try {
        window.sessionStorage.setItem("gwt:ui:briefing", "1");
      } catch {
        /* no-op */
      }
    });
    await page.goto(BASE);
    // KNOWN ISSUE workarounds (track in follow-up Issue):
    //  1. Splash overlay refuses to dismiss in live mode even with the
    //     sessionStorage flag set; force-hide it.
    //  2. Workspace state can carry over modal backdrops (file-tree picker,
    //     etc.) that intercept pointer events. Inject a permanent style rule
    //     that disables `pointer-events` on every modal-backdrop so any
    //     late-arriving modal cannot steal the click on `#app-version`.
    await page.addStyleTag({
      content: `.modal-backdrop, #op-briefing { display: none !important; pointer-events: none !important; }`,
    });
    await page.evaluate(() => {
      const overlay = document.getElementById("op-briefing");
      if (overlay) overlay.hidden = true;
    });
    await expect(page.locator("#op-briefing")).toBeHidden();
  });

  test("clicking #app-version opens the Release Notes window with current version focused", async ({
    page,
  }) => {
    const label = page.locator("#app-version");
    await expect(label).toBeVisible();
    const labelText = (await label.textContent())?.trim() ?? "";
    const currentVersion = labelText.replace(/^v/, "").split(" -> ")[0];
    expect(currentVersion).toMatch(/^\d+\.\d+\.\d+$/);

    await label.click();

    const window = page.locator("#release-notes-window");
    await expect(window).toBeVisible();
    await expect(window).toHaveAttribute("role", "dialog");

    const selected = window.locator(".release-notes-sidebar-item.is-selected");
    await expect(selected).toHaveAttribute("data-version", currentVersion);

    // Right pane should render the corresponding entry's H2 (`v<version>`).
    await expect(
      window.locator(".release-notes-content h2"),
    ).toHaveText(`v${currentVersion}`);
  });

  test("selecting a different version in the sidebar updates the content pane", async ({
    page,
  }) => {
    await page.locator("#app-version").click();
    const window = page.locator("#release-notes-window");
    await expect(window).toBeVisible();

    const items = window.locator(".release-notes-sidebar-item");
    // Pick the second entry so we differ from the default selection.
    await items.nth(1).click();

    const target = await items.nth(1).getAttribute("data-version");
    expect(target).toBeTruthy();
    await expect(
      window.locator(".release-notes-sidebar-item.is-selected"),
    ).toHaveAttribute("data-version", target as string);
    await expect(
      window.locator(".release-notes-content h2"),
    ).toHaveText(`v${target}`);
  });

  test("close button removes the window and isOpen reflects that", async ({
    page,
  }) => {
    await page.locator("#app-version").click();
    const window = page.locator("#release-notes-window");
    await expect(window).toBeVisible();
    await window.locator(".release-notes-close").click();
    await expect(window).toHaveCount(0);
  });

  test("keyboard activation (Enter on #app-version) opens the window", async ({
    page,
  }) => {
    const label = page.locator("#app-version");
    await expect(label).toBeVisible();
    // Use locator.press so Playwright handles focus + key dispatch atomically;
    // separate focus()+keyboard.press() races against any element that competes
    // for focus during workspace boot.
    await label.press("Enter");
    await expect(page.locator("#release-notes-window")).toBeVisible();
  });
});
