/* SPEC-1939 Phase 12 / T-IDX-109 + T-IDX-110 — Project Index status badge.
 *
 * Reuses the SPEC-2017 Kanban fixture pattern (`tests/kanban.spec.ts`):
 * the fixture serves embedded frontend assets through Playwright routes
 * and stubs WebSocket with a deterministic backend that emits canned
 * `workspace_state` + `project_index_status` events. No xvfb / wry / live
 * gwt process required, so the suite stays reliable in headless CI.
 */
import { expect, test } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";

const APP_URL = "http://gwt-playwright.local/";
const WEB_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../web",
);

const ROOT_MODULES = new Set([
  "app.js",
  "board-surface.js",
  "branch-cleanup-modal.js",
  "focus-trap.js",
  "hotkey.js",
  "index-settings-panel.js",
  "index-status-controller.js",
  "migration-modal.js",
  "operator-shell.js",
  "terminal-context-menu.js",
  "terminal-viewport-reflow.js",
  "theme-manager.js",
  "theme-toggle.js",
  "update-cta.js",
  "window-docking.js",
  "workspace-kanban-surface.js",
]);

test.describe("Project Index status badge", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("repair_required surfaces the red badge as a clickable button", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, { state: "repair_required" });

    await page.goto(APP_URL);

    const badge = page.locator("#index-status");
    await expect(badge).toBeVisible({ timeout: 10_000 });
    await expect(badge).toHaveAttribute("type", "button");
    await expect(badge).toHaveAttribute("aria-label", /index/i);
    await expect(badge).toContainText(/Index:\s+repair$/);
    await expect(badge).toHaveClass(/repair_required/);
  });

  test("repairing surfaces the yellow badge with a spinner glyph", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, { state: "repairing" });

    await page.goto(APP_URL);

    const badge = page.locator("#index-status");
    await expect(badge).toBeVisible({ timeout: 10_000 });
    await expect(badge).toContainText(/Index:\s+repairing/);
    await expect(badge).toHaveClass(/repairing/);
  });

  test("ready surfaces the green badge with the steady-state label", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, { state: "ready" });

    await page.goto(APP_URL);

    const badge = page.locator("#index-status");
    await expect(badge).toBeVisible({ timeout: 10_000 });
    await expect(badge).toContainText(/Index:\s+ready/);
    await expect(badge).toHaveClass(/ready/);
  });

  test("skipped keeps the badge hidden so non-git projects do not flash chrome", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, { state: "skipped" });

    await page.goto(APP_URL);

    // Wait for the workspace state to settle (project tab attached, status
    // dispatched) before asserting the badge is hidden — otherwise the
    // assertion can race with the initial render.
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator("#index-status")).toBeHidden();
  });

  test("error surfaces the red badge with the failure title", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, { state: "error" });

    await page.goto(APP_URL);

    const badge = page.locator("#index-status");
    await expect(badge).toBeVisible({ timeout: 10_000 });
    await expect(badge).toContainText(/Index:\s+error/);
    await expect(badge).toHaveClass(/error/);
    await expect(badge).toHaveAttribute("title", /failed/i);
  });

  test("badge click dispatches settings:open with target=index (T-IDX-105)", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, { state: "repair_required" });

    await page.goto(APP_URL);
    await expect(page.locator("#index-status")).toBeVisible({ timeout: 10_000 });

    const dispatched = await page.evaluate(async () => {
      return await new Promise((resolve) => {
        const handler = (event) => {
          const detail = event.detail || {};
          document.removeEventListener("settings:open", handler);
          resolve({ target: detail.target ?? "" });
        };
        document.addEventListener("settings:open", handler, { once: true });
        const badge = document.getElementById("index-status");
        if (!badge) {
          resolve(null);
          return;
        }
        badge.click();
        setTimeout(() => resolve(null), 2_000);
      });
    });
    expect(dispatched).toEqual({ target: "index" });
  });

  test("badge transitions repair_required -> repairing -> ready over WebSocket events (T-IDX-109)", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    // Initial state: repair_required.
    await installIndexStatusBackend(page, { state: "repair_required" });

    await page.goto(APP_URL);

    const badge = page.locator("#index-status");
    await expect(badge).toHaveClass(/repair_required/, { timeout: 10_000 });
    await expect(badge).toContainText(/Index:\s+repair$/);

    // Drive a repairing(1/2) update through the fixture WebSocket. The
    // fixture exposes itself on window.__gwtFixtureWebSocket so tests can
    // simulate orchestrator state-machine progress without a real backend.
    await page.evaluate(() => {
      const ws = window.__gwtFixtureWebSocket;
      ws.emit({
        kind: "project_index_status",
        project_root: "/fixture",
        status: {
          state: "repairing",
          detail: "",
          progress: { scopes_done: 1, scopes_total: 2 },
          scopes: {},
          worktrees: {},
        },
      });
    });
    await expect(badge).toHaveClass(/repairing/, { timeout: 5_000 });
    await expect(badge).toContainText(/Index:\s+repairing/);

    // Final transition to ready.
    await page.evaluate(() => {
      const ws = window.__gwtFixtureWebSocket;
      ws.emit({
        kind: "project_index_status",
        project_root: "/fixture",
        status: {
          state: "ready",
          detail: "",
          progress: null,
          scopes: {},
          worktrees: {},
        },
      });
    });
    await expect(badge).toHaveClass(/ready/, { timeout: 5_000 });
    await expect(badge).toContainText(/Index:\s+ready/);
    await expect(badge).not.toHaveClass(/repairing/);
  });

  test("project tab dot reflects aggregated worktree health (T-IDX-107)", async ({ page }) => {
    await installEmbeddedRoutes(page);

    // Initial state: one healthy file scope in wtA — dot should be green.
    await installIndexStatusBackend(page, {
      state: "ready",
      scopes: {
        files: {
          wtAhash: { healthy: true, repair_required: false, document_count: 310 },
        },
        "files-docs": {
          wtAhash: { healthy: true, repair_required: false, document_count: 16 },
        },
      },
      worktrees: {
        wtAhash: { branch: "develop", path: "/abs/wtA" },
      },
    });

    await page.goto(APP_URL);
    const dot = page.locator(".project-tab .project-tab-dot");
    await expect(dot).toHaveAttribute("data-state", "ready", { timeout: 10_000 });

    // Drive an unhealthy `files` cell on the same worktree → dot should
    // flip to `error` (red).
    await page.evaluate(() => {
      window.__gwtFixtureWebSocket.emit({
        kind: "project_index_status",
        project_root: "/fixture",
        status: {
          state: "repair_required",
          detail: "",
          progress: null,
          scopes: {
            files: {
              wtAhash: {
                healthy: false,
                repair_required: true,
                document_count: 0,
                reason: "manifest_missing",
              },
            },
            "files-docs": {
              wtAhash: { healthy: true, repair_required: false, document_count: 16 },
            },
          },
          worktrees: {
            wtAhash: { branch: "develop", path: "/abs/wtA" },
          },
        },
      });
    });
    await expect(dot).toHaveAttribute("data-state", "error", { timeout: 5_000 });

    // Transition to repairing (state==="repairing") → dot should be yellow.
    await page.evaluate(() => {
      window.__gwtFixtureWebSocket.emit({
        kind: "project_index_status",
        project_root: "/fixture",
        status: {
          state: "repairing",
          detail: "",
          progress: { scopes_done: 0, scopes_total: 1 },
          scopes: {
            files: {
              wtAhash: {
                healthy: true,
                repair_required: false,
                document_count: 1,
              },
            },
            "files-docs": {
              wtAhash: { healthy: true, repair_required: false, document_count: 16 },
            },
          },
          worktrees: {
            wtAhash: { branch: "develop", path: "/abs/wtA" },
          },
        },
      });
    });
    await expect(dot).toHaveAttribute("data-state", "repairing", { timeout: 5_000 });
  });

  test("multi-worktree dot aggregates: unhealthy in one worktree turns the dot red", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);

    // wtA healthy, wtB healthy → dot ready.
    await installIndexStatusBackend(page, {
      state: "ready",
      scopes: {
        files: {
          wtAhash: { healthy: true, repair_required: false, document_count: 310 },
          wtBhash: { healthy: true, repair_required: false, document_count: 200 },
        },
        "files-docs": {
          wtAhash: { healthy: true, repair_required: false, document_count: 16 },
          wtBhash: { healthy: true, repair_required: false, document_count: 10 },
        },
      },
      worktrees: {
        wtAhash: { branch: "develop", path: "/abs/wtA" },
        wtBhash: { branch: "feature/x", path: "/abs/wtB" },
      },
    });

    await page.goto(APP_URL);
    const dot = page.locator(".project-tab .project-tab-dot");
    await expect(dot).toHaveAttribute("data-state", "ready", { timeout: 10_000 });

    // Force wtB's files unhealthy → dot must flip to error even though
    // wtA stays healthy (aggregation is "any unhealthy → red").
    await page.evaluate(() => {
      window.__gwtFixtureWebSocket.emit({
        kind: "project_index_status",
        project_root: "/fixture",
        status: {
          state: "repair_required",
          detail: "",
          progress: null,
          scopes: {
            files: {
              wtAhash: { healthy: true, repair_required: false, document_count: 310 },
              wtBhash: {
                healthy: false,
                repair_required: true,
                document_count: 0,
                reason: "manifest_missing",
              },
            },
            "files-docs": {
              wtAhash: { healthy: true, repair_required: false, document_count: 16 },
              wtBhash: { healthy: true, repair_required: false, document_count: 10 },
            },
          },
          worktrees: {
            wtAhash: { branch: "develop", path: "/abs/wtA" },
            wtBhash: { branch: "feature/x", path: "/abs/wtB" },
          },
        },
      });
    });
    await expect(dot).toHaveAttribute("data-state", "error", { timeout: 5_000 });

    // Restore wtB to healthy → dot returns to ready.
    await page.evaluate(() => {
      window.__gwtFixtureWebSocket.emit({
        kind: "project_index_status",
        project_root: "/fixture",
        status: {
          state: "ready",
          detail: "",
          progress: null,
          scopes: {
            files: {
              wtAhash: { healthy: true, repair_required: false, document_count: 310 },
              wtBhash: { healthy: true, repair_required: false, document_count: 200 },
            },
            "files-docs": {
              wtAhash: { healthy: true, repair_required: false, document_count: 16 },
              wtBhash: { healthy: true, repair_required: false, document_count: 10 },
            },
          },
          worktrees: {
            wtAhash: { branch: "develop", path: "/abs/wtA" },
            wtBhash: { branch: "feature/x", path: "/abs/wtB" },
          },
        },
      });
    });
    await expect(dot).toHaveAttribute("data-state", "ready", { timeout: 5_000 });
  });

  test("badge click opens Settings.Index tab and renders the scope health table (T-IDX-106)", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "repair_required",
      scopes: {
        specs: {
          healthy: false,
          repair_required: true,
          document_count: 5,
          reason: "count_mismatch",
        },
        files: {
          wtAhash: {
            healthy: false,
            repair_required: true,
            document_count: 0,
            reason: "manifest_missing",
          },
        },
      },
      worktrees: {
        wtAhash: { branch: "develop", path: "/abs/wtA" },
      },
    });

    await page.goto(APP_URL);
    await expect(page.locator("#index-status")).toBeVisible({ timeout: 10_000 });
    await page.locator("#index-status").click();

    // The Settings window mounts asynchronously after the create_window
    // round-trip. The Index panel is one of three tabs and should be
    // active by the time we look for the table.
    const indexPanel = page.locator("[data-settings-panel='index']").first();
    await expect(indexPanel).toBeVisible({ timeout: 10_000 });
    await expect(indexPanel).not.toHaveClass(/hidden/);

    const table = indexPanel.locator("[data-role='index-settings-table']");
    await expect(table).toBeVisible();

    const specsRow = table.locator("tr[data-scope='specs']");
    await expect(specsRow.locator(".settings-index-cell.unhealthy"))
      .toContainText("count_mismatch");

    const filesRow = table.locator("tr[data-scope='files']");
    await expect(filesRow.locator(".settings-index-cell[data-worktree-hash='wtAhash']"))
      .toContainText("manifest_missing");

    // Worktree column header should reflect the supplied branch label.
    await expect(table.locator("thead th[data-worktree-hash='wtAhash']"))
      .toContainText("develop");
  });

  test("repairing click shows a progress toast (T-IDX-108)", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "repairing",
      progress: { scopes_done: 1, scopes_total: 4 },
    });

    await page.goto(APP_URL);
    await expect(page.locator("#index-status")).toBeVisible({ timeout: 10_000 });

    await page.locator("#index-status").click();

    const toast = page.locator("#index-status-toast");
    await expect(toast).toHaveAttribute("data-visible", "true");
    await expect(toast).toContainText(/1 of 4 scope/);
  });
});

