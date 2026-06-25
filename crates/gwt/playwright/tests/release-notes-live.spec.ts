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
import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { gotoLiveGwt, suppressInitialFrontendReady } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const RELEASE_NOTES_VISIBLE_TIMEOUT_MS = 15_000;
const PROJECT_ROOT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? process.cwd();
const CURRENT_VERSION = currentGwtVersion();

// `serial` because every test in this file talks to the same live backend.
// In parallel mode the chromium-dark and chromium-light projects race against
// each other for the modal-backdrop state and the WebSocket session, which
// flaps the click interception logic. Sequential execution keeps the spec
// stable while leaving the rest of `pnpm test:visual` parallel.
test.describe.serial("Release Notes window (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test.beforeEach(async ({ page }) => {
    await suppressInitialFrontendReady(page);
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await injectReleaseNotesWorkspace(page, CURRENT_VERSION);
    await expect(page.locator("#op-briefing")).toBeHidden();
    await expect(page.locator("#project-picker")).toBeHidden();
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
    await expect(window).toBeVisible({
      timeout: RELEASE_NOTES_VISIBLE_TIMEOUT_MS,
    });
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
    await expect(window).toBeVisible({
      timeout: RELEASE_NOTES_VISIBLE_TIMEOUT_MS,
    });

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
    await expect(window).toBeVisible({
      timeout: RELEASE_NOTES_VISIBLE_TIMEOUT_MS,
    });
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
    await expect(page.locator("#release-notes-window")).toBeVisible({
      timeout: RELEASE_NOTES_VISIBLE_TIMEOUT_MS,
    });
  });
});

function currentGwtVersion(): string {
  const cargoToml = readFileSync(join(PROJECT_ROOT, "Cargo.toml"), "utf8");
  const match = cargoToml.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) {
    throw new Error(`Could not read package version from ${PROJECT_ROOT}/Cargo.toml`);
  }
  return match[1];
}

async function injectReleaseNotesWorkspace(
  page: Page,
  currentVersion: string,
): Promise<void> {
  await page.evaluate(
    ({ projectRoot, version }) => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "workspace_state",
            workspace: {
              app_version: version,
              tabs: [
                {
                  id: "release-notes-tab",
                  title: "Release Notes Fixture",
                  project_root: projectRoot,
                  kind: "git",
                  workspace: {
                    viewport: { x: 0, y: 0, zoom: 1 },
                    windows: [],
                  },
                },
              ],
              active_tab_id: "release-notes-tab",
              recent_projects: [],
            },
          },
        }),
      );
    },
    { projectRoot: PROJECT_ROOT, version: currentVersion },
  );
}
