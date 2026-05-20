/* SPEC-1939 Phase 13 — project-bar Index badge withdrawn. The remaining
 * coverage exercises the per-tab dot aggregator and the Settings.Index
 * panel (per-cell rebuild IPC) using the SPEC-2017 Kanban fixture pattern:
 * the embedded frontend is served via `installEmbeddedRoutes`
 * (`_helpers/embedded-frontend.ts`) and the WebSocket is stubbed with a
 * deterministic backend that emits canned `workspace_state` +
 * `project_index_status` events. No xvfb / wry / live gwt process required.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Project Index status surface", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("project-bar Index badge has been withdrawn (Phase 13)", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, { state: "repair_required" });

    await page.goto(APP_URL);

    // The project tab still mounts, but the legacy badge slot must be gone.
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator("#index-status")).toHaveCount(0);
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

  test("Settings.Index renders the scope health table from project_index_status (T-IDX-106)", async ({
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
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });

    // SPEC-1939 Phase 13: badge entry point is gone; tests drive the
    // Settings.Index tab directly via the canonical `settings:open` event.
    await page.evaluate(() => {
      document.dispatchEvent(
        new CustomEvent("settings:open", { detail: { target: "index" }, bubbles: true }),
      );
    });

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

  test("Settings.Index scope-row Rebuild all dispatches without worktree_hash", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "repair_required",
      scopes: {
        issues: {
          healthy: false,
          repair_required: true,
          document_count: 0,
          reason: "manifest_missing",
        },
      },
    });

    await page.goto(APP_URL);
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });
    await page.evaluate(() => {
      document.dispatchEvent(
        new CustomEvent("settings:open", { detail: { target: "index" }, bubbles: true }),
      );
    });

    const issuesRow = page
      .locator("[data-settings-panel='index'] tr[data-scope='issues']")
      .first();
    await expect(issuesRow).toBeVisible({ timeout: 10_000 });

    // The scope-row Rebuild button lives in the row header (`th`),
    // distinct from the per-cell Rebuild button inside `td`.
    const rebuildAll = issuesRow.locator(".settings-index-rebuild-all[data-scope='issues']");
    await rebuildAll.click();

    const lastRebuild = await page.evaluate(() => {
      const sends = (window.__gwtFixtureWebSocket && window.__gwtFixtureWebSocket.recordedSends) || [];
      return sends
        .map((raw) => {
          try {
            return JSON.parse(raw);
          } catch (e) {
            return null;
          }
        })
        .filter((m) => m && m.kind === "rebuild_index_cell")
        .pop();
    });

    expect(lastRebuild).toMatchObject({
      kind: "rebuild_index_cell",
      project_root: "/fixture",
      scope: "issues",
    });
    // Repo-shared scopes (`issues`, `specs`) must NOT carry a worktree_hash
    // since they are not per-worktree.
    expect(lastRebuild).not.toHaveProperty("worktree_hash");
  });

  test("Settings.Index renders the memory scope row and dispatches rebuild_index_cell (SPEC-2805)", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "repair_required",
      scopes: {
        memory: {
          healthy: false,
          repair_required: true,
          document_count: 0,
          reason: "manifest_missing",
        },
      },
    });

    await page.goto(APP_URL);
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });
    await page.evaluate(() => {
      document.dispatchEvent(
        new CustomEvent("settings:open", { detail: { target: "index" }, bubbles: true }),
      );
    });

    const memoryRow = page
      .locator("[data-settings-panel='index'] tr[data-scope='memory']")
      .first();
    await expect(memoryRow).toBeVisible({ timeout: 10_000 });
    await expect(memoryRow.locator(".settings-index-cell.unhealthy")).toContainText(
      "manifest_missing",
    );

    const rebuildAll = memoryRow.locator(".settings-index-rebuild-all[data-scope='memory']");
    await rebuildAll.click();

    const lastRebuild = await page.evaluate(() => {
      const sends = (window.__gwtFixtureWebSocket && window.__gwtFixtureWebSocket.recordedSends) || [];
      return sends
        .map((raw) => {
          try {
            return JSON.parse(raw);
          } catch (e) {
            return null;
          }
        })
        .filter((m) => m && m.kind === "rebuild_index_cell")
        .pop();
    });

    expect(lastRebuild).toMatchObject({
      kind: "rebuild_index_cell",
      project_root: "/fixture",
      scope: "memory",
    });
    // Memory is repo-scoped just like issues/specs, so worktree_hash must
    // not be included in the dispatched IPC payload.
    expect(lastRebuild).not.toHaveProperty("worktree_hash");
  });

  test("Settings.Index per-cell Rebuild dispatches rebuild_index_cell IPC (T-IDX-102/T-IDX-110)", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "error",
      scopes: {
        files: {
          wtAhash: {
            healthy: false,
            repair_required: true,
            document_count: 0,
            reason: "rebuild_failed",
          },
        },
      },
      worktrees: {
        wtAhash: { branch: "develop", path: "/abs/wtA" },
      },
    });

    await page.goto(APP_URL);
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });
    await page.evaluate(() => {
      document.dispatchEvent(
        new CustomEvent("settings:open", { detail: { target: "index" }, bubbles: true }),
      );
    });

    // Wait for Settings.Index to render the row, then click the per-cell
    // Rebuild button.
    const filesCell = page
      .locator("[data-settings-panel='index'] tr[data-scope='files'] .settings-index-cell[data-worktree-hash='wtAhash']")
      .first();
    await expect(filesCell).toBeVisible({ timeout: 10_000 });

    await filesCell.locator(".settings-index-rebuild").click();

    // The fixture WebSocket has captured every send() call; the click
    // must enqueue a `rebuild_index_cell` payload with the right scope +
    // worktree hash.
    const lastRebuild = await page.evaluate(() => {
      const sends = (window.__gwtFixtureWebSocket && window.__gwtFixtureWebSocket.recordedSends) || [];
      return sends
        .map((raw) => {
          try {
            return JSON.parse(raw);
          } catch (e) {
            return null;
          }
        })
        .filter((m) => m && m.kind === "rebuild_index_cell")
        .pop();
    });

    expect(lastRebuild).toMatchObject({
      kind: "rebuild_index_cell",
      project_root: "/fixture",
      scope: "files",
      worktree_hash: "wtAhash",
    });
  });

});

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
        // SPEC-1939 T-IDX-102 — record every payload the frontend sends so
        // tests can assert on `rebuild_index_cell` and similar dispatches.
        this.recordedSends = [];
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw) {
        this.recordedSends.push(raw);
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
