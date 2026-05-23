/* SPEC-1939 Phase 13+15 — project-bar Index badge withdrawn. The remaining
 * coverage exercises the per-tab dot aggregator and the dedicated Index
 * window health panel (per-cell rebuild IPC) using the SPEC-2017 Kanban fixture pattern:
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

  test("Index window Health renders the scope health table from project_index_status (T-IDX-106)", async ({
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
    const { table } = await openIndexHealthPanel(page);
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

  test("Index window Health scope-row Rebuild all dispatches without worktree_hash", async ({ page }) => {
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
    const { table } = await openIndexHealthPanel(page);
    const issuesRow = table.locator("tr[data-scope='issues']").first();
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

  test("Index window Health renders the memory scope row and dispatches rebuild_index_cell (SPEC-2805)", async ({
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
    const { table } = await openIndexHealthPanel(page);
    const memoryRow = table.locator("tr[data-scope='memory']").first();
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

  test("Index window Search shows animated loading feedback in the results pane", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "ready",
      scopes: {
        issues: { healthy: true, repair_required: false, document_count: 214 },
        specs: { healthy: true, repair_required: false, document_count: 774 },
        memory: { healthy: true, repair_required: false, document_count: 260 },
        board: { healthy: true, repair_required: false, document_count: 1471 },
      },
      worktrees: {},
    });

    await page.goto(APP_URL);
    const { root } = await openIndexSearchPanel(page);
    await root.locator(".index-search-input").fill("memory search loading");
    await root.locator(".index-run-button").click();

    const loading = root.locator(".index-search-loading");
    await expect(loading).toBeVisible({ timeout: 5_000 });
    await expect(loading).toHaveAttribute("role", "status");
    await expect(loading).toContainText("Searching semantic index");
    await expect(root.locator(".index-search-layout")).toHaveAttribute("aria-busy", "true");

    const animatedDot = loading.locator(".index-search-loading-dot").first();
    await expect(animatedDot).toBeVisible();
    await expect(animatedDot).toHaveCSS("animation-name", /index-search-loading/);
  });

  test("Index window Search all-terms mode separates strict results and suggestions", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "ready",
      scopes: {
        specs: { healthy: true, repair_required: false, document_count: 774 },
        discussions: { healthy: true, repair_required: false, document_count: 91 },
      },
      worktrees: {},
    });

    await page.goto(APP_URL);
    const { root } = await openIndexSearchPanel(page);
    await root.locator("[data-match-mode='all_terms']").click();
    await root.locator(".index-search-input").fill("Workspace volatile");
    await root.locator(".index-run-button").click();

    const request = await page.waitForFunction(() => {
      const sends = (window.__gwtFixtureWebSocket && window.__gwtFixtureWebSocket.recordedSends) || [];
      return sends
        .map((raw) => {
          try {
            return JSON.parse(raw);
          } catch (e) {
            return null;
          }
        })
        .filter((m) => m && m.kind === "search_project_index")
        .pop();
    });
    const requestPayload = await request.jsonValue();
    expect(requestPayload).toMatchObject({
      kind: "search_project_index",
      match_mode: "all_terms",
      query: "Workspace volatile",
    });

    await page.evaluate((payload) => {
      window.__gwtFixtureWebSocket.emit(payload);
    }, {
      kind: "project_index_search_results",
      project_root: "/fixture",
      id: requestPayload.id,
      request_id: requestPayload.request_id,
      query: requestPayload.query,
      scope: "all",
      results: [
        {
          scope: "specs",
          title: "Workspace volatility decision",
          subtitle: "SPEC #1939",
          preview: "Workspace is current state; Work is durable.",
          distance: 0.08,
          match_mode: "all_terms",
          matched_terms: ["Workspace", "volatile"],
          missing_terms: [],
        },
      ],
      suggestions: [
        {
          scope: "discussions",
          title: "Workspace naming discussion",
          subtitle: "discussion",
          preview: "Workspace was confusing and Work may be a better durable unit.",
          distance: 0.18,
          match_mode: "all_terms",
          matched_terms: ["Workspace"],
          missing_terms: ["volatile"],
        },
      ],
    });

    await expect(root.locator(".index-search-status")).toContainText(
      "1 strict results · 1 semantic suggestions",
    );
    await expect(root.locator(".index-result-group-label").first()).toContainText("Strict results");
    await expect(root.locator(".index-result-row").first()).toContainText(
      "Workspace volatility decision",
    );
    await expect(root.locator(".index-result-row.is-suggestion")).toContainText(
      "Workspace naming discussion",
    );
    await expect(root.locator(".index-result-row.is-suggestion")).toContainText(
      "Matched: Workspace",
    );
    await root.locator(".index-result-row.is-suggestion").click();
    await expect(root.locator(".index-detail-meta", { hasText: "Missing: volatile" })).toBeVisible();
  });

  test("Index window Health per-cell Rebuild dispatches rebuild_index_cell IPC (T-IDX-102/T-IDX-110)", async ({
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
    const { table } = await openIndexHealthPanel(page);

    // Wait for Index Health to render the row, then click the per-cell
    // Rebuild button.
    const filesCell = table
      .locator("tr[data-scope='files'] .settings-index-cell[data-worktree-hash='wtAhash']")
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

async function openIndexHealthPanel(page) {
  await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });

  // SPEC-1939 Phase 15: settings target=index is now the compatibility
  // entrypoint for the dedicated Index window, not a Settings tab.
  await page.evaluate(() => {
    document.dispatchEvent(
      new CustomEvent("settings:open", { detail: { target: "index" }, bubbles: true }),
    );
  });

  const root = page.locator(".index-search-root").first();
  await expect(root).toBeVisible({ timeout: 10_000 });
  await root.locator("[data-index-tab='health']").click();

  const panel = root.locator("[data-index-panel='health']");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  const table = panel.locator("[data-role='index-settings-table']");
  return { root, panel, table };
}

async function openIndexSearchPanel(page) {
  await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });

  await page.evaluate(() => {
    document.dispatchEvent(
      new CustomEvent("settings:open", { detail: { target: "index" }, bubbles: true }),
    );
  });

  const root = page.locator(".index-search-root").first();
  await expect(root).toBeVisible({ timeout: 10_000 });
  await root.locator("[data-index-tab='search']").click();

  const panel = root.locator("[data-index-panel='search']");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  return { root, panel };
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
        // SPEC-1939 T-IDX-106/Phase 15: simulate the backend create_window
        // behaviour for `preset === "index"` so settings:open target=index →
        // focusOrSpawnPreset("index") can drive a real Index window mount
        // end-to-end. The fixture appends an Index window to the
        // current tab's workspace and re-emits workspace_state.
        if (message.kind === "create_window" && message.preset === "index") {
          const tab = workspaceState.workspace.tabs[0];
          tab.workspace.windows = (tab.workspace.windows || []).concat([
            {
              id: `index-${Date.now()}`,
              title: "Index",
              preset: "index",
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
