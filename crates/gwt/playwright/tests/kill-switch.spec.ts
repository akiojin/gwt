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

  test("manual and Monitor-linked panes dedupe waiting projection and rearm after running", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installKillSwitchBackend(page);
    await page.goto(APP_URL);

    const waitingCell = page.locator(".op-status-strip__cell--waiting");
    const waitingCount = page.locator("#op-strip-waiting");
    const completionToast = page.locator(
      '.toast-alerts__item[data-toast-id="agent-completion"]',
    );
    const uiFor = (id: string) => {
      const win = page.locator(`.workspace-window[data-id="${id}"]`);
      const windowTab = win.locator(
        `.window-tab[data-window-tab-id="${id}"]`,
      );
      return {
        win,
        statusChip: win.locator(".status-chip"),
        windowTab,
        windowTabCue: windowTab.locator(".window-tab-state"),
        minimapCell: page.locator(
          `.fleet-minimap__cell[data-window-id="${id}"]`,
        ),
        toast: page.locator(
          `.toast-alerts__item[data-toast-id="attention-${id}"]`,
        ),
      };
    };
    const manual = uiFor("agent-2");
    const monitor = uiFor("agent-3");
    await expect(manual.win).toBeVisible({ timeout: 10_000 });
    await expect(monitor.win).toBeHidden();

    const emitStatePair = (id: string, state: string) =>
      page.evaluate(
        ({ id, state }) =>
          (window as any).__killSwitchFixture.emitBatchAndWait([
            { kind: "window_state", window_id: id, state },
            { kind: "terminal_status", id, status: state },
          ]),
        { id, state },
      );
    const toastStats = () =>
      page.evaluate(() => (window as any).__killSwitchToastStats);
    const expectWaitingUi = async (ui) => {
      await expect(ui.win).toHaveAttribute("data-agent-state", "waiting");
      await expect(ui.statusChip).toHaveClass(/waiting/);
      await expect(ui.statusChip.locator(".status-label")).toHaveText("Waiting");
      await expect(ui.windowTab).toHaveAttribute("data-agent-state", "waiting");
      await expect(ui.windowTabCue).toBeVisible();
      await expect(ui.windowTabCue).toHaveText("WAIT");
      await expect(ui.minimapCell).toHaveAttribute("data-telemetry", "waiting");
    };
    const expectRunningUi = async (ui) => {
      await expect(ui.win).toHaveAttribute("data-agent-state", "running");
      await expect(ui.statusChip).toHaveClass(/running/);
      await expect(ui.statusChip.locator(".status-label")).toHaveText("Running");
      await expect(ui.windowTab).toHaveAttribute("data-agent-state", "running");
      await expect(ui.windowTabCue).toBeVisible();
      await expect(ui.windowTabCue).toHaveText("RUN");
      await expect(ui.minimapCell).toHaveAttribute("data-telemetry", "running");
    };

    // Count insertions as well as the final DOM. The shared toast host replaces
    // duplicate ids, so a plain toHaveCount(1) would hide duplicate publishes.
    await page.evaluate(() => {
      const stats = { attentionAdds: {}, completionAdds: 0 };
      (window as any).__killSwitchToastStats = stats;
      const observer = new MutationObserver((records) => {
        for (const record of records) {
          for (const node of record.addedNodes) {
            if (!(node instanceof Element)) continue;
            const items = node.matches(".toast-alerts__item")
              ? [node]
              : [...node.querySelectorAll(".toast-alerts__item")];
            for (const item of items) {
              const toastId = item.getAttribute("data-toast-id") || "";
              if (toastId.startsWith("attention-")) {
                const id = toastId.slice("attention-".length);
                stats.attentionAdds[id] = (stats.attentionAdds[id] || 0) + 1;
              } else if (toastId === "agent-completion") {
                stats.completionAdds += 1;
              }
            }
          }
        }
      });
      observer.observe(document.body, { childList: true, subtree: true });
      (window as any).__killSwitchToastObserver = observer;
    });

    // The daemon projection and authoritative PTY status may report the same
    // transition. emitStatePair does not return until BOTH dispatches and the
    // resulting controller/render work have crossed a two-frame barrier.
    await emitStatePair("agent-2", "waiting");

    // SPEC #3206: attention renders in the shared alerts stack, deduped by
    // window via data-toast-id; needs_input maps to the warn level; the whole
    // card is the jump button (onActivate).
    await expect(manual.toast).toBeVisible();
    await expect(manual.toast).toHaveAttribute("data-level", "warn");
    await expect(manual.toast.locator(".toast-alerts__title")).toHaveText(
      "Waiting for input",
    );
    await expect(completionToast).toHaveCount(0);
    expect(await toastStats()).toEqual({
      attentionAdds: { "agent-2": 1 },
      completionAdds: 0,
    });
    await expectWaitingUi(manual);
    await expect(waitingCount).toHaveText("1");
    await expect(waitingCell).toHaveClass(/op-status-strip__cell--alert/);

    const stage = page.locator("#canvas-stage");
    const before = await stage.evaluate((el) => el.style.transform);
    await manual.toast.click();
    await expect.poll(() => stage.evaluate((el) => el.style.transform)).not.toBe(before);
    await expect(manual.toast).toHaveCount(0);

    // Leaving waiting clears every persistent cue and rearms attention.
    await emitStatePair("agent-2", "running");
    await expectRunningUi(manual);
    await expect(waitingCount).toHaveText("0");
    await expect(waitingCell).not.toHaveClass(/op-status-strip__cell--alert/);

    // A later waiting episode creates exactly one new attention item even
    // when both backend event shapes carry the transition again.
    await emitStatePair("agent-2", "waiting");
    await expect(manual.toast).toHaveCount(1);
    await expect(completionToast).toHaveCount(0);
    expect(await toastStats()).toEqual({
      attentionAdds: { "agent-2": 2 },
      completionAdds: 0,
    });
    await expectWaitingUi(manual);
    await expect(waitingCount).toHaveText("1");
    await expect(waitingCell).toHaveClass(/op-status-strip__cell--alert/);

    await manual.toast.locator(".toast-alerts__dismiss").click();
    await emitStatePair("agent-2", "running");
    await expectRunningUi(manual);
    await expect(waitingCount).toHaveText("0");
    await expect(waitingCell).not.toHaveClass(/op-status-strip__cell--alert/);

    // `agent-3` models a pane launched by Issue Monitor. WindowState has no
    // launch-source field by design, so the fixture deliberately uses the same
    // Agent window schema and exact event projection as the manual pane. Switch
    // to it through the real tab-group interaction before exercising its cues.
    await manual.win
      .locator('.window-tab[data-window-tab-id="agent-3"]')
      .click();
    await expect(monitor.win).toBeVisible();
    await expect(manual.win).toBeHidden();
    await emitStatePair("agent-3", "waiting");
    await expect(monitor.toast).toHaveCount(1);
    await expect(monitor.toast.locator(".toast-alerts__title")).toHaveText(
      "Waiting for input",
    );
    await expectWaitingUi(monitor);
    await expect(waitingCount).toHaveText("1");
    await expect(waitingCell).toHaveClass(/op-status-strip__cell--alert/);
    expect(await toastStats()).toEqual({
      attentionAdds: { "agent-2": 2, "agent-3": 1 },
      completionAdds: 0,
    });

    await emitStatePair("agent-3", "running");
    await expectRunningUi(monitor);
    await expect(waitingCount).toHaveText("0");
    await expect(waitingCell).not.toHaveClass(/op-status-strip__cell--alert/);
    await emitStatePair("agent-3", "waiting");
    await expectWaitingUi(monitor);
    await expect(monitor.toast).toHaveCount(1);
    await expect(waitingCount).toHaveText("1");
    await expect(waitingCell).toHaveClass(/op-status-strip__cell--alert/);
    expect(await toastStats()).toEqual({
      attentionAdds: { "agent-2": 2, "agent-3": 2 },
      completionAdds: 0,
    });
    await expect(completionToast).toHaveCount(0);
  });
});

