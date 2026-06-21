import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// SPEC-2008 camera-focus: the manual zoom-out floor was lowered from 0.6 to
// 0.15 (VIEWPORT_ZOOM_MIN, applied by zoomCanvasAt). A user can now pull the
// canvas much further out so a crowded fleet fits on screen. The zoom-in upper
// bound (VIEWPORT_ZOOM_MAX = 2.4) is unchanged.
//
// `applyViewport()` writes `#canvas-stage` as
//   transform: translate(<x>px, <y>px) scale(<zoom>)
// so we read the inline transform and parse the scale factor.
test.describe("Canvas manual zoom envelope", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("zoom-out can pull the canvas below the old 0.6 floor toward 0.15", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installCanvasBackend(page);
    await page.goto(APP_URL);

    await expect(
      page.locator(".workspace-window[data-id='agent-1']"),
    ).toBeVisible({ timeout: 10_000 });

    const stage = page.locator("#canvas-stage");
    const zoomOut = page.locator("#zoom-out-button");
    await expect(zoomOut).toBeVisible();

    // Boot zoom is 1.0.
    expect(await stageScale(stage)).toBeCloseTo(1, 2);

    // Each click multiplies zoom by 0.9. 0.9^5 ≈ 0.59 already dips under the old
    // 0.6 floor; keep clicking well past that to drive toward the 0.15 floor.
    for (let i = 0; i < 30; i += 1) {
      await zoomOut.click();
    }

    const scale = await stageScale(stage);
    // Must NOT be stuck at the retired 0.6 floor.
    expect(scale).toBeLessThan(0.6);
    // Must clamp at the new 0.15 floor (never below it).
    expect(scale).toBeGreaterThanOrEqual(0.15 - 1e-6);
    expect(scale).toBeCloseTo(0.15, 2);
  });

  test("zoom-in upper bound stays at 2.4", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installCanvasBackend(page);
    await page.goto(APP_URL);

    await expect(
      page.locator(".workspace-window[data-id='agent-1']"),
    ).toBeVisible({ timeout: 10_000 });

    const stage = page.locator("#canvas-stage");
    const zoomIn = page.locator("#zoom-in-button");
    await expect(zoomIn).toBeVisible();

    // Each click multiplies zoom by 1.1; clamp at 2.4. ~10 clicks (1.1^10 ≈ 2.6)
    // already exceeds the ceiling, so click well past it to prove the clamp.
    for (let i = 0; i < 25; i += 1) {
      await zoomIn.click();
    }

    const scale = await stageScale(stage);
    expect(scale).toBeCloseTo(2.4, 2);
    expect(scale).toBeLessThanOrEqual(2.4 + 1e-6);
  });
});

async function stageScale(stage: any): Promise<number> {
  return stage.evaluate((el: HTMLElement) => {
    const match = /scale\(([-0-9.]+)\)/.exec(el.style.transform || "");
    return match ? Number(match[1]) : NaN;
  });
}

async function installCanvasBackend(page: any) {
  await page.addInitScript(() => {
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Zoom Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  id: "agent-1",
                  title: "Agent",
                  preset: "agent",
                  geometry: { x: 200, y: 160, width: 520, height: 320 },
                  geometry_revision: 0,
                  z_index: 1,
                  status: "running",
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
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(workspaceState);
        }, 0);
      }

      // The camera is per-viewer; outgoing viewport persists are not reflected
      // back, so the fixture simply accepts and drops every send.
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
