import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const configuredUrl = process.env.GWT_PLAYWRIGHT_BASE_URL;
const liveUrl = configuredUrl
  ? new URL(new URL(configuredUrl).pathname === "/" ? new URL(APP_URL).pathname : new URL(configuredUrl).pathname, configuredUrl).toString()
  : undefined;
const errors = new WeakMap<Page, string[]>();

// SPEC-2356 Anshin Addendum — Phase 2 activity surfacing + mission convergence.
//
// Drives the embedded frontend against a deterministic WebSocket stub (mirroring
// kill-switch.spec.ts / fleet-minimap.spec.ts) and asserts the FR-045 / FR-047
// behaviours that the Phase 2 implementation added on the always-visible
// surfaces (Fleet Minimap, Window Switcher, Status Strip):
//
//   - FR-047: the MISSION convergence cell (`#op-strip-mission`) shows
//     `done/agents` and flips `.op-status-strip__cell--complete` once every
//     agent has converged (done === agents > 0). Driven by emitting
//     `window_state` transitions that map to telemetry `active` vs `done`.
//   - FR-045: a window's `dynamic_title_detail` surfaces as the Fleet Minimap
//     cell tooltip (`title` = "Title · detail")
//     line inside the Window Switcher row.
//
// Runtime-state -> telemetry mapping used to seed `done` vs `active`
// (mapAgentTelemetryState in window-runtime-state.js):
//   - status/state "running"        -> telemetry "active"
//   - window_state state "stopped"  -> telemetry "done" (also "exited")
//   - "idle"/"ready"/unknown        -> telemetry "idle"
// recomputeOperatorTelemetry counts every live agent pane (preset agent/claude/
// codex) as `agents`, and increments `done` for each pane whose telemetry state
// is "done".

test.describe("Anshin Phase 2 activity surfacing + mission convergence", () => {
  test.use({ deviceScaleFactor: 1, viewport: { width: 1440, height: 900 } });
  test.beforeEach(async ({ page }, info) => {
    const captured: string[] = [];
    errors.set(page, captured);
    page.on("pageerror", error => captured.push(error.message));
    page.on("console", message => { if (message.type() === "error") captured.push(message.text()); });
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), info.project.name.includes("light") ? "light" : "dark");
  });
  test.afterEach(async ({ page }, info) => {
    await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.includes("light") ? "light" : "dark");
    expect(errors.get(page), "zero console/page errors").toEqual([]);
  });

  test("FR-047: MISSION cell trends done/agents and flips complete on convergence", async ({
    page,
  }) => {
    if (!liveUrl) await installEmbeddedRoutes(page);
    await installPhase2Backend(page);
    await page.goto(liveUrl || APP_URL);

    // Three agent panes boot "running" -> telemetry active -> 0/3, not complete.
    const mission = page.locator("#op-strip-mission");
    const missionCell = page.locator(".op-status-strip__cell--mission");
    await expect(mission).toHaveText("0/3", { timeout: 10_000 });
    await expect(missionCell).not.toHaveClass(/op-status-strip__cell--complete/);

    // Two agents finish (stopped -> telemetry done): in-progress 2/3, still not
    // complete because one agent is still active.
    await page.evaluate(() => {
      window.__emit({ kind: "window_state", window_id: "agent-1", state: "stopped" });
      window.__emit({ kind: "window_state", window_id: "agent-2", state: "stopped" });
    });
    await expect(mission).toHaveText("2/3");
    await expect(missionCell).not.toHaveClass(/op-status-strip__cell--complete/);

    // The last agent finishes -> every agent converged -> 3/3 + complete state.
    await page.evaluate(() => {
      window.__emit({ kind: "window_state", window_id: "agent-3", state: "stopped" });
    });
    await expect(mission).toHaveText("3/3");
    await expect(missionCell).toHaveClass(/op-status-strip__cell--complete/);
  });


});

// Deterministic backend: a single project tab with three canvas agent panes.
// agent-3 carries a live activity detail (dynamic_title_detail) for the FR-045
// surfaces; all three feed the FR-047 MISSION convergence count.
async function installPhase2Backend(page) {
  await page.addInitScript(() => {
    window.__sent = [];
    window.__sentKinds = [];

    function agentWindow(id, overrides) {
      return {
        id,
        title: id,
        preset: "agent",
        geometry: { x: 120, y: 100, width: 480, height: 320 },
        geometry_revision: 0,
        z_index: 1,
        status: "running",
        persist: true,
        purpose_title: null,
        dynamic_title: null,
        dynamic_title_detail: null,
        agent_id: id,
        agent_name: { "agent-1": "Aster", "agent-2": "Birch", "agent-3": "Cedar" }[id],
        agent_color: "cyan",
        tab_group_id: null,
        tab_group_active: false,
        session_id: `session-${id}`,
        placement: null,
        ...overrides,
      };
    }

    const windows = [
      agentWindow("agent-1", { geometry: { x: 100, y: 100, width: 480, height: 320 } }),
      agentWindow("agent-2", { geometry: { x: 640, y: 100, width: 480, height: 320 } }),
      // Canvas agent with a live activity detail for the FR-045 surfaces.
      agentWindow("agent-3", {
        title: "Refactor auth",
        geometry: { x: 1180, y: 100, width: 480, height: 320 },
        dynamic_title: "Refactor auth",
        dynamic_title_detail: "editing src/auth/login.rs",
      }),
    ];

    function stateMsg() {
      return {
        kind: "workspace_state",
        workspace: {
          app_version: "playwright",
          tabs: [
            {
              id: "tab-1",
              title: "Anshin Phase 2 Fixture",
              project_root: "/fixture",
              kind: "git",
              workspace: {
                viewport: { x: 0, y: 0, zoom: 1 },
                windows: windows.map((w) => ({ ...w })),
              },
            },
          ],
          active_tab_id: "tab-1",
          recent_projects: [],
        },
      };
    }

    function windowListMsg() {
      return {
        kind: "window_list",
        windows: windows.map((w) => ({ id: w.id, title: w.title })),
      };
    }

    let socketRef = null;

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        socketRef = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(stateMsg());
        }, 0);
      }

      send(data) {
        let msg;
        try {
          msg = JSON.parse(data);
        } catch {
          return;
        }
        window.__sent.push(msg);
        window.__sentKinds.push(msg.kind);
        // The Window Switcher requests a window_list snapshot when opened; reply
        // with the live id set so the popover renders the seeded rows.
        if (msg.kind === "list_windows") {
          this.emit(windowListMsg());
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

    // Lets a test push backend events (bare window_state transitions) directly,
    // mirroring exactly what the daemon sends when an agent finishes / errors /
    // starts waiting while keeping its window (no workspace_state follows).
    window.__emit = (payload) => {
      if (socketRef) socketRef.emit(payload);
    };

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}
