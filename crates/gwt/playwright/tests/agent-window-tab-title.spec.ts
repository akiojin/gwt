import { expect, test, type Page } from "@playwright/test";
import { APP_URL, APP_PROJECT_KEY, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { liveGwtProjectUrl } from "./_helpers/live-gwt";

const liveBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
const appUrl = liveBase
  ? new URL(liveBase).pathname === "/" ? liveGwtProjectUrl(liveBase, APP_PROJECT_KEY) : liveBase
  : APP_URL;
const errors = new WeakMap<Page, string[]>();
const rowFor = (page: Page, title: string) => page.locator(".window-list-row")
  .filter({ has: page.locator(".window-list-title", { hasText: title }) });

// #4777 T-3: grouped agents remain reachable through Windows after removing
// the window tab strip. The existing titlebar/list retain title and telemetry.
test.describe("Agent window navigation without tabs", () => {
  test.use({ deviceScaleFactor: 1, viewport: { width: 1440, height: 900 } });

  test.beforeEach(async ({ page }, info) => {
    const captured: string[] = [];
    errors.set(page, captured);
    page.on("pageerror", error => captured.push(error.message));
    page.on("console", message => { if (message.type() === "error") captured.push(message.text()); });
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme),
      info.project.name.includes("light") ? "light" : "dark");
    if (!liveBase) await installEmbeddedRoutes(page);
    await installTabbedAgentsBackend(page);
    await page.goto(appUrl);
    await page.getByRole("button", { name: "Works", exact: true }).click();
    await page.locator('.work-row[data-window-id="agent-1"] .work-open').click();
    await expect(page.locator(".works-terminal[data-window-id='agent-1']")).toBeVisible();
  });

  test.afterEach(async ({ page }, info) => {
    await expect(page.locator("html")).toHaveAttribute("data-theme",
      info.project.name.includes("light") ? "light" : "dark");
    expect(errors.get(page), "zero console/page errors").toEqual([]);
  });

  test("full titles remain in the titlebar and Windows list without tab DOM", async ({ page }) => {
    await expect(page.locator(".window-tab-strip, .window-tab")).toHaveCount(0);
    await expect(page.locator(".workspace-window[data-id='agent-1'] .title-text"))
      .toHaveAttribute("title", "Codex · implementing complete maximize");
    await page.locator("#window-list-button").click();
    await expect(rowFor(page, "Aster").locator(".window-list-title"))
      .toHaveAttribute("title", "Codex · implementing complete maximize");
    await expect(rowFor(page, "Birch").locator(".window-list-title"))
      .toHaveAttribute("title", "Birch");
  });

  test("Windows focuses a legacy grouped agent for input without destructive close chrome", async ({ page }) => {
    const active = page.locator(".works-terminal[data-window-id='agent-1']");
    const target = page.locator(".works-terminal[data-window-id='agent-2']");
    const source = page.locator(".workspace-window[data-id='agent-2']");
    await expect(target).toBeHidden();
    await page.locator("#window-list-button").click();
    await rowFor(page, "Birch").click();
    await expect(target).toBeVisible();
    await expect(active).toBeHidden();
    await expect(page.locator(".works-full-name")).toHaveText("Birch");
    // Read-only list caches intentionally omit the startup performance beacon;
    // the existing input readiness metric and real input prove the live gate.
    await expect.poll(() => page.evaluate(() => window.__gwtTerminalTestApi.metrics("agent-2").isReady)).toBe(true);
    await target.locator(".xterm-helper-textarea").focus();
    await expect(target.locator(".xterm-helper-textarea")).toBeFocused();
    await page.keyboard.type("hello");
    await expect.poll(() => page.evaluate(() => window.__tabbedAgentsFixture.sent
      .filter(message => message.kind === "terminal_input" && message.id === "agent-2")
      .map(message => message.data).join(""))).toBe("hello");
    const closedIds = () => page.evaluate(() => window.__tabbedAgentsFixture.sent
      .filter(message => message.kind === "close_window").map(message => message.id));
    await expect(source.locator('[data-action="close"]')).toHaveCount(0);
    await expect(page.locator("#window-close-confirm-modal")).toHaveCount(0);
    expect(await closedIds()).toEqual([]);
    // Back changes presentation only; the native terminal remains owned by its session.
    await page.locator('.works-full [data-action="back"]').click();
    expect(await closedIds()).toEqual([]);
    await page.locator("#window-list-button").click();
    await rowFor(page, "Birch").click();
    await expect(target).toBeVisible();
  });

  test("Windows list batches inactive telemetry without delaying navigation", async ({ page }, info) => {
    await page.locator("#window-list-button").click();
    const trigger = page.locator("#window-list-button");
    await trigger.focus();
    await trigger.hover();
    const panel = page.locator("#window-list-panel");
    const chip = rowFor(page, "Birch").locator(".status-chip");
    await expect(chip).toHaveClass(/running/);
    await expect(chip.locator(".status-label")).toHaveText("Running");
    const burst = await panel.evaluate(async panel => {
      const before = panel.innerHTML;
      const focused = document.activeElement;
      let rebuilds = 0;
      const observer = new MutationObserver(records => {
        rebuilds += records.filter(record => record.type === "childList" && record.removedNodes.length > 0).length;
      });
      observer.observe(panel, { childList: true });
      try {
        for (let n = 1; n <= 50; n++) window.__tabbedAgentsFixture.emitTerminalStatus("agent-2", n % 2 ? "running" : "idle", true);
        const unchanged = panel.innerHTML === before;
        await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        return { unchanged, rebuilds, focusPreserved: document.activeElement === focused,
          hoverPreserved: focused?.matches(":hover") };
      } finally { observer.disconnect(); }
    });
    expect(burst).toEqual({ unchanged: true, rebuilds: 1, focusPreserved: true, hoverPreserved: true });
    await expect(chip).toHaveClass(/idle/);
    await expect(chip.locator(".status-label")).toHaveText("Idle");
    await expect(page.locator(".works-full-name")).toHaveText("Aster");
    const fallback = await panel.evaluate(async panel => {
      const originalRaf = window.requestAnimationFrame;
      window.requestAnimationFrame = () => 0;
      const started = performance.now();
      const before = panel.innerHTML;
      try {
        return await new Promise<{ unchanged: boolean; latency: number }>((resolve, reject) => {
          const observer = new MutationObserver(() => {
            if (!panel.textContent?.includes("Running")) return;
            observer.disconnect(); clearTimeout(timeout);
            resolve({ unchanged, latency: performance.now() - started });
          });
          const timeout = setTimeout(() => { observer.disconnect(); reject(new Error("Windows list fallback exceeded 1 second")); }, 1000);
          observer.observe(panel, { childList: true });
          window.__tabbedAgentsFixture.emitTerminalStatus("agent-2", "running", true);
          const unchanged = panel.innerHTML === before;
        });
      } finally { window.requestAnimationFrame = originalRaf; }
    });
    expect(fallback.unchanged).toBe(true);
    expect(fallback.latency).toBeLessThan(1000);
    await expect(chip).toHaveClass(/running/);
    await info.attach("windows-list-frame", { body: JSON.stringify({ burst, fallback }), contentType: "application/json" });
    await page.screenshot({ path: info.outputPath("windows-list-frame.png") });
    // A receipt waiting for paint must not retarget the row already displayed.
    await rowFor(page, "Birch").evaluate(row => {
      window.__tabbedAgentsFixture.emitTerminalStatus("agent-2", "idle", true);
      (row as HTMLButtonElement).click();
    });
    await expect(panel).toBeHidden();
    await expect(page.locator(".works-full-name")).toHaveText("Birch");
    await trigger.click();
    await expect(panel).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(panel).toBeHidden();
    await expect(trigger).toBeFocused();
  });

  test("provider runtime pushes update Works and fleet counts within one second", async ({ page }, info) => {
    await page.locator('.works-full [data-action="back"]').click();
    await page.evaluate(() => window.__tabbedAgentsFixture.emitStaleWorkProjection());
    const transitions = [];
    for (const state of ["running", "waiting", "idle"]) {
      const label = state[0].toUpperCase() + state.slice(1);
      await page.evaluate(state => window.__tabbedAgentsFixture.emitProviderStates(state), state);
      await expect.poll(() => page.evaluate(({ state, label }) => {
        const rows = ["agent-1", "agent-2"].every(id => document.querySelector(`.work-row[data-window-id="${id}"] .work-state`)?.textContent === label);
        const counts = ["running", "waiting", "idle"].every(candidate =>
          document.querySelector(`.works-counts [data-count="${candidate}"]`)?.textContent === (candidate === state ? "2" : "0") &&
          document.getElementById(`op-strip-${candidate}`)?.textContent === (candidate === state ? "2" : "0"));
        if (rows && counts) window.__tabbedAgentsFixture.lastStateObserved = performance.now();
        return rows && counts;
      }, { state, label }), { timeout: 1000, intervals: [10, 25, 50] }).toBe(true);
      const elapsed = await page.evaluate(() => window.__tabbedAgentsFixture.lastStateObserved - window.__tabbedAgentsFixture.lastStatePush);
      expect(elapsed, `${state} push reaches Works and fleet`).toBeLessThan(1000);
      transitions.push({ state, elapsed_ms: elapsed });
      for (const [id, name] of [["agent-1", "Aster"], ["agent-2", "Birch"]]) {
        const row = page.locator(`.work-row[data-window-id="${id}"]`);
        await expect(row.locator(".work-name")).toHaveText(name);
        await expect(row).toHaveAttribute("data-state", state);
        await expect(row.locator(".work-state")).toHaveText(label);
      }
    }
    await info.attach("runtime-transitions", { body: JSON.stringify(transitions, null, 2), contentType: "application/json" });
    await page.screenshot({ path: info.outputPath("provider-runtime-states.png"), fullPage: true });
  });
});

