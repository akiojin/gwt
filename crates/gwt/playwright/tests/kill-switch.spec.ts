import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// SPEC-2356 Anshin Addendum — Phase 1 kill-switch + attention UI.
//
// Drives the embedded frontend against a WebSocket stub that captures every
// event the frontend sends. Asserts the FR-040..044 controls:
//   - FR-041: window-chrome STOP sends stop_window (window stays on canvas)
//   - FR-044: a stopped window swaps STOP for RESTART -> restart_window
//   - FR-042: STOP ALL rail item confirms then sends stop_all_windows
//   - FR-043: the palette send-input entry routes pane_send_input by session_id
//   - FR-040: a needs_input transition pops an in-app attention toast that
//     frames the window on click
test.describe("Anshin Phase 1 kill-switch + attention", () => {
  test.use({ deviceScaleFactor: 1, viewport: { width: 1440, height: 900 } });

  test("STOP sends stop_window and the window stays on canvas", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installKillSwitchBackend(page);
    await page.goto(APP_URL);

    const win = page.locator('.workspace-window[data-id="agent-1"]');
    await expect(win).toBeVisible({ timeout: 10_000 });

    const stop = win.locator('[data-action="stop"]');
    await expect(stop).toBeVisible();
    await stop.click();

    await expect
      .poll(() => page.evaluate(() => window.__sentKinds))
      .toContain("stop_window");
    // The window must remain on the canvas after stopping its runtime.
    await expect(win).toBeVisible();
    const sent = await page.evaluate(() => window.__sent);
    const stopEvent = sent.find((e) => e.kind === "stop_window");
    expect(stopEvent.id).toBe("agent-1");
  });

  test("a stopped window exposes RESTART -> restart_window", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installKillSwitchBackend(page);
    await page.goto(APP_URL);

    const win = page.locator('.workspace-window[data-id="agent-1"]');
    await expect(win).toBeVisible({ timeout: 10_000 });

    // Push a Stopped runtime state; the chrome must swap STOP for RESTART.
    await page.evaluate(() => window.__emit({ kind: "window_state", window_id: "agent-1", state: "stopped" }));

    const restart = win.locator('[data-action="restart"]');
    await expect(restart).toBeVisible();
    await expect(win.locator('[data-action="stop"]')).toBeHidden();
    await restart.click();

    await expect
      .poll(() => page.evaluate(() => window.__sentKinds))
      .toContain("restart_window");
    const sent = await page.evaluate(() => window.__sent);
    expect(sent.find((e) => e.kind === "restart_window").id).toBe("agent-1");
  });

  test("STOP ALL confirms then sends stop_all_windows", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installKillSwitchBackend(page);
    await page.goto(APP_URL);

    await expect(page.locator('.workspace-window[data-id="agent-1"]')).toBeVisible({
      timeout: 10_000,
    });

    const stopAll = page.locator('.op-rail__item[data-cmd="stop-all-windows"]');
    await expect(stopAll).toBeVisible();
    page.once("dialog", (dialog) => dialog.accept());
    await stopAll.click();

    await expect
      .poll(() => page.evaluate(() => window.__sentKinds))
      .toContain("stop_all_windows");
  });

  test("the palette send-input entry routes pane_send_input by session_id", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installKillSwitchBackend(page);
    await page.goto(APP_URL);

    const win = page.locator('.workspace-window[data-id="agent-1"]');
    await expect(win).toBeVisible({ timeout: 10_000 });
    // Focus the agent window so the focused-pane helper has a target.
    await win.locator(".titlebar .title-text").click();

    await page.evaluate(() => {
      window.prompt = () => "hello agent";
    });
    await page.keyboard.press("Meta+K");
    const paletteInput = page.locator("#op-palette-input");
    await expect(paletteInput).toBeVisible();
    await paletteInput.fill("Send Input");
    const sendInputRow = page
      .locator("#op-palette-list .op-palette__row", { hasText: "Send Input" })
      .first();
    await expect(sendInputRow).toBeVisible();
    await page.keyboard.press("Enter");

    await expect
      .poll(() => page.evaluate(() => window.__sentKinds))
      .toContain("pane_send_input");
    const sent = await page.evaluate(() => window.__sent);
    const inj = sent.find((e) => e.kind === "pane_send_input");
    expect(inj.session_id).toBe("session-agent-1");
    expect(inj.text).toBe("hello agent");
  });

  test("a needs_input transition pops an attention toast that frames on click", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installKillSwitchBackend(page);
    await page.goto(APP_URL);

    await expect(page.locator('.workspace-window[data-id="agent-2"]')).toBeVisible({
      timeout: 10_000,
    });

    // agent-2 enters waiting -> needs_input attention toast.
    await page.evaluate(() => window.__emit({ kind: "window_state", window_id: "agent-2", state: "waiting" }));

    // SPEC #3206: attention renders in the shared alerts stack, deduped by
    // window via data-toast-id; needs_input maps to the warn level; the whole
    // card is the jump button (onActivate).
    const toast = page.locator('.toast-alerts__item[data-toast-id="attention-agent-2"]');
    await expect(toast).toBeVisible();
    await expect(toast).toHaveAttribute("data-level", "warn");

    const stage = page.locator("#canvas-stage");
    const before = await stage.evaluate((el) => el.style.transform);
    await toast.click();
    await page.waitForTimeout(500);
    await expect.poll(() => stage.evaluate((el) => el.style.transform)).not.toBe(before);
    await expect(toast).toHaveCount(0);
  });
});

async function installKillSwitchBackend(page) {
  await page.addInitScript(() => {
    window.__sent = [];
    window.__sentKinds = [];

    function agentWindow(id, x, y, z) {
      return {
        id,
        title: id,
        preset: "agent",
        geometry: { x, y, width: 480, height: 320 },
        geometry_revision: 0,
        z_index: z,
        status: "running",
        persist: true,
        purpose_title: null,
        dynamic_title: null,
        dynamic_title_detail: null,
        agent_id: id,
        agent_color: "cyan",
        tab_group_id: null,
        tab_group_active: false,
        session_id: `session-${id}`,
      };
    }

    const windows = [
      agentWindow("agent-1", 120, 100, 1),
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
              title: "Kill Switch Fixture",
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

    let socketRef = null;

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        socketRef = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(stateMsg());
        }, 0);
      }

      send(data) {
        let msg;
        try {
          msg = JSON.parse(data);
        } catch {
          return;
        }
        window.__sent.push(msg);
        window.__sentKinds.push(msg.kind);
        if (msg.kind === "focus_window") {
          const target = windows.find((w) => w.id === msg.id);
          if (target) {
            target.z_index += 100;
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

    // Lets a test push backend events (e.g. window_state transitions) directly.
    window.__emit = (payload) => {
      if (socketRef) socketRef.emit(payload);
    };

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}
