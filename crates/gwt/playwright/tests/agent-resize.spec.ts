import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Agent window resize tracking", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("pointerup coordinates decide the final resize geometry", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installAgentWindowBackend(page);
    await page.goto(APP_URL);

    const windowFrame = page.locator(".workspace-window[data-id='agent-1']");
    const resizeHandle = windowFrame.locator(".resize-handle");
    await expect(windowFrame).toBeVisible();
    await expect(resizeHandle).toBeVisible();

    const box = await windowFrame.boundingBox();
    expect(box).not.toBeNull();
    const startX = Math.round(box!.x + box!.width - 3);
    const startY = Math.round(box!.y + box!.height - 3);

    await resizeHandle.dispatchEvent("pointerdown", {
      pointerId: 23,
      pointerType: "mouse",
      button: 0,
      buttons: 1,
      clientX: startX,
      clientY: startY,
    });
    await page.evaluate(
      ({ x, y }) => {
        window.dispatchEvent(
          new PointerEvent("pointermove", {
            pointerId: 23,
            pointerType: "mouse",
            buttons: 1,
            clientX: x,
            clientY: y,
            bubbles: true,
          }),
        );
      },
      { x: startX + 20, y: startY + 20 },
    );
    await page.evaluate(
      ({ x, y }) => {
        window.dispatchEvent(
          new PointerEvent("pointerup", {
            pointerId: 23,
            pointerType: "mouse",
            button: 0,
            buttons: 0,
            clientX: x,
            clientY: y,
            bubbles: true,
          }),
        );
      },
      { x: startX + 120, y: startY + 80 },
    );

    await expect
      .poll(() => windowFrame.evaluate((element) => ({
        width: element.style.width,
        height: element.style.height,
      })))
      .toEqual({ width: "640px", height: "380px" });
  });
});

async function installAgentWindowBackend(page) {
  await page.addInitScript(() => {
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Resize Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  id: "agent-1",
                  title: "Agent",
                  preset: "agent",
                  geometry: { x: 140, y: 100, width: 520, height: 300 },
                  geometry_revision: 0,
                  z_index: 1,
                  status: "running",
                  minimized: false,
                  maximized: false,
                  pre_maximize_geometry: null,
                  persist: true,
                  purpose_title: null,
                  dynamic_title: null,
                  dynamic_title_detail: null,
                  agent_id: "agent-1",
                  agent_color: null,
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
        this.recordedSends = [];
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(workspaceState);
        }, 0);
      }

      send(data) {
        this.recordedSends.push(JSON.parse(data));
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