async function installTabbedAgentsBackend(page) {
  await page.addInitScript(() => {
    window.__gwtPlaywrightTestBridge = true;
    const baseWindow = {
      preset: "agent",
      geometry: { x: 180, y: 120, width: 720, height: 360 },
      geometry_revision: 0,
      status: "idle",
      minimized: false,
      maximized: false,
      pre_maximize_geometry: null,
      persist: true,
      purpose_title: null,
      dynamic_title: null,
      dynamic_title_detail: null,
      tab_group_id: "grp-1",
    };

    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Tab Title Fixture",
            project_key: location.pathname.split("/").filter(Boolean).at(-1),
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  ...baseWindow,
                  id: "agent-1",
                  session_id: "session-agent-1",
                  title: "Codex",
                  z_index: 2,
                  agent_id: "codex",
                  agent_name: "Aster",
                  agent_color: "cyan",
                  dynamic_title_detail: "Codex · implementing complete maximize",
                  tab_group_active: true,
                },
                {
                  ...baseWindow,
                  id: "agent-2",
                  session_id: "session-agent-2",
                  title: "Claude",
                  status: "running",
                  z_index: 1,
                  agent_id: "claude",
                  agent_name: "Birch",
                  agent_color: "violet",
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

    const sent = [];

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      static instances = [];

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        FixtureWebSocket.instances.push(this);
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(workspaceState);
          this.emit({ kind: "active_work_projection", projection: { id: "fixture-local-live", active_works: [], local_agent_activity: { project_key: location.pathname.split("/").filter(Boolean).at(-1), host_id: "window-navigation-fixture", observed_at: new Date().toISOString(), agents: [
          { window_id: 'agent-1', session_id: 'session-agent-1', runtime_incarnation: 1, is_pm: false, issue_number: null },
          { window_id: 'agent-2', session_id: 'session-agent-2', runtime_incarnation: 1, is_pm: false, issue_number: null }
        ] } } });
        }, 0);
      }

      send(raw) {
        const message = JSON.parse(raw);
        sent.push(message);
        const windows = workspaceState.workspace.tabs[0].workspace.windows;
        if (message.kind === "activate_window_tab") {
          for (const data of windows) data.tab_group_active = data.id === message.id;
          this.emit(workspaceState);
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
    window.__tabbedAgentsFixture = {
      sent,
      lastStatePush: 0,
      lastStateObserved: 0,
      emitStaleWorkProjection() {
        FixtureWebSocket.instances.forEach(socket => socket.emit({ kind: "active_work_projection", projection: {
          status_category: "active", active_agents: 2, blocked_agents: 0,
          agents: [{ window_id: "agent-1" }, { window_id: "agent-2" }], active_works: [],
        } }));
      },
      emitProviderStates(status) {
        this.lastStatePush = performance.now();
        FixtureWebSocket.instances.forEach(socket => {
          socket.emit({ kind: "window_state", window_id: "agent-1", state: status });
          socket.emit({ kind: "terminal_status", id: "agent-2", status });
        });
      },
      resetUngrouped() {
        workspaceState.workspace.tabs[0].workspace.windows.forEach((data, index) => {
          data.tab_group_id = null;
          data.tab_group_active = false;
          data.geometry = { x: 120 + index * 650, y: 120, width: 500, height: 360 };
        });
        FixtureWebSocket.instances.forEach(socket => socket.emit(workspaceState));
      },
      emitTerminalStatus(windowId, status, immediate = false) {
        const payload = { kind: "terminal_status", id: windowId, status };
        // Controlled receiver bursts start after the transport dispatcher.
        if (immediate) {
          window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
          return;
        }
        FixtureWebSocket.instances?.forEach((socket) => {
          socket.emit(payload);
        });
      },
    };
  });
}
