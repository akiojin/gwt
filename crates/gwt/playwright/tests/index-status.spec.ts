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
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Fixture Project",
            project_root: projectRoot,
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [],
            },
          },
        ],
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
  }, indexStatus);
}