async function installEmbeddedRoutes(page) {
  await page.route("http://gwt-playwright.local/**", async (route) => {
    const url = new URL(route.request().url());
    const assetPath = resolveAssetPath(url.pathname);
    if (!assetPath) {
      await route.fulfill({
        status: 404,
        contentType: "text/plain",
        body: `No test asset for ${url.pathname}`,
      });
      return;
    }
    await route.fulfill({
      path: assetPath,
      contentType: contentTypeFor(assetPath),
    });
  });
}

function resolveAssetPath(pathname) {
  if (pathname === "/" || pathname === "/index.html") {
    return path.join(WEB_ROOT, "index.html");
  }
  if (pathname === "/assets/xterm/xterm.css") {
    return path.join(WEB_ROOT, "vendor/xterm/xterm.css");
  }
  if (pathname === "/assets/xterm/xterm.mjs") {
    return path.join(WEB_ROOT, "vendor/xterm/xterm.mjs");
  }
  if (pathname === "/assets/xterm/addon-fit.mjs") {
    return path.join(WEB_ROOT, "vendor/xterm/addon-fit.mjs");
  }
  if (pathname.startsWith("/assets/fonts/")) {
    return path.join(WEB_ROOT, "fonts", path.basename(pathname));
  }
  if (pathname.startsWith("/styles/")) {
    return path.join(WEB_ROOT, "styles", path.basename(pathname));
  }
  const moduleName = pathname.slice(1);
  if (ROOT_MODULES.has(moduleName)) {
    return path.join(WEB_ROOT, moduleName);
  }
  return null;
}

