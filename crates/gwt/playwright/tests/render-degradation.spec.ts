import { expect, test } from "@playwright/test";
import { APP_PROJECT_KEY, APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const degradationBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
const degradationAppUrl = degradationBase ? new URL(new URL(APP_URL).pathname, degradationBase).href : APP_URL;

// Issue #5595: legacy canvas geometry no longer participates in rendering.
// Missing geometry must not degrade the Works surface or freeze later state.

declare global {
  interface Window {
    __gwtRenderDegradationFixtureSocket?: { emit(payload: unknown): void };
  }
}

function fixtureWindow(id: string, x: number, geometry: unknown = undefined) {
  return {
    id,
    title: `Window ${id}`,
    session_id: `session-${id}`,
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
          project_key: APP_PROJECT_KEY,
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

      emit(payload: any) {
        if (payload.kind === "workspace_state") this.emit({ kind: "active_work_projection", projection: { id: "fixture-local-live", active_works: [], local_agent_activity: { project_key: location.pathname.split("/").filter(Boolean).at(-1), host_id: "geometry-fixture", observed_at: new Date().toISOString(), agents: [
          { window_id: 'agent-1', session_id: 'session-agent-1', runtime_incarnation: 1, is_pm: false, issue_number: null },
          { window_id: 'poison', session_id: 'session-poison', runtime_incarnation: 1, is_pm: false, issue_number: null },
          { window_id: 'agent-2', session_id: 'session-agent-2', runtime_incarnation: 1, is_pm: false, issue_number: null },
          { window_id: 'agent-3', session_id: 'session-agent-3', runtime_incarnation: 1, is_pm: false, issue_number: null }
        ] } } });
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

  test("legacy missing geometry is ignored and later workspace state still renders", async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    if (!degradationBase) await installEmbeddedRoutes(page);
    await installDegradationBackend(page);
    await page.goto(degradationAppUrl);
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
      page.locator(".work-row[data-window-id='agent-1']"),
    ).toBeVisible({ timeout: 10_000 });
    await expect(page.locator(".render-degradation-banner")).toHaveCount(0);

    // Canvas geometry is no longer a frontend dependency. Both entries must
    // mount in the fixed Works surface even when old geometry is missing.
    await page.evaluate((state) => {
      window.__gwtRenderDegradationFixtureSocket!.emit(state);
    }, workspaceState([fixtureWindow("poison", 640, null), fixtureWindow("agent-2", 1040)]));

    await expect(
      page.locator(".work-row[data-window-id='agent-2']"),
    ).toBeVisible({ timeout: 10_000 });
    const banner = page.locator(".render-degradation-banner");
    await expect(banner).toHaveCount(0);
    await expect(page.locator(".work-row[data-window-id='poison']")).toBeVisible();

    // A following workspace state still replaces the rendered agent catalog.
    await page.evaluate((state) => {
      window.__gwtRenderDegradationFixtureSocket!.emit(state);
    }, workspaceState([fixtureWindow("agent-2", 1040), fixtureWindow("agent-3", 200)]));

    await expect(
      page.locator(".work-row[data-window-id='agent-3']"),
    ).toBeVisible({ timeout: 10_000 });
    await expect(
      page.locator(".workspace-window[data-id='poison']"),
    ).toHaveCount(0);

    await expect(banner).toHaveCount(0);
    expect(errors).toEqual([]);
  });
});
