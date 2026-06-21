import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// SPEC-2008 camera-focus / FR-094 — Fleet Minimap + camera.
//
// The always-on minimap (`#fleet-minimap`) shows one cell per canvas window in
// world position and overlays the camera viewport as a `.fleet-minimap__camera`
// frame. Clicking a cell flies the LOCAL camera to that window (frameWindow),
// which moves the `#canvas-stage` transform and marks the cell `.is-focused`.
// Panning / zooming the camera repositions the camera frame.
test.describe("Fleet Minimap + camera", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("minimap shows one cell per window and clicking a cell frames it", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installMinimapBackend(page);
    await page.goto(APP_URL);

    const minimap = page.locator("#fleet-minimap");
    await expect(minimap).toBeVisible({ timeout: 10_000 });
    await expect(minimap).toHaveAttribute("data-empty", "false");

    // One cell per window (two windows in the fixture).
    const cells = minimap.locator(".fleet-minimap__cell");
    await expect(cells).toHaveCount(2);
    await expect(minimap.locator('.fleet-minimap__cell[data-window-id="agent-1"]')).toHaveCount(1);
    await expect(minimap.locator('.fleet-minimap__cell[data-window-id="agent-2"]')).toHaveCount(1);

    const stage = page.locator("#canvas-stage");
    const before = await stage.evaluate((el) => el.style.transform);

    // Clicking the far window's cell flies the camera (changes the transform)
    // and marks that cell focused.
    const targetCell = minimap.locator('.fleet-minimap__cell[data-window-id="agent-2"]');
    await targetCell.click();
    await page.waitForTimeout(500); // let the framing tween settle.

    await expect
      .poll(() => stage.evaluate((el) => el.style.transform))
      .not.toBe(before);
    await expect(targetCell).toHaveClass(/is-focused/);
  });

  test("the camera frame moves after the camera pans/zooms", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installMinimapBackend(page);
    await page.goto(APP_URL);

    const minimap = page.locator("#fleet-minimap");
    await expect(minimap).toBeVisible({ timeout: 10_000 });

    const cameraFrame = minimap.locator(".fleet-minimap__camera");
    await expect(cameraFrame).toBeVisible();
    const before = await cameraFrame.evaluate((el) => ({
      left: el.style.left,
      top: el.style.top,
      width: el.style.width,
    }));

    // Pan + zoom the camera by framing the far window.
    await minimap
      .locator('.fleet-minimap__cell[data-window-id="agent-2"]')
      .click();
    await page.waitForTimeout(500);

    await expect
      .poll(() =>
        cameraFrame.evaluate((el) => ({
          left: el.style.left,
          top: el.style.top,
          width: el.style.width,
        })),
      )
      .not.toEqual(before);
  });
});

async function installMinimapBackend(page) {
  await page.addInitScript(() => {
    function agentWindow(id, x, y, z) {
      return {
        id,
        title: id,
        preset: "agent",
        geometry: { x, y, width: 480, height: 320 },
        geometry_revision: 0,
        z_index: z,
        status: "idle",
        persist: true,
        purpose_title: null,
        dynamic_title: null,
        dynamic_title_detail: null,
        agent_id: id,
        agent_color: "cyan",
        tab_group_id: null,
        tab_group_active: false,
      };
    }

    // Live window set the fixture mutates on focus (z-order raise), mirroring
    // the real backend so the minimap's `is-focused` cell update — which is
    // driven by the workspace render path — has a state echo to react to.
    let zCounter = 2;
    const windows = [
      agentWindow("agent-1", 120, 100, 1),
      // Far away so framing it clearly moves the camera + frame.
      agentWindow("agent-2", 1600, 1200, 2),
    ];

    function stateMsg() {
      return {
        kind: "workspace_state",
        workspace: {
          app_version: "playwright",
          tabs: [
            {
              id: "tab-1",
              title: "Minimap Fixture",
              project_root: "/fixture",
              kind: "git",
              workspace: {
                viewport: { x: 0, y: 0, zoom: 1 },
                windows: windows.map((w) => ({ ...w })),
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
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(stateMsg());
        }, 0);
      }

      send(data) {
        // The real backend raises z-order on focus_window and re-broadcasts the
        // workspace, which re-runs the render path (and the minimap renderCells
        // that applies `is-focused`). The camera stays local (no viewport echo).
        let msg;
        try {
          msg = JSON.parse(data);
        } catch {
          return;
        }
        if (msg.kind === "focus_window") {
          const target = windows.find((w) => w.id === msg.id);
          if (target) {
            zCounter += 1;
            target.z_index = zCounter;
            this.emit(stateMsg());
          }
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
