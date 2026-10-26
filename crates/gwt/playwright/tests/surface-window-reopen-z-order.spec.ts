import { expect, test } from "@playwright/test";
import {
  APP_URL,
  installEmbeddedRoutes,
} from "./_helpers/embedded-frontend";

test.describe("surface reopen", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("reopening an existing surface selects its fixed pane before backend focus ack", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    const board = page.locator('.workspace-window[data-preset="board"]');
    const index = page.locator('.workspace-window[data-preset="index"]');
    await expect(page.locator("#split-surfaces")).toBeVisible();
    await expect(page.locator("#canvas, #canvas-stage, #fleet-minimap")).toHaveCount(0);

    await page.locator('.op-rail__surface[data-surface="board"]').click();

    await expect(board).toHaveClass(/focused/);
    await expect(page.getByRole("region", { name: "Main pane", exact: true })).toHaveAttribute("data-surface", "board");
    await expect(board).toBeVisible();
    await expect(index).toBeHidden();

    await expect
      .poll(async () => {
        return page.evaluate(() => (window as any).__focusRequests ?? []);
      })
      .toEqual(["tab-1::board-1"]);
  });
});

async function installWorkspaceFixture(page: any): Promise<void> {
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
                {
                  id: "tab-1::board-1",
                  title: "Board",
                  preset: "board",
                  status: "running",
                  geometry: { x: 160, y: 120, width: 720, height: 420 },
                  z_index: 1,
                  placement: { kind: "canvas" },
                },
                {
                  id: "tab-1::index-1",
                  title: "Index",
                  preset: "index",
                  status: "running",
                  geometry: { x: 220, y: 170, width: 720, height: 420 },
                  z_index: 2,
                  placement: { kind: "canvas" },
                },
              ],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };

    (window as any).__focusRequests = [];

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url: string) {
        super();
        (this as any).url = url;
        (this as any).readyState = FixtureWebSocket.CONNECTING;
        setTimeout(() => {
          (this as any).readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw: string): void {
        let message: any;
        try {
          message = JSON.parse(raw);
        } catch {
          return;
        }
        if (message.kind === "frontend_ready") {
          (this as any).emit(workspaceState);
          return;
        }
        if (message.kind === "focus_window") {
          (window as any).__focusRequests.push(message.id);
        }
      }

      close(): void {
        (this as any).readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: any): void {
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
