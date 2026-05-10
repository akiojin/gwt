/* SPEC-2017 Phase 4 — Knowledge Bridge Kanban visual coverage.
 *
 * The fixture serves the embedded frontend assets directly through Playwright
 * routes and replaces WebSocket with a deterministic cache-backed backend.
 * That keeps visual coverage active in CI without depending on a live gwt GUI
 * process, GitHub cache state, or the user's local workspace.
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

test.describe("Knowledge Bridge Kanban visual snapshots", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 3840, height: 1100 },
  });

  for (const hideDone of [false, true]) {
    test(`${hideDone ? "hides" : "shows"} done column`, async ({ page }, testInfo) => {
      await installEmbeddedRoutes(page);
      await installKanbanBackend(page, {
        hideDone,
        theme: testInfo.project.name.includes("light") ? "light" : "dark",
      });

      await page.goto(APP_URL);

      const board = page.locator(".surface-knowledge .kanban-board");
      await expect(board).toHaveAttribute(
        "data-hide-done",
        hideDone ? "true" : "false",
      );
      await expect(page.locator(".surface-knowledge .kanban-card")).toHaveCount(6);
      await expect(
        page.locator(".surface-knowledge .kanban-column[data-phase='done'] [data-role='count']"),
      ).toHaveText("1");

      await expect(board).toHaveScreenshot(
        hideDone ? "kanban-hide-done.png" : "kanban-show-done.png",
      );
    });
  }
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

async function installKanbanBackend(page, { hideDone, theme }) {
  await page.addInitScript(
    ({ hideDone: shouldHideDone, theme: selectedTheme }) => {
      const entries = [
        {
          number: 2017,
          title: "SPEC Issue Kanban View",
          state: "open",
          meta: "Phase 4 visual coverage",
          labels: ["gwt-spec", "phase/implementation"],
          linked_branch_count: 2,
          match_score: 99,
          phase: "implementation",
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 1935,
          title: "Coordination hooks and Board reminders",
          state: "open",
          meta: "Planning refinement",
          labels: ["gwt-spec", "phase/planning"],
          linked_branch_count: 1,
          match_score: 88,
          phase: "planning",
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 2008,
          title: "Window host interaction model",
          state: "open",
          meta: "Review follow-up",
          labels: ["gwt-spec", "phase/review"],
          linked_branch_count: 3,
          match_score: 82,
          phase: "review",
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 2077,
          title: "Runtime daemon event transport",
          state: "open",
          meta: "Draft architecture",
          labels: ["gwt-spec", "phase/draft"],
          linked_branch_count: 0,
          match_score: 76,
          phase: "draft",
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 2359,
          title: "Workspace Kanban stabilization",
          state: "open",
          meta: "Unscheduled backlog",
          labels: ["gwt-spec"],
          linked_branch_count: 0,
          match_score: 71,
          phase: null,
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 2470,
          title: "Merge Kanban implementation bundle",
          state: "closed",
          meta: "Completed rollout",
          labels: ["gwt-spec", "phase/done"],
          linked_branch_count: 1,
          match_score: 100,
          phase: "done",
          has_unknown_phase: false,
          is_spec: true,
        },
      ];

      const workspaceState = {
        kind: "workspace_state",
        workspace: {
          app_version: "playwright",
          tabs: [
            {
              id: "tab-1",
              title: "Fixture Project",
              project_root: "/fixture",
              kind: "git",
              workspace: {
                viewport: { x: 0, y: 0, zoom: 1 },
                windows: [
                  {
                    id: "spec-kanban",
                    title: "SPEC Kanban",
                    preset: "spec",
                    geometry: { x: 96, y: 76, width: 3600, height: 820 },
                    z_index: 1,
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
                ],
              },
            },
          ],
          active_tab_id: "tab-1",
          recent_projects: [],
        },
      };

      localStorage.setItem("gwt:ui:theme", selectedTheme);
      if (shouldHideDone) {
        localStorage.setItem("kanban-hide-done", "1");
      } else {
        localStorage.removeItem("kanban-hide-done");
      }

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
          const message = JSON.parse(raw);
          if (message.kind === "frontend_ready") {
            this.emit(workspaceState);
            return;
          }
          if (message.kind === "load_knowledge_bridge") {
            this.emit({
              kind: "knowledge_entries",
              id: message.id,
              knowledge_kind: message.knowledge_kind,
              request_id: message.request_id,
              list_scope: message.list_scope ?? null,
              entries,
              selected_number: 2017,
              empty_message: null,
              refresh_enabled: true,
            });
            return;
          }
          if (message.kind === "select_knowledge_bridge_entry") {
            this.emit({
              kind: "knowledge_detail",
              id: message.id,
              knowledge_kind: message.knowledge_kind,
              request_id: message.request_id,
              detail: {
                number: message.number,
                title: `SPEC #${message.number}`,
                state: "open",
                subtitle: "Deterministic fixture detail",
                labels: ["gwt-spec"],
                launch_issue_number: message.number,
                sections: [
                  {
                    title: "Acceptance",
                    body: "Kanban columns stay readable in dark and light themes.",
                  },
                ],
              },
            });
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
    },
    { hideDone, theme },
  );
}
