import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Agent title role badge", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("fallback runtime title still displays the Agent runtime badge", async ({
    page,
  }, testInfo) => {
    await installEmbeddedRoutes(page);
    await installAgentTitleBadgeBackend(page);

    await page.goto(APP_URL);

    const agentWindow = page.locator(".workspace-window[data-id='agent-1']");
    await expect(agentWindow).toBeVisible({ timeout: 10_000 });
    await expect(agentWindow.locator(".title-text")).toHaveText("Codex");

    const badge = agentWindow.locator(".window-role-badge").first();
    await expect(badge).toBeVisible();
    await expect(badge).toHaveText("Codex");

    await agentWindow.screenshot({
      path:
        process.env.GWT_AGENT_BADGE_SCREENSHOT_PATH ||
        testInfo.outputPath("agent-title-badge.png"),
    });
  });
});

async function installAgentTitleBadgeBackend(page) {
  await page.addInitScript(() => {
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Agent Badge Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
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
                },
              ],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
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
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(workspaceState);
        }, 0);
      }

      send() {}

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
  });
}