async function installKillSwitchBackend(page) {
  await page.addInitScript(() => {
    window.__sent = [];
    window.__sentKinds = [];

    function agentWindow(id, x, y, z, overrides = {}) {
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
        ...overrides,
      };
    }

    const windows = [
      agentWindow("agent-1", 120, 100, 1),
      agentWindow("agent-2", 1600, 1200, 2, {
        title: "Manual Agent",
        tab_group_id: "approval-agent-tabs",
        tab_group_active: true,
      }),
      // Issue Monitor launches an ordinary Agent window; its issue linkage is
      // session metadata and is intentionally not copied into the frontend
      // window schema. This second pane therefore exercises source parity
      // without inventing a frontend-only `source` discriminator.
      agentWindow("agent-3", 1600, 1200, 2, {
        title: "Monitor-linked Agent",
        agent_color: "violet",
        tab_group_id: "approval-agent-tabs",
      }),
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
        if (msg.kind === "activate_window_tab") {
          const target = windows.find((w) => w.id === msg.id);
          if (target?.tab_group_id) {
            for (const candidate of windows) {
              if (candidate.tab_group_id === target.tab_group_id) {
                candidate.tab_group_active = candidate.id === target.id;
              }
            }
            this.emit(stateMsg());
          }
        }
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload) {
        return new Promise((resolve) => {
          setTimeout(() => {
            this.dispatchEvent(
              new MessageEvent("message", { data: JSON.stringify(payload) }),
            );
            queueMicrotask(resolve);
          }, 0);
        });
      }
    }

    // Lets a test push backend events (e.g. window_state transitions) directly.
    window.__emit = (payload) => {
      if (socketRef) socketRef.emit(payload);
    };
    window.__killSwitchFixture = {
      async emitBatchAndWait(payloads) {
        if (!socketRef) throw new Error("fixture socket is not connected");
        for (const payload of payloads) {
          await socketRef.emit(payload);
        }
        // Dispatch handlers are synchronous today. Two animation frames also
        // cover MutationObserver delivery and render work if that changes.
        await new Promise((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(resolve)),
        );
      },
    };

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}
