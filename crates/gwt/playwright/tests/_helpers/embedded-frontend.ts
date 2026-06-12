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

const WEB_ROOT = path.resolve(process.cwd(), "crates/gwt/web");

export const APP_URL = "http://gwt-playwright.local/";

const ROOT_MODULES = new Set([
  "app.js",
  "board-surface.js",
  "branch-cleanup-modal.js",
  // SPEC-2009 Phase 7 (FR-064..FR-067): Branches detail-check reconnect
  // self-heal / last-known retention / stale-load guard.
  "branch-list-state.js",
  // SPEC-2008 Phase 29: latch canvas wheel pan/zoom mode per gesture.
  "canvas-wheel-gesture.js",
  // SPEC-2013 FR-012 — confirm modal shown when closing a project tab
  // while one or more agent panes are still running.
  "close-project-tab-confirm-modal.js",
  "window-close-confirm-modal.js",
  // Issue #2704 — terminal-focus guard for modal-friendly workspace renders.
  "clone-modal-focus-guard.js",
  "custom-agent-env-editor.js",
  "focus-trap.js",
  "hotkey.js",
  "index-settings-panel.js",
  // SPEC-2014 2026-05-29 — Launch Agent setting controls (reasoning slider +
  // Auto toggle, count-adaptive segmented/select, boolean toggle).
  "launch-controls.js",
  "launch-pending-controller.js",
  "connection-overlay.js",
  // Issue #2698 PR 1 (B7) — defer destructive wizard re-renders.
  "interaction-guard.js",
  "migration-modal.js",
  "operator-shell.js",
  "project-clone-modal.js",
  // SPEC-3064 Phase 3 (E3) — Project Index window surface.
  "project-index-search-surface.js",
  "project-tabs-renderer.js",
  // SPEC-3064 Phase 3 (E1) — provider usage & rate limits surface.
  "provider-usage-surface.js",
  "window-tabs-renderer.js",
  // SPEC-3015 — generated protocol enum contract + extracted window runtime
  // state helpers.
  "protocol-enums.js",
  "window-runtime-state.js",
  // SPEC #2780 — Release Notes window opened from #app-version label.
  "release-notes-window.js",
  // SPEC-2809 — Console window per-kind tab live tail.
  "console-window.js",
  "socket-receive-dispatcher.js",
  // SPEC-3064 Phase 3 (E2) — terminal attachments & clipboard surface.
  "terminal-attachments.js",
  "terminal-copy-shortcut.js",
  "terminal-context-menu.js",
  "terminal-output-buffer.js",
  "terminal-wheel-scroll.js",
  "terminal-viewport-reflow.js",
  "theme-manager.js",
  "theme-toggle.js",
  "ui-trace-profiler.js",
  "ui-trace-wiring.js",
  "update-cta.js",
  // Issue #2698 PR 2 (B1) — throttle update_viewport WS sends.
  "viewport-persist-throttle.js",
  "viewport-sync.js",
  "window-geometry-sync.js",
  "window-docking.js",
  "workspace-kanban-surface.js",
  // SPEC-2359 US-42 — Workspace Resume Picker modal renderer.
  "workspace-resume-picker-modal.js",
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
  // SPEC-2009 Phase 2b: highlight.js bundled module + dark theme.
  if (pathname === "/assets/highlight/highlight.min.js") {
    return path.join(WEB_ROOT, "vendor/highlight/highlight.min.js");
  }
  if (pathname === "/assets/highlight/github-dark.min.css") {
    return path.join(WEB_ROOT, "vendor/highlight/github-dark.min.css");
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
