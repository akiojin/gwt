import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// Regression: a maximized window's fill is a LOCAL, per-client view concern.
// Two clients with different viewport sizes used to ping-pong the shared
// maximized geometry forever (each client forced its own visibleBounds back
// into the shared state), which the user saw as severe flicker.
//
// The fix makes each client render the maximized fill locally and never send a
// `maximize_window` correction. This test simulates a *competing client* by
// injecting a workspace_state where the same window is maximized at a much
// smaller width, then asserts the client neither sends a correction nor lets
// the foreign geometry shrink its own window.
test.describe("Maximize multi-client (no ping-pong)", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("a competing client's maximized geometry never triggers a correction", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installMaximizeBackend(page);

    await page.goto(APP_URL);

    const win = page.locator(".workspace-window[data-id='agent-1']");
    await expect(win).toBeVisible({ timeout: 10_000 });

    // Record every outgoing message kind from this point on.
    await page.evaluate(() => {
      window.__sent = [];
      const ws = window.__fixtureWs;
      const orig = ws.send.bind(ws);
      ws.send = (data) => {
        try {
          window.__sent.push(JSON.parse(data).kind);
        } catch (e) {}
        return orig(data);
      };
    });

    // Maximize (user action → one maximize_window is expected here).
    await win.locator(".titlebar button[data-action='maximize']").click();
    await expect(win).toHaveClass(/maximized/);

    const localWidth = await win.evaluate((el) =>
      Math.round(el.getBoundingClientRect().width),
    );
    // No inset → the maximized window fills nearly the full viewport width.
    expect(localWidth).toBeGreaterThan(1000);

    // Now a *second client* with a narrow canvas maximizes the same window to
    // width 400 and the backend broadcasts that geometry to us.
    await page.evaluate(() => {
      window.__sent = [];
      window.__injectCompetingMaximize(400);
    });
    await page.waitForTimeout(400);

    // The fix: we must NOT send a maximize_window correction (that is what
    // produced the infinite ping-pong), and our own window must stay at our
    // local fill instead of shrinking to the foreign 400px geometry.
    const sent = await page.evaluate(() => window.__sent);
    expect(sent.filter((k) => k === "maximize_window").length).toBe(0);

    const widthAfter = await win.evaluate((el) =>
      Math.round(el.getBoundingClientRect().width),
    );
    expect(Math.abs(widthAfter - localWidth)).toBeLessThan(5);
  });
});

async function installMaximizeBackend(page) {
  await page.addInitScript(() => {
    const win = {
      id: "agent-1",
      title: "Codex",
      preset: "agent",
      geometry: { x: 180, y: 120, width: 720, height: 360 },
      geometry_revision: 0,
      z_index: 1,
      status: "idle",
      minimized: false,
      maximized: false,
      pre_maximize_geometry: null,
      persist: true,
      purpose_title: null,
      dynamic_title: null,
      dynamic_title_detail: null,
      agent_id: "codex",
      agent_color: "cyan",
      tab_group_id: null,
      tab_group_active: false,
    };

    function stateMsg() {
      return {
        kind: "workspace_state",
        workspace: {
          app_version: "playwright",
          tabs: [
            {
              id: "tab-1",
              title: "Maximize Fixture",
              project_root: "/fixture",
              kind: "git",
              workspace: {
                viewport: { x: 0, y: 0, zoom: 1 },
                windows: [{ ...win }],
              },
            },
          ],
          active_tab_id: "tab-1",
          recent_projects: [],
        },
      };
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
        window.__fixtureWs = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(stateMsg());
        }, 0);
      }

      send(data) {
        try {
          const msg = JSON.parse(data);
          if (msg.kind === "maximize_window") {
            win.maximized = true;
            if (msg.bounds) win.geometry = msg.bounds;
            this.emit(stateMsg());
          } else if (msg.kind === "restore_window") {
            win.maximized = false;
            this.emit(stateMsg());
          }
        } catch (e) {}
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

    // Simulate a competing client that maximized this window at a different
    // (narrow) width; the backend broadcasts that shared geometry to us.
    window.__injectCompetingMaximize = (width) => {
      win.maximized = true;
      win.geometry = { x: 0, y: 0, width, height: 600 };
      window.__fixtureWs.emit(stateMsg());
    };

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}
