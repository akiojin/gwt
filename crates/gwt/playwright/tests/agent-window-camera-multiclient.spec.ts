import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// SPEC-2008 camera-focus / FR-095 — the canvas camera is PER-VIEWER.
//
// Replaces the old maximize ping-pong regression. Manual maximize/minimize is
// gone; "focusing" a window now flies the LOCAL camera to frame it
// (frameWindow) and the framed viewport is never broadcast to other clients.
//
// Each client adopts the persisted server viewport exactly once (its initial
// restore) and then ignores every later server viewport. So when one client
// frames a window — which, in the worst case, would echo a new viewport
// through the backend — a second client's `#canvas-stage` transform must NOT
// move. This test drives that worst case directly: after both clients boot and
// adopt the initial viewport, a competing viewport is broadcast and we assert
// the second client's stage transform is unchanged.
test.describe("Camera focus multi-client (per-viewer camera)", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("a competing client's framed viewport never moves another client's camera", async ({
    browser,
  }) => {
    // Two independent clients (separate contexts → separate WebSockets and
    // separate per-viewer camera state), same fixture backend.
    const contextA = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const contextB = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const pageA = await contextA.newPage();
    const pageB = await contextB.newPage();

    for (const page of [pageA, pageB]) {
      await installEmbeddedRoutes(page);
      await installCameraBackend(page);
      await page.goto(APP_URL);
      await expect(
        page.locator(".workspace-window[data-id='agent-1']"),
      ).toBeVisible({ timeout: 10_000 });
    }

    const stageA = pageA.locator("#canvas-stage");
    const stageB = pageB.locator("#canvas-stage");

    // Both clients adopt the persisted server viewport on boot.
    const initialB = await stageB.evaluate((el) => el.style.transform);

    // Client A frames a window locally (titlebar DOUBLE click — a single click
    // only focuses; a double click is the deliberate framing gesture). This is
    // the action that used to broadcast a maximize geometry; under FR-095 it is
    // camera local, so it must not move client B.
    const titlebarA = pageA.locator(
      ".workspace-window[data-id='agent-1'] .titlebar",
    );
    await titlebarA.dblclick();
    // Let any framing tween + (worst case) backend echo settle on both clients.
    await pageA.waitForTimeout(500);

    // Worst case: a competing client broadcasts a DIFFERENT viewport through
    // the backend. The per-viewer camera must ignore it on an already-adopted
    // client, so client B's stage transform is unchanged.
    await pageB.evaluate(() => {
      (window as any).__injectCompetingViewport({ x: -640, y: -360, zoom: 1.8 });
    });
    await pageB.waitForTimeout(300);

    const finalB = await stageB.evaluate((el) => el.style.transform);
    expect(finalB).toBe(initialB);

    // Sanity: client A did move its own camera by framing (so the assertion
    // above is about isolation, not a frozen UI).
    const finalA = await stageA.evaluate((el) => el.style.transform);
    expect(finalA).not.toBe(initialB);

    await contextA.close();
    await contextB.close();
  });
});

async function installCameraBackend(page) {
  await page.addInitScript(() => {
    const win = {
      id: "agent-1",
      title: "Codex",
      preset: "agent",
      // Offscreen-ish so framing produces a clearly different transform.
      geometry: { x: 900, y: 700, width: 720, height: 360 },
      geometry_revision: 0,
      z_index: 1,
      status: "idle",
      persist: true,
      purpose_title: null,
      dynamic_title: null,
      dynamic_title_detail: null,
      agent_id: "codex",
      agent_color: "cyan",
      tab_group_id: null,
      tab_group_active: false,
    };
    let viewport = { x: 0, y: 0, zoom: 1 };

    function stateMsg() {
      return {
        kind: "workspace_state",
        workspace: {
          app_version: "playwright",
          tabs: [
            {
              id: "tab-1",
              title: "Camera Fixture",
              project_root: "/fixture",
              kind: "git",
              workspace: {
                viewport: { ...viewport },
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

      send() {
        // The camera is per-viewer; we intentionally do not reflect any
        // outgoing viewport back into shared state (that is the whole point of
        // FR-095). focus_window etc. are accepted and ignored here.
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

    // Simulate a competing client pushing a different persisted viewport.
    window.__injectCompetingViewport = (next) => {
      viewport = next;
      window.__fixtureWs.emit(stateMsg());
    };

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}
