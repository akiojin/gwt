import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// SPEC-2008 camera-focus / FR-094 — Fleet Minimap (centered-radar model).
//
// The always-on minimap (`#fleet-minimap`) mirrors the canvas frame of
// reference: the camera viewport is FIXED at the minimap centre and the world
// (window cells, inside `.fleet-minimap__world`) MOVES under it as the camera
// pans — like `#canvas-stage`. The cyan `.fleet-minimap__camera` frame stays
// centred; its size reflects the canvas zoom. The minimap has its own zoom
// (`.fleet-minimap__zoom-button`). Clicking a cell flies the LOCAL camera to
// that window (frameWindow), translating the world layer and marking the cell
// `.is-focused`. The fixture centres the camera on the fleet so both windows
// are visible in the radar.
test.describe("Fleet Minimap centered radar", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("minimap shows one cell per window inside the world layer; clicking frames", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installMinimapBackend(page);
    await page.goto(APP_URL);

    const minimap = page.locator("#fleet-minimap");
    await expect(minimap).toBeVisible({ timeout: 10_000 });
    await expect(minimap).toHaveAttribute("data-empty", "false");

    // Cells live inside the single world layer (so a pan translates them as one).
    await expect(minimap.locator(".fleet-minimap__world")).toHaveCount(1);
    const cells = minimap.locator(".fleet-minimap__world .fleet-minimap__cell");
    await expect(cells).toHaveCount(2);
    await expect(minimap.locator('.fleet-minimap__cell[data-window-id="agent-1"]')).toHaveCount(1);
    await expect(minimap.locator('.fleet-minimap__cell[data-window-id="agent-2"]')).toHaveCount(1);

    const stage = page.locator("#canvas-stage");
    const before = await stage.evaluate((el) => el.style.transform);

    // Clicking a window's cell flies the camera (changes the stage transform)
    // and marks that cell focused.
    const targetCell = minimap.locator('.fleet-minimap__cell[data-window-id="agent-2"]');
    await targetCell.click();
    await page.waitForTimeout(500); // let the framing tween settle.

    await expect
      .poll(() => stage.evaluate((el) => el.style.transform))
      .not.toBe(before);
    await expect(targetCell).toHaveClass(/is-focused/);
  });

  test("panning translates the world layer while the camera frame stays centred", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installMinimapBackend(page);
    await page.goto(APP_URL);

    const minimap = page.locator("#fleet-minimap");
    await expect(minimap).toBeVisible({ timeout: 10_000 });
    const world = minimap.locator(".fleet-minimap__world");
    const cameraFrame = minimap.locator(".fleet-minimap__camera");
    await expect(cameraFrame).toBeVisible();

    const worldBefore = await world.evaluate((el) => el.style.transform);

    // Frame the far window: the camera pans/zooms, so the world layer translates
    // (windows move under the fixed radar centre).
    await minimap.locator('.fleet-minimap__cell[data-window-id="agent-2"]').click();
    await page.waitForTimeout(500);

    await expect
      .poll(() => world.evaluate((el) => el.style.transform))
      .not.toBe(worldBefore);

    // The camera frame remains centred in the minimap (viewport fixed): its
    // centre stays at the minimap centre within a small tolerance.
    const centered = await page.evaluate(() => {
      const map = document.getElementById("fleet-minimap");
      const frame = map?.querySelector(".fleet-minimap__camera");
      if (!map || !frame) return false;
      const m = map.getBoundingClientRect();
      const f = frame.getBoundingClientRect();
      const dx = Math.abs((f.left + f.width / 2) - (m.left + m.width / 2));
      const dy = Math.abs((f.top + f.height / 2) - (m.top + m.height / 2));
      return dx < 2 && dy < 2;
    });
    expect(centered).toBe(true);
  });

  test("radar zoom controls rescale the cells", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installMinimapBackend(page);
    await page.goto(APP_URL);

    const minimap = page.locator("#fleet-minimap");
    await expect(minimap).toBeVisible({ timeout: 10_000 });
    const cell = minimap.locator('.fleet-minimap__cell[data-window-id="agent-1"]');
    await expect(cell).toHaveCount(1);

    const widthBefore = await cell.evaluate((el) => parseFloat(el.style.width));
    // The "+" zoom button widens the radar (cells grow).
    await minimap.locator(".fleet-minimap__zoom-button").first().click();
    await expect
      .poll(() => cell.evaluate((el) => parseFloat(el.style.width)))
      .toBeGreaterThan(widthBefore);
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

    let zCounter = 2;
    const windows = [
      agentWindow("agent-1", 120, 100, 1),
      // Far away so framing it clearly translates the world layer.
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
                // Centre the camera on the fleet bounding box (~1100, 810) so the
                // centered radar shows BOTH windows (a far window centred on the
                // camera would otherwise clip off the radar edge).
                viewport: { x: -380, y: -360, zoom: 1 },
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
