import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Window tab group close", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("titlebar close confirms and closes every grouped tab", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installWindowGroupBackend(page);
    await page.goto(APP_URL);

    const groupWindow = page.locator(".workspace-window[data-id='claude-1']");
    await expect(groupWindow).toBeVisible();
    await expect(groupWindow.locator(".window-tab")).toHaveCount(2);

    await groupWindow.locator("[data-action='close']").click();

    const modal = page.locator("#close-window-group-modal");
    await expect(modal).toHaveClass(/open/);
    await expect(modal.locator(".close-project-tab-modal__title")).toHaveText(
      "Close window?",
    );
    await expect(modal.locator(".close-project-tab-modal__subtitle")).toHaveText(
      "2 tabs will be closed.",
    );

    await modal.locator(".close-project-tab-modal__confirm").click();

    await expect(modal).not.toHaveClass(/open/);
    await expect(page.locator(".workspace-window[data-id='claude-1']")).toHaveCount(0);
    await expect(page.locator(".workspace-window[data-id='codex-1']")).toHaveCount(0);
    await expect(page.locator(".workspace-window[data-id='shell-1']")).toBeVisible();
    await expect
      .poll(() => latestSentKind(page))
      .toBe("close_window_group");
  });

  test("tab strip close keeps the remaining grouped tab open", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installWindowGroupBackend(page);
    await page.goto(APP_URL);

    const groupWindow = page.locator(".workspace-window[data-id='claude-1']");
    await expect(groupWindow).toBeVisible();
    await expect(groupWindow.locator(".window-tab")).toHaveCount(2);

    await groupWindow.locator(".window-tab-item").filter({ hasText: "Codex" })
      .locator(".window-tab-close")
      .click();

    await expect(page.locator("#close-window-group-modal")).not.toHaveClass(/open/);
    await expect(page.locator(".workspace-window[data-id='claude-1']")).toBeVisible();
    await expect(page.locator(".workspace-window[data-id='codex-1']")).toHaveCount(0);
    await expect(groupWindow.locator(".window-tab")).toHaveCount(1);
    await expect
      .poll(() => latestSentKind(page))
      .toBe("close_window");
  });
});

async function latestSentKind(page) {
  return page.evaluate(() => {
    const sends = (window as any).__gwtRecordedSends || [];
    return sends.at(-1)?.kind || null;
  });
}

async function installWindowGroupBackend(page) {
  await page.addInitScript(() => {
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Fixture Project",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                windowFixture({
                  id: "claude-1",
                  title: "Claude",
                  preset: "claude",
                  x: 140,
                  y: 120,
                  zIndex: 3,
                  tabGroupId: "group-agent",
                  tabGroupActive: true,
                  agentId: "claude",
                }),
                windowFixture({
                  id: "codex-1",
                  title: "Codex",
                  preset: "codex",
                  x: 140,
                  y: 120,
                  zIndex: 3,
                  tabGroupId: "group-agent",
                  tabGroupActive: false,
                  agentId: "codex",
                }),
                windowFixture({
                  id: "shell-1",
                  title: "Shell",
                  preset: "shell",
                  x: 720,
                  y: 120,
                  zIndex: 2,
                }),
              ],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };

    function activeWindows() {
      return workspaceState.workspace.tabs[0].workspace.windows;
    }

    function emit(socket, payload) {
      setTimeout(() => {
        socket.dispatchEvent(
          new MessageEvent("message", { data: JSON.stringify(payload) }),
        );
      }, 0);
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
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw) {
        let message;
        try {
          message = JSON.parse(raw);
        } catch {
          return;
        }
        window.__gwtRecordedSends = window.__gwtRecordedSends || [];
        window.__gwtRecordedSends.push(message);
        if (message.kind === "frontend_ready") {
          emit(this, workspaceState);
          return;
        }
        if (message.kind === "close_window_group") {
          closeWindowGroup(message.id);
          emit(this, workspaceState);
          return;
        }
        if (message.kind === "close_window") {
          closeWindow(message.id);
          emit(this, workspaceState);
        }
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }
    }

    function closeWindow(id) {
      const windows = activeWindows();
      const removed = windows.find((windowData) => windowData.id === id);
      workspaceState.workspace.tabs[0].workspace.windows = windows.filter(
        (windowData) => windowData.id !== id,
      );
      if (removed?.tab_group_active && removed.tab_group_id) {
        const next = activeWindows().find(
          (windowData) => windowData.tab_group_id === removed.tab_group_id,
        );
        if (next) next.tab_group_active = true;
      }
    }

    function closeWindowGroup(id) {
      const windows = activeWindows();
      const target = windows.find((windowData) => windowData.id === id);
      if (!target) return;
      const groupId = target.tab_group_id || target.id;
      workspaceState.workspace.tabs[0].workspace.windows = windows.filter(
        (windowData) => (windowData.tab_group_id || windowData.id) !== groupId,
      );
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });

    function windowFixture({
      id,
      title,
      preset,
      x,
      y,
      zIndex,
      tabGroupId = null,
      tabGroupActive = false,
      agentId = null,
    }) {
      return {
        id,
        title,
        preset,
        geometry: { x, y, width: 520, height: 300 },
        geometry_revision: 0,
        z_index: zIndex,
        status: "running",
        minimized: false,
        maximized: false,
        pre_maximize_geometry: null,
        persist: true,
        purpose_title: null,
        dynamic_title: null,
        dynamic_title_detail: null,
        agent_id: agentId,
        agent_color: null,
        tab_group_id: tabGroupId,
        tab_group_active: tabGroupActive,
      };
    }
  });
}
