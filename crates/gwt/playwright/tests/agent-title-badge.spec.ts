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

  test("Intake and Execution lane badges render across agent chrome", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installAgentTitleBadgeBackend(page);

    await page.goto(APP_URL);

    const executionWindow = page.locator(".workspace-window[data-id='agent-1']");
    const intakeWindow = page.locator(".workspace-window[data-id='agent-2']");
    await expect(executionWindow).toBeVisible({ timeout: 10_000 });
    await expect(intakeWindow).toBeVisible();

    const executionBadge = executionWindow.locator(".window-lane-badge");
    await expect(executionBadge).toBeVisible();
    await expect(executionBadge).toHaveText("Execution");
    await expect(executionBadge).toHaveAttribute("data-lane-kind", "execution");
    await expect(executionBadge).toHaveAttribute("aria-label", "Execution lane");

    const intakeBadge = intakeWindow.locator(".window-lane-badge");
    await expect(intakeBadge).toBeVisible();
    await expect(intakeBadge).toHaveText("Intake");
    await expect(intakeBadge).toHaveAttribute("data-lane-kind", "intake");
    await expect(intakeBadge).toHaveAttribute("aria-label", "Intake lane");

    const minimap = page.locator("#fleet-minimap");
    await expect(minimap).toBeVisible();
    await expect(
      minimap.locator('.fleet-minimap__cell[data-window-id="agent-1"]'),
    ).toHaveAttribute("data-lane-symbol", "E");
    await expect(
      minimap.locator('.fleet-minimap__cell[data-window-id="agent-2"]'),
    ).toHaveAttribute("data-lane-symbol", "I");

    await page.locator("#window-list-button").click();
    const panel = page.locator("#window-list-panel");
    await expect(panel).toBeVisible();
    await expect(
      panel
        .locator(".window-list-row", { hasText: "Codex" })
        .first()
        .locator(".window-list-lane"),
    ).toHaveText("Execution");
    await expect(
      panel
        .locator(".window-list-row", { hasText: "Intake Agent" })
        .first()
        .locator(".window-list-lane"),
    ).toHaveText("Intake");
  });
});

async function installAgentTitleBadgeBackend(page) {
  await page.addInitScript(() => {
    const windows = [
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
        lane_kind: "execution",
        tab_group_id: null,
        tab_group_active: false,
      },
      {
        id: "agent-2",
        title: "Intake Agent",
        preset: "agent",
        geometry: { x: 940, y: 120, width: 600, height: 320 },
        geometry_revision: 0,
        z_index: 2,
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
        lane_kind: "intake",
        tab_group_id: null,
        tab_group_active: false,
      },
    ];
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
              windows: windows.map((windowData) => ({ ...windowData })),
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };
    const windowListState = {
      kind: "window_list",
      windows: windows.map((windowData) => ({ ...windowData })),
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

      send(data) {
        let msg;
        try {
          msg = JSON.parse(data);
        } catch {
          return;
        }
        if (msg.kind === "list_windows") {
          this.emit(windowListState);
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
  });
}
