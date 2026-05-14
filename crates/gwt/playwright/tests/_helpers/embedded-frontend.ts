/* SPEC-1939 Phase 12 — shared Playwright fixture for the embedded gwt
 * frontend.
 *
 * Provides a `page.route` handler that serves `crates/gwt/web/**` directly
 * so behaviour specs can boot the full frontend without launching a live
 * `gwt` GUI process. The companion specs install their own WebSocket stub
 * so they can drive deterministic backend events. See
 * `tests/kanban.spec.ts` and `tests/index-status.spec.ts` for examples.
 */
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const WEB_ROOT = path.resolve(HERE, "../../../web");

export const APP_URL = "http://gwt-playwright.local/";

const ROOT_MODULES = new Set([
  "app.js",
  "board-surface.js",
  "branch-cleanup-modal.js",
  // Issue #2704 — terminal-focus guard for modal-friendly workspace renders.
  "clone-modal-focus-guard.js",
  "custom-agent-env-editor.js",
  "focus-trap.js",
  "hotkey.js",
  "index-settings-panel.js",
  "index-status-controller.js",
  // Issue #2698 PR 1 (B7) — defer destructive wizard re-renders.
  "interaction-guard.js",
  "migration-modal.js",
  "operator-shell.js",
  "project-clone-modal.js",
  "socket-receive-dispatcher.js",
  "terminal-context-menu.js",
  "terminal-viewport-reflow.js",
  "theme-manager.js",
  "theme-toggle.js",
  "update-cta.js",
  // Issue #2698 PR 2 (B1) — throttle update_viewport WS sends.
  "viewport-persist-throttle.js",
  "window-geometry-sync.js",
  "window-docking.js",
  "workspace-kanban-surface.js",
]);

export async function installEmbeddedRoutes(page: any): Promise<void> {
  await page.route(`${APP_URL}**`, async (route: any) => {
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

function resolveAssetPath(pathname: string): string | null {
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

function contentTypeFor(assetPath: string): string {
  if (assetPath.endsWith(".html")) return "text/html";
  if (assetPath.endsWith(".css")) return "text/css";
  if (assetPath.endsWith(".js") || assetPath.endsWith(".mjs")) {
    return "text/javascript";
  }
  if (assetPath.endsWith(".woff2")) return "font/woff2";
  return "application/octet-stream";
}
