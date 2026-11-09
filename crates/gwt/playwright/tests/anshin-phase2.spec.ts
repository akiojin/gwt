import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const configuredUrl = process.env.GWT_PLAYWRIGHT_BASE_URL;
const liveUrl = configuredUrl
  ? new URL(new URL(configuredUrl).pathname === "/" ? new URL(APP_URL).pathname : new URL(configuredUrl).pathname, configuredUrl).toString()
  : undefined;
const errors = new WeakMap<Page, string[]>();

// SPEC #3340 AS-03: stopping agent processes is not a mission completion rate.
// Keep this deterministic frontend fixture against either embedded routes or
// the isolated checkout server, and assert both themes without browser errors.

test.describe("Status Strip runtime telemetry", () => {
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

  test("AS-03: stopped agents never render a MISSION completion indicator", async ({ page }, info) => {
    if (!liveUrl) await installEmbeddedRoutes(page);
    await installPhase2Backend(page);
    await page.goto(liveUrl || APP_URL);

    await expect(page.locator("#op-strip-running")).toHaveText("3");
    await expect(page.locator("#op-strip-mission, .op-status-strip__cell--mission")).toHaveCount(0);
    await expect(page.locator("#op-briefing")).toHaveCount(1);
    await page.evaluate(() => {
      for (const id of ["agent-1", "agent-2", "agent-3"]) {
        window.__emit({ kind: "window_state", window_id: id, state: "stopped" });
      }
      window.__emit({ kind: "active_work_projection", projection: {
        id: "stopped-census", active_works: [], local_agent_activity: {
          project_key: location.pathname.split("/").filter(Boolean).at(-1),
          host_id: "mission-fixture", observed_at: new Date().toISOString(), agents: [],
        },
      } });
    });
    await expect(page.locator("#op-strip-running")).toHaveText("0");
    await expect(page.locator("#op-strip-idle")).toHaveText("0");
    await expect(page.locator("#op-strip-mission, .op-status-strip__cell--complete")).toHaveCount(0);
    await expect(page.locator("#op-strip-branches")).toBeVisible();
    await info.attach("stopped-status-strip", {
      body: await page.screenshot({ path: info.outputPath("stopped-status-strip.png") }), contentType: "image/png",
    });
  });


});

// Deterministic backend: a single project tab with three canvas agent panes.
// All three initially have live children in the host census.
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
              project_key: location.pathname.split("/").filter(Boolean).at(-1),
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
          this.emit({ kind: "active_work_projection", projection: {
            id: "running-census", active_works: [], local_agent_activity: {
              project_key: location.pathname.split("/").filter(Boolean).at(-1),
              host_id: "mission-fixture", observed_at: new Date().toISOString(),
              agents: windows.map(w => ({ window_id: w.id, session_id: w.session_id, runtime_incarnation: 1, is_pm: false, issue_number: null })),
            },
          } });
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
    // mirroring exactly what the daemon sends when an agent stops / errors /
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
