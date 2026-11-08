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
const errorsByPage = new WeakMap<Page, string[]>();

// `serial` because every test in this file talks to the same live backend.
// In parallel mode the chromium-dark and chromium-light projects race against
// each other for the modal-backdrop state and the WebSocket session, which
// flaps the click interception logic. Sequential execution keeps the spec
// stable while leaving the rest of `pnpm test:visual` parallel.
test.describe.serial("Release Notes window (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test.beforeEach(async ({ page }, info) => {
    const errors: string[] = [];
    errorsByPage.set(page, errors);
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    const theme = info.project.name.includes('light') ? 'light' : 'dark';
    await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
    await suppressInitialFrontendReady(page);
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    await injectReleaseNotesWorkspace(page, CURRENT_VERSION);
    await expect(page.locator("#op-briefing")).toBeHidden();
    await expect(page.locator("#project-picker")).toBeHidden();
  });

  test.afterEach(async ({ page }) => {
    expect(errorsByPage.get(page)).toEqual([]);
  });

  test("clicking #app-version opens the Release Notes window with the label target focused", async ({
    page,
  }, info) => {
    const label = page.locator("#app-version");
    await expect(label).toBeVisible();
    const labelText = (await label.textContent())?.trim() ?? "";
    const [currentVersion, latestVersion = ""] = labelText
      .replace(/^v/, "")
      .split(" -> ");
    const expectedVersion = latestVersion || currentVersion;
    expect(currentVersion).toMatch(/^\d+\.\d+\.\d+$/);
    expect(expectedVersion).toMatch(/^\d+\.\d+\.\d+$/);

    await label.click();

    const window = page.locator("#release-notes-window");
    await expect(window).toBeVisible({
      timeout: RELEASE_NOTES_VISIBLE_TIMEOUT_MS,
    });
    await expect(window).toHaveAttribute("role", "dialog");

    const selected = window.locator(".release-notes-sidebar-item.is-selected");
    await expect(selected).toHaveAttribute("data-version", expectedVersion);
    const currentRow = window.locator(
      `.release-notes-sidebar-item[data-version="${currentVersion}"]`,
    );
    await expect(currentRow).toHaveClass(/is-current-version/);
    await expect(currentRow.locator(".release-notes-sidebar-current")).toHaveText(
      "Current",
    );

    // Right pane should render the corresponding entry's H2 (`v<version>`).
    await expect(
      window.locator(".release-notes-content h2"),
    ).toHaveText(`v${expectedVersion}`);
    await page.screenshot({ path: info.outputPath("release-notes.png") });
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

  test("rendered notes recover from an error without merge or develop-sync noise", async ({ page }) => {
    await page.locator("#app-version").click();
    const window = page.locator("#release-notes-window");
    await expect(window).toBeVisible();
    await expect(window.locator(".release-notes-content")).not.toContainText(
      /Merge .*develop|develop sync|sync develop|develop.*(sync|同期)|Developを同期/i,
    );
    await page.evaluate(() => {
      globalThis.dispatchEvent(new CustomEvent("__gwt_test_inject", {
        detail: { kind: "release_notes_error", message: "Release notes fixture unavailable" },
      }));
    });
    await expect(window.locator(".release-notes-empty")).toContainText("Release notes fixture unavailable");
    await expect(window.locator(".release-notes-sidebar-item")).toHaveCount(0);
    await page.locator("#app-version").click();
    await expect(window.locator(".release-notes-empty")).toHaveCount(0);
    await expect(window.locator(".release-notes-sidebar-item.is-selected")).toHaveCount(1);
    await expect(window.locator(".release-notes-content h2")).toHaveText(`v${CURRENT_VERSION}`);
  });

  test("close and Escape release the window and allow the same controller to reopen", async ({
    page,
  }) => {
    await page.locator("#app-version").click();
    const window = page.locator("#release-notes-window");
    await expect(window).toBeVisible({
      timeout: RELEASE_NOTES_VISIBLE_TIMEOUT_MS,
    });
    await window.locator(".release-notes-close").click();
    await expect(window).toHaveCount(0);
    await page.locator("#app-version").click();
    await expect(window).toBeVisible();
    await window.press("Escape");
    await expect(window).toHaveCount(0);
    await expect(page.locator("#app-version")).toBeFocused();
    await page.locator("#app-version").press("Enter");
    await expect(window).toHaveCount(1);
    await expect(window.locator(".release-notes-content h2")).toHaveText(`v${CURRENT_VERSION}`);
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

  test("payload bursts share one render and closed windows stay closed with pending frames", async ({ page }, info) => {
    await page.locator("#app-version").click();
    const notes = page.locator("#release-notes-window");
    await expect(notes.locator(".release-notes-content h2")).toBeVisible();
    const burst = await page.evaluate(async () => {
      const content = document.querySelector(".release-notes-content")!;
      const previous = content.firstChild;
      const focused = document.activeElement;
      let renders = 0;
      const observer = new MutationObserver(records => {
        renders += records.flatMap(record => [...record.addedNodes])
          .filter(node => node instanceof Element && node.matches(".release-notes-version-header")).length;
      });
      observer.observe(content, { childList: true });
      try {
        for (let index = 1; index <= 50; index++) {
          window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
            kind: "release_notes_payload", current_version: "9.0.50",
            entries: [{ version: `9.0.${index}`, sections: [] }],
          } }));
        }
        const deferred = content.firstChild === previous;
        await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        const header = content.firstChild;
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
          kind: "runtime_health", snapshot: {},
        } }));
        await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        return { deferred, renders, heading: content.querySelector("h2")?.textContent,
          selected: document.querySelector(".release-notes-sidebar-item.is-selected")?.getAttribute("data-version"),
          unrelatedPreserved: content.firstChild === header,
          focusPreserved: document.activeElement === focused };
      } finally { observer.disconnect(); }
    });
    expect(burst).toEqual({ deferred: true, renders: 1, heading: "v9.0.50",
      selected: "9.0.50", unrelatedPreserved: true, focusPreserved: true });
    await info.attach("release-notes-frame-burst", { body: JSON.stringify(burst), contentType: "application/json" });
    await page.evaluate(async () => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
        kind: "release_notes_error", message: "Pending error",
      } }));
      (document.querySelector(".release-notes-close") as HTMLButtonElement).click();
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    });
    await expect(notes).toHaveCount(0);
    await expect(page.locator("#app-version")).toBeFocused();
    await page.locator("#app-version").click();
    await expect(notes.locator(".release-notes-content h2")).toHaveText(`v${CURRENT_VERSION}`);
    await page.evaluate(() => {
      (window as any).__releaseNotesRaf = window.requestAnimationFrame;
      window.requestAnimationFrame = () => 0;
      (window as any).__releaseNotesFallbackStart = performance.now();
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
        kind: "release_notes_error", message: "Fallback release notes error",
      } }));
    });
    try {
      await expect(notes.locator(".release-notes-empty")).toContainText("Fallback release notes error", { timeout: 1000 });
      const elapsed = await page.evaluate(() => performance.now() - (window as any).__releaseNotesFallbackStart);
      expect(elapsed).toBeLessThan(1000);
      await info.attach("release-notes-frame-fallback", { body: JSON.stringify({ elapsed }), contentType: "application/json" });
      await page.screenshot({ path: info.outputPath("release-notes-frame.png") });
    } finally {
      await page.evaluate(() => { window.requestAnimationFrame = (window as any).__releaseNotesRaf; });
    }
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