function contentTypeFor(assetPath) {
  if (assetPath.endsWith(".html")) return "text/html";
  if (assetPath.endsWith(".css")) return "text/css";
  if (assetPath.endsWith(".js") || assetPath.endsWith(".mjs")) {
    return "text/javascript";
  }
  if (assetPath.endsWith(".woff2")) return "font/woff2";
  return "application/octet-stream";
}

async function installIndexStatusBackend(page, indexStatus) {
  await page.addInitScript((indexStatusPayload) => {
    const projectRoot = "/fixture";
    const baseTabState = {
      id: "tab-1",
      title: "Fixture Project",
      project_root: projectRoot,
      kind: "git",
      workspace: {
        viewport: { x: 0, y: 0, zoom: 1 },
        windows: [],
      },
    };
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [JSON.parse(JSON.stringify(baseTabState))],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };
    const projectIndexStatus = {
      kind: "project_index_status",
      project_root: projectRoot,
      status: {
        state: indexStatusPayload.state,
        // Empty detail by default so the formatted title strings from
        // index-status-controller.js (`Auto-rebuild not started`,
        // `Auto-rebuild failed`, etc.) drive the title attribute. Tests
        // that need a specific detail can pass one explicitly.
        detail: indexStatusPayload.detail || "",
        progress: indexStatusPayload.progress || null,
        scopes: indexStatusPayload.scopes || {},
        worktrees: indexStatusPayload.worktrees || {},
      },
    };

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw) {
        let message;
        try {
          message = JSON.parse(raw);
        } catch (e) {
          return;
        }
        if (message.kind === "frontend_ready") {
          this.emit(workspaceState);
          this.emit(projectIndexStatus);
          return;
        }
        // SPEC-1939 T-IDX-106: simulate the backend create_window behaviour
        // for `preset === "settings"` so click → settings:open →
        // focusOrSpawnPreset("settings") can drive a real Settings window
        // mount end-to-end. The fixture appends a Settings window to the
        // current tab's workspace and re-emits workspace_state.
        if (message.kind === "create_window" && message.preset === "settings") {
          const tab = workspaceState.workspace.tabs[0];
          tab.workspace.windows = (tab.workspace.windows || []).concat([
            {
              id: `settings-${Date.now()}`,
              title: "Settings",
              preset: "settings",
              geometry: { x: 96, y: 76, width: 720, height: 540 },
              z_index: tab.workspace.windows.length + 1,
              status: "running",
              minimized: false,
              maximized: false,
              pre_maximize_geometry: null,
              persist: true,
              purpose_title: null,
              dynamic_title: null,
              dynamic_title_detail: null,
              agent_id: null,
              agent_color: null,
              tab_group_id: null,
              tab_group_active: false,
            },
          ]);
          this.emit(workspaceState);
        }
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload) {
        setTimeout(() => {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }, 0);
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });

    // Expose the most recently constructed FixtureWebSocket on window so
    // transition tests can drive additional `project_index_status` events
    // without needing a second WebSocket. The wrapper guards against a
    // race where the test calls `.emit` before app.js has constructed the
    // WebSocket — in that case `__gwtFixtureWebSocket` is undefined and
    // the test fails fast.
    const originalConstructor = FixtureWebSocket;
    const FixtureWebSocketWithTracking = function (url) {
      const instance = new originalConstructor(url);
      window.__gwtFixtureWebSocket = instance;
      return instance;
    };
    FixtureWebSocketWithTracking.CONNECTING = 0;
    FixtureWebSocketWithTracking.OPEN = 1;
    FixtureWebSocketWithTracking.CLOSING = 2;
    FixtureWebSocketWithTracking.CLOSED = 3;
    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocketWithTracking,
    });
  }, indexStatus);
}
