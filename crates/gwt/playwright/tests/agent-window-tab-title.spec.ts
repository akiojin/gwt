import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// A tabbed window tab truncates its label at max-width, so hovering must reveal
// the full title via the native `title` tooltip. The tooltip text mirrors the
// titlebar/window-list helper `windowTitleTooltip`: it prefers
// `dynamic_title_detail`, otherwise falls back to the display title.
test.describe("Agent window tab hover title", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("tabbed window tabs expose the full title as a native tooltip", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installTabbedAgentsBackend(page);

    await page.goto(APP_URL);

    const activeWindow = page.locator(".workspace-window[data-id='agent-1']");
    await expect(activeWindow).toBeVisible({ timeout: 10_000 });
    await expect(activeWindow).toHaveClass(/tabbed/);

    const strip = activeWindow.locator(".window-tab-strip");
    const detailTab = strip.locator(".window-tab[data-window-tab-id='agent-1']");
    const fallbackTab = strip.locator(
      ".window-tab[data-window-tab-id='agent-2']",
    );

    // agent-1 carries a dynamic detail, so the tooltip shows that detail.
    await expect(detailTab).toHaveAttribute(
      "title",
      "Codex · implementing complete maximize",
    );
    // agent-2 has no dynamic detail, so the tooltip falls back to the title.
    await expect(fallbackTab).toHaveAttribute("title", "Claude");
  });
});

async function installTabbedAgentsBackend(page) {
  await page.addInitScript(() => {
    const baseWindow = {
      preset: "agent",
      geometry: { x: 180, y: 120, width: 720, height: 360 },
      geometry_revision: 0,
      status: "idle",
      minimized: false,
      maximized: false,
      pre_maximize_geometry: null,
      persist: true,
      purpose_title: null,
      dynamic_title: null,
      dynamic_title_detail: null,
      tab_group_id: "grp-1",
    };

    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Tab Title Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  ...baseWindow,
                  id: "agent-1",
                  title: "Codex",
                  z_index: 2,
                  agent_id: "codex",
                  agent_color: "cyan",
                  dynamic_title_detail: "Codex · implementing complete maximize",
                  tab_group_active: true,
                },
                {
                  ...baseWindow,
                  id: "agent-2",
                  title: "Claude",
                  z_index: 1,
                  agent_id: "claude",
                  agent_color: "violet",
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
