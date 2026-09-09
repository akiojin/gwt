import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// Issue #3365 — renderWorkspace exception safety.
//
// A window whose `geometry` is null passes `workspaceWindowsRenderKey`
// (which guards with `windowData?.geometry || {}`) but throws inside
// `ensureWindow` when the update path dereferences `windowData.geometry`.
// Before the fix, that exception escaped `renderWorkspace` AFTER the render
// key was already committed, so every following identical workspace_state
// short-circuited on the key diff: the Fleet Minimap, the window list, and
// the telemetry counts froze until reload, and the only trace was a console
// warn from the WebSocket dispatcher.
//
// The spec drives the real frontend (embedded routes + fixture WebSocket)
// through poison -> recovery:
// - the poisoned window is isolated (a healthy window in the same state
//   still mounts),
// - the degradation banner becomes visible instead of a console-only warn,
// - the NEXT workspace_state still renders windows and minimap cells (the
//   render key was not committed by the failed sync).

declare global {
  interface Window {
    __gwtRenderDegradationFixtureSocket?: { emit(payload: unknown): void };
  }
}

function fixtureWindow(id: string, x: number, geometry: unknown = undefined) {
  return {
    id,
    title: `Window ${id}`,
    preset: "agent",
    geometry: geometry === undefined ? { x, y: 160, width: 420, height: 300 } : geometry,
    geometry_revision: 0,
    z_index: 1,
    status: "running",
    persist: true,
    purpose_title: null,
    dynamic_title: null,
    dynamic_title_detail: null,
    agent_id: id,
    agent_color: null,
    tab_group_id: null,
    tab_group_active: false,
  };
}

function workspaceState(windows: unknown[]) {
  return {
    kind: "workspace_state",
    workspace: {
      app_version: "playwright",
      tabs: [
        {
          id: "tab-1",
          title: "Degradation Fixture",
          project_root: "/fixture",
          kind: "git",
          workspace: {
            viewport: { x: 0, y: 0, zoom: 1 },
            windows,
          },
        },
      ],
      active_tab_id: "tab-1",
      recent_projects: [],
    },
  };
}

async function installDegradationBackend(page: any) {
  await page.addInitScript(() => {
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url: string) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        (window as any).__gwtRenderDegradationFixtureSocket = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send() {}

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: unknown) {
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
  });
}

test.describe("Render degradation isolation and recovery", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("a poisoned window degrades visibly and the next workspace_state still renders", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installDegradationBackend(page);
    await page.goto(APP_URL);
    await page.waitForFunction(
      () => Boolean(window.__gwtRenderDegradationFixtureSocket),
      undefined,
      { timeout: 10_000 },
    );

    // Boot with one healthy window.
    await page.evaluate((state) => {
      window.__gwtRenderDegradationFixtureSocket!.emit(state);
    }, workspaceState([fixtureWindow("agent-1", 200)]));
    await expect(
      page.locator(".workspace-window[data-id='agent-1']"),
    ).toBeVisible({ timeout: 10_000 });
    await expect(
      page.locator(".fleet-minimap__cell[data-window-id='agent-1']"),
    ).toBeAttached();
    await expect(page.locator(".render-degradation-banner")).toHaveCount(0);

    // Poisoned state: geometry:null throws inside ensureWindow. The healthy
    // window listed AFTER the poison must still mount (per-window isolation)
    // and the degradation banner must become visible.
    await page.evaluate((state) => {
      window.__gwtRenderDegradationFixtureSocket!.emit(state);
    }, workspaceState([fixtureWindow("poison", 640, null), fixtureWindow("agent-2", 1040)]));

    await expect(
      page.locator(".workspace-window[data-id='agent-2']"),
    ).toBeVisible({ timeout: 10_000 });
    const banner = page.locator(".render-degradation-banner");
    await expect(banner).toBeVisible();
    await expect(banner).toContainText(/failed to render/);
    await expect(
      page.locator(".fleet-minimap__cell[data-window-id='agent-2']"),
    ).toBeAttached();

    // Recovery: the failed sync must NOT have committed the render key, so
    // the next workspace_state re-syncs — new windows and minimap cells keep
    // appearing instead of freezing until reload.
    await page.evaluate((state) => {
      window.__gwtRenderDegradationFixtureSocket!.emit(state);
    }, workspaceState([fixtureWindow("agent-2", 1040), fixtureWindow("agent-3", 200)]));

    await expect(
      page.locator(".workspace-window[data-id='agent-3']"),
    ).toBeVisible({ timeout: 10_000 });
    await expect(
      page.locator(".fleet-minimap__cell[data-window-id='agent-3']"),
    ).toBeAttached();
    await expect(
      page.locator(".workspace-window[data-id='poison']"),
    ).toHaveCount(0);

    // The banner persists until dismissed.
    await expect(banner).toBeVisible();
    await banner.locator(".render-degradation-banner__dismiss").click();
    await expect(page.locator(".render-degradation-banner")).toHaveCount(0);
  });
});
