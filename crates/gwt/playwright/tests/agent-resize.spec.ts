import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// SPEC-2008 camera-focus (UX amendment 2026-06-20):
// - The manual resize handle is RESTORED: dragging `.resize-handle` changes the
//   window's inline width/height (min 420x260) and never moves the camera.
// - A SINGLE click (titlebar / body / terminal) only focuses the window
//   (`.focused`) and never moves the camera (`#canvas-stage` transform stays).
// - A DOUBLE click on the titlebar (<=300ms) is the deliberate "frame this
//   window" gesture and DOES move the camera (transform changes).
test.describe("Window controls: resize handle + click semantics", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("the resize handle exists, drags to resize the window, and never moves the camera", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installAgentWindowBackend(page);
    await page.goto(APP_URL);

    const windowFrame = page.locator(".workspace-window[data-id='agent-1']");
    const resizeHandle = windowFrame.locator(".resize-handle");
    await expect(windowFrame).toBeVisible();
    await expect(resizeHandle).toBeVisible();

    const stage = page.locator("#canvas-stage");
    const cameraBefore = await stage.evaluate((el) => el.style.transform);

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
      { x: startX + 120, y: startY + 80 },
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

    // Fixture window starts 520x300; dragging +120/+80 at zoom 1 grows it.
    await expect
      .poll(() =>
        windowFrame.evaluate((element) => ({
          width: element.style.width,
          height: element.style.height,
        })),
      )
      .toEqual({ width: "640px", height: "380px" });

    // Resizing must NOT move the camera.
    expect(await stage.evaluate((el) => el.style.transform)).toBe(cameraBefore);
  });

  test("the resize handle honors the 420x260 minimum floor", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installAgentWindowBackend(page);
    await page.goto(APP_URL);

    const windowFrame = page.locator(".workspace-window[data-id='agent-1']");
    const resizeHandle = windowFrame.locator(".resize-handle");
    await expect(resizeHandle).toBeVisible();

    const box = await windowFrame.boundingBox();
    const startX = Math.round(box!.x + box!.width - 3);
    const startY = Math.round(box!.y + box!.height - 3);

    await resizeHandle.dispatchEvent("pointerdown", {
      pointerId: 24,
      pointerType: "mouse",
      button: 0,
      buttons: 1,
      clientX: startX,
      clientY: startY,
    });
    // Drag far up-left, well past the minimum, to clamp at the floor.
    for (const [dx, dy] of [
      [-400, -300],
      [-800, -600],
    ]) {
      await page.evaluate(
        ({ x, y }) => {
          window.dispatchEvent(
            new PointerEvent("pointermove", {
              pointerId: 24,
              pointerType: "mouse",
              buttons: 1,
              clientX: x,
              clientY: y,
              bubbles: true,
            }),
          );
        },
        { x: startX + dx, y: startY + dy },
      );
    }
    await page.evaluate(
      ({ x, y }) => {
        window.dispatchEvent(
          new PointerEvent("pointerup", {
            pointerId: 24,
            pointerType: "mouse",
            button: 0,
            buttons: 0,
            clientX: x,
            clientY: y,
            bubbles: true,
          }),
        );
      },
      { x: startX - 800, y: startY - 600 },
    );

    await expect
      .poll(() =>
        windowFrame.evaluate((element) => ({
          width: element.style.width,
          height: element.style.height,
        })),
      )
      .toEqual({ width: "420px", height: "260px" });
  });

  test("a single titlebar click focuses the window without moving the camera", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installAgentWindowBackend(page);
    await page.goto(APP_URL);

    const windowFrame = page.locator(".workspace-window[data-id='agent-1']");
    const titlebar = windowFrame.locator(".titlebar");
    await expect(windowFrame).toBeVisible();

    const stage = page.locator("#canvas-stage");
    const cameraBefore = await stage.evaluate((el) => el.style.transform);

    await titlebar.click();
    // The focus class lands immediately on the first click.
    await expect(windowFrame).toHaveClass(/focused/);
    // A short settle window to prove the camera does NOT animate on a single
    // click (framing would otherwise tween the transform here).
    await page.waitForTimeout(450);
    expect(await stage.evaluate((el) => el.style.transform)).toBe(cameraBefore);
  });

  test("a single body click focuses the window without moving the camera", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installAgentWindowBackend(page);
    await page.goto(APP_URL);

    const windowFrame = page.locator(".workspace-window[data-id='agent-1']");
    const body = windowFrame.locator(".window-body");
    await expect(windowFrame).toBeVisible();

    const stage = page.locator("#canvas-stage");
    const cameraBefore = await stage.evaluate((el) => el.style.transform);

    await body.click({ position: { x: 10, y: 10 } });
    await expect(windowFrame).toHaveClass(/focused/);
    await page.waitForTimeout(450);
    expect(await stage.evaluate((el) => el.style.transform)).toBe(cameraBefore);
  });

  test("a double titlebar click frames the window (moves the camera)", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installAgentWindowBackend(page);
    await page.goto(APP_URL);

    const windowFrame = page.locator(".workspace-window[data-id='agent-1']");
    const titlebar = windowFrame.locator(".titlebar");
    await expect(windowFrame).toBeVisible();

    const stage = page.locator("#canvas-stage");
    const cameraBefore = await stage.evaluate((el) => el.style.transform);

    // Two clicks within the 300ms threshold upgrade to a framing gesture.
    await titlebar.dblclick();
    await page.waitForTimeout(500); // let the framing tween settle.

    await expect
      .poll(() => stage.evaluate((el) => el.style.transform))
      .not.toBe(cameraBefore);
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
                  // Offscreen-ish so a double-click frame visibly moves the camera.
                  geometry: { x: 900, y: 700, width: 520, height: 300 },
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
