import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// browser-check supplies a fresh checkout server. Keep the deterministic
// protocol fixture in both modes, but never replace that server's UI assets.
const focusAppUrl = process.env.GWT_PLAYWRIGHT_BASE_URL
  ? new URL(new URL(APP_URL).pathname, process.env.GWT_PLAYWRIGHT_BASE_URL).href
  : APP_URL;

type SentMessage = {
  kind?: string;
  id?: string;
  data?: string;
};

const agentNames: Record<string, string> = {
  "agent-running": "Aster", "agent-waiting": "Birch", "agent-stopped-tab-peer": "Cedar",
  "agent-live-future": "Dune", "agent-starting-hidden": "Elm",
};

test.describe("Agent-prioritized focus cycling", () => {
  test.use({ deviceScaleFactor: 1, viewport: { width: 1440, height: 900 } });

  test("selects individual Agent tabs in live priority order and reuses their terminal", async ({ page }, testInfo) => {
    const problems = recordErrors(page);
    await openFixture(page);
    await expect(windowById(page, "shell-topmost")).toHaveClass(/focused/);
    await expect(windowById(page, "agent-running")).toBeHidden();
    await page.evaluate(() => (window as any).__emitAgentFocusCycleFutureState());
    const transform = await canvasTransform(page);
    await settleAndClearMessages(page);

    // The starting Agent shares running's rank. Waiting comes next, then the
    // unknown live state and stopped Agent in their stable workspace order.
    await pressFocusCycle(page, "ArrowRight", "Control");
    await expectAgentTabFocus(page, "agent-running");
    await agentTile(page, "agent-running").locator(".xterm").evaluate(node => {
      (window as any).__focusCycleTerminal = node;
    });
    await pressFocusCycle(page, "ArrowRight", "Meta");
    await expectAgentTabFocus(page, "agent-starting-hidden");
    await pressFocusCycle(page, "ArrowRight", "Control");
    await expectAgentTabFocus(page, "agent-waiting");
    await pressFocusCycle(page, "ArrowRight", "Meta");
    await expectAgentTabFocus(page, "agent-live-future");
    await pressFocusCycle(page, "ArrowRight", "Control");
    await expectAgentTabFocus(page, "agent-stopped-tab-peer");
    await pressFocusCycle(page, "ArrowRight", "Meta");
    await expectAgentTabFocus(page, "agent-running");
    await pressFocusCycle(page, "ArrowLeft", "Control");
    await expectAgentTabFocus(page, "agent-stopped-tab-peer");
    await pressFocusCycle(page, "ArrowLeft", "Meta");
    await expectAgentTabFocus(page, "agent-live-future");

    // A live runtime update moves this earlier workspace entry into the active
    // bucket. Its next neighbor is now running, rather than stopped.
    await page.evaluate(() => (window as any).__emitAgentFocusCycleFutureState("running"));
    await expect(agentTile(page, "agent-live-future").locator(".agent-tile__header span")).toHaveText("Interactive");
    await pressFocusCycle(page, "ArrowRight", "Control");
    await expectAgentTabFocus(page, "agent-running");
    await expect.poll(() => focusTargets(page)).toEqual([
      "agent-running", "agent-starting-hidden", "agent-waiting",
      "agent-live-future", "agent-stopped-tab-peer", "agent-running",
      "agent-stopped-tab-peer", "agent-live-future", "agent-running",
    ]);
    await expect(page.locator('.agent-tile[data-agent-id="pm-window"]')).toHaveCount(0);
    await expect(page.locator('.agent-tile[data-agent-id="shell-topmost"]')).toHaveCount(0);

    await page.getByRole("button", { name: "Close Aster tab", exact: true }).click();
    await expect(page.getByRole("tab", { name: "Aster", exact: true })).toHaveCount(0);
    await expect(page.getByRole("tab", { name: "All agents", exact: true })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("tab", { name: "All agents", exact: true })).toBeFocused();
    await page.evaluate(() => (window as any).__agentFocusCycleSocket().emit({
      kind: "terminal_output", id: "agent-running", data_base64: btoa("LIVE AFTER TAB CLOSE\r\n"),
    }));
    await expect(agentTile(page, "agent-running").locator(".xterm-rows")).toContainText("LIVE AFTER TAB CLOSE");
    expect(problems).toEqual([]);
    await page.screenshot({ path: testInfo.outputPath("agent-cycle-all.png") });
    await pressFocusCycle(page, "ArrowRight", "Control");
    await expectAgentTabFocus(page, "agent-starting-hidden");
    await pressFocusCycle(page, "ArrowLeft", "Meta");
    await expectAgentTabFocus(page, "agent-running");
    expect(await agentTile(page, "agent-running").locator(".xterm").evaluate(
      node => node === (window as any).__focusCycleTerminal,
    )).toBe(true);

    // Keyboard navigation must not leak a CSI sequence to either PTY. The
    // only input goes once to the original window's unchanged session.
    await page.keyboard.press("q");
    await expect.poll(async () => (await sentMessages(page)).filter(message => message.kind === "terminal_input"))
      .toEqual([{ kind: "terminal_input", id: "agent-running", data: "q" }]);
    expectNavigationOnly(await sentMessages(page));
    expect(await canvasTransform(page)).toBe(transform);
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.includes("light") ? "light" : "dark");
    expect(problems).toEqual([]);
    await page.screenshot({ path: testInfo.outputPath("agent-cycle-selected.png") });
  });

  test("keeps split selection when a superseded frame request finally arrives", async ({ page }) => {
    const problems = recordErrors(page);
    await openFixture(page);
    await expect(windowById(page, "shell-topmost")).toHaveClass(/focused/);

    // Use the existing runtime-health launcher to enter requestWindowFrame.
    // The grouped tab's activation projection is deliberately held back.
    await page.evaluate(() => (window as any).__operatorShell.applyRuntimeHealth({
      state: "ok", processes: [{ pid: 301, role: "codex", name: "codex",
        focus_window_id: "agent-starting-hidden" }],
    }));
    await page.locator("#op-strip-runtime-health").hover();
    await page.getByRole("button", { name: "Focus codex process 301", exact: true }).click();
    await expect.poll(async () => (await sentMessages(page)).filter(message => message.kind === "activate_window_tab"))
      .toEqual([{ kind: "activate_window_tab", id: "agent-starting-hidden" }]);

    await page.getByRole("button", { name: "Split view", exact: true }).click();
    const left = page.getByRole("region", { name: "Left pane", exact: true });
    const right = page.getByRole("region", { name: "Right pane", exact: true });
    await right.getByRole("combobox", { name: "Right pane surface" }).selectOption("agents");
    await expect(left).toHaveAttribute("data-surface", "issues");
    await expect(right).toHaveAttribute("data-surface", "agents");
    // Start in the sibling pane: cycling must activate the existing Agents
    // pane rather than replace Issues with a second Agents surface.
    await left.getByRole("combobox", { name: "Left pane surface" }).focus();
    await expect(left).toHaveAttribute("data-active", "true");
    const transform = await canvasTransform(page);
    await settleAndClearMessages(page);
    await pressFocusCycle(page, "ArrowRight", "Control");
    await expectAgentTabFocus(page, "agent-running");

    const catalogVersion = await right.locator(".agents-surface").getAttribute("data-catalog-version");
    await page.evaluate(() => (window as any).__releaseAgentFocusCycleActivation());
    await expect(right.locator(".agents-surface")).not.toHaveAttribute("data-catalog-version", catalogVersion!);
    await expect(page.locator("#split-surfaces")).toBeVisible();
    await expect(left).toHaveAttribute("data-surface", "issues");
    await expect(right).toHaveAttribute("data-surface", "agents");
    await expect(right).toHaveAttribute("data-active", "true");
    await expectAgentTabFocus(page, "agent-running");
    await expect.poll(() => focusTargets(page)).toEqual(["agent-running"]);
    expectNavigationOnly(await sentMessages(page));
    expect(await canvasTransform(page)).toBe(transform);
    expect(problems).toEqual([]);
  });
});

async function openFixture(page: Page): Promise<void> {
  if (!process.env.GWT_PLAYWRIGHT_BASE_URL) await installEmbeddedRoutes(page);
  await installFocusCycleBackend(page);
  await page.goto(focusAppUrl);
}

function recordErrors(page: Page): string[] {
  const problems: string[] = [];
  page.on("console", message => { if (message.type() === "error") problems.push(message.text()); });
  page.on("pageerror", error => problems.push(String(error)));
  return problems;
}

function windowById(page: Page, id: string) {
  return page.locator(`.workspace-window[data-id='${id}']`);
}

function agentTile(page: Page, id: string) {
  return page.locator(`.agents-surface:not(.agents-surface--preview) .agent-tile[data-agent-id='${id}']`);
}

async function expectAgentTabFocus(page: Page, id: string): Promise<void> {
  await expect(page.getByRole("tab", { name: agentNames[id], exact: true })).toHaveAttribute("aria-selected", "true");
  await expect(agentTile(page, id)).toBeVisible();
  await expect(agentTile(page, id).locator(".xterm-helper-textarea")).toBeFocused();
}

async function pressFocusCycle(page: Page, key: "ArrowRight" | "ArrowLeft", modifier: "Control" | "Meta"): Promise<void> {
  // Control exercises the xterm CSI leak path on every platform; Meta covers
  // the actual macOS shortcut without synthesizing a DOM-only key event.
  await page.keyboard.press(`${modifier}+Shift+${key}`);
}

async function sentMessages(page: Page): Promise<SentMessage[]> {
  return page.evaluate(() => [...(((window as any).__agentFocusCycleSent ?? []) as SentMessage[])]);
}

async function focusTargets(page: Page): Promise<string[]> {
  return (await sentMessages(page)).filter(message => message.kind === "focus_window")
    .map(message => message.id ?? "");
}

function expectNavigationOnly(messages: SentMessage[]): void {
  const forbidden = new Set([
    "activate_window_tab", "update_viewport", "update_window_geometry", "cycle_focus",
    "dock_window_tab", "detach_window_tab", "place_agent_window_in_kanban",
    "move_agent_kanban_card", "undock_agent_window", "dock_agent_window_to_issue",
    "create_window", "close_window", "stop_window", "stop_all_windows", "restart_window",
    "open_pm_agent", "arrange_windows",
  ]);
  expect(messages.filter(message => forbidden.has(message.kind ?? "")
    || /(?:save.*group|placement|launch_wizard)/.test(message.kind ?? ""))).toEqual([]);
}

async function canvasTransform(page: Page): Promise<string> {
  return page.locator("#canvas-stage").evaluate(element => (element as HTMLElement).style.transform);
}

async function settleAndClearMessages(page: Page): Promise<void> {
  await page.evaluate(async () => {
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    (window as any).__agentFocusCycleSent.length = 0;
  });
}

async function installFocusCycleBackend(page: Page): Promise<void> {
  await page.addInitScript(names => {
    const canvasWindow = (id: string, overrides: Record<string, unknown>) => ({
      id,
      title: id,
      preset: "agent",
      geometry: { x: 120, y: 100, width: 560, height: 340 },
      geometry_revision: 0,
      z_index: 1,
      status: "idle",
      minimized: false,
      maximized: false,
      pre_maximize_geometry: null,
      persist: true,
      purpose_title: null,
      dynamic_title: null,
      dynamic_title_detail: null,
      agent_id: id,
      agent_name: names[id] || null,
      session_id: `${id}-session`,
      agent_color: "cyan",
      tab_group_id: null,
      tab_group_active: false,
      placement: { kind: "canvas" },
      ...overrides,
    });

    // Workspace order deliberately differs from runtime rank. The running
    // Agent is off-canvas; the PM and Shell must never become candidates.
    const windows = [
      canvasWindow("shell-topmost", {
        title: "Shell",
        preset: "shell",
        agent_id: null,
        status: "running",
        geometry: { x: 80, y: 80, width: 560, height: 340 },
        z_index: 40,
      }),
      canvasWindow("pm-window", {
        preset: "claude",
        is_pm: true,
        status: "running",
        z_index: 2,
      }),
      canvasWindow("issue-window", {
        preset: "issue",
        agent_id: null,
        z_index: 3,
      }),
      canvasWindow("board-window", {
        preset: "board",
        agent_id: null,
        z_index: 4,
      }),
      canvasWindow("agent-waiting", {
        status: "waiting",
        geometry: { x: 700, y: 80, width: 560, height: 340 },
        z_index: 10,
      }),
      canvasWindow("agent-live-future", {
        status: "idle",
        geometry: { x: 1010, y: 460, width: 560, height: 340 },
        z_index: 15,
      }),
      canvasWindow("agent-running", {
        status: "running",
        placement: { kind: "issue_preview", issue_window_id: "issue-window", issue_number: 4777 },
        geometry: { x: 1320, y: 80, width: 560, height: 340 },
        z_index: 20,
      }),
      canvasWindow("agent-stopped-tab-peer", {
        status: "stopped",
        geometry: { x: 1940, y: 80, width: 560, height: 340 },
        z_index: 30,
        tab_group_id: "agent-tabs",
        tab_group_active: true,
      }),
      canvasWindow("agent-starting-hidden", {
        status: "starting",
        geometry: { x: 1940, y: 80, width: 560, height: 340 },
        z_index: 5,
        tab_group_id: "agent-tabs",
        tab_group_active: false,
      }),
    ];

    let zCounter = 40;
    let socket: FixtureWebSocket | null = null;

    const workspaceState = () => ({
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Agent Focus Cycle Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: windows.map((windowData) => ({ ...windowData })),
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    });
    (window as any).__agentFocusCycleActivationReleased = false;
    (window as any).__emitAgentFocusCycleFutureState = (status = "future-state") => {
      return socket?.emit({
        kind: "terminal_status",
        id: "agent-live-future",
        status,
        detail: "future runtime fixture",
      });
    };
    (window as any).__releaseAgentFocusCycleActivation = () => {
      (window as any).__agentFocusCycleActivationReleased = true;
      // Keep focus echoes on the old group selection until this publication.
      for (const candidate of windows) {
        if (candidate.tab_group_id === "agent-tabs") {
          candidate.tab_group_active = candidate.id === "agent-starting-hidden";
        }
      }
      return socket?.emit(workspaceState());
    };

    (window as any).__agentFocusCycleSent = [];

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      url: string;
      readyState = FixtureWebSocket.CONNECTING;

      constructor(url: string) {
        super();
        this.url = url;
        socket = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(workspaceState());
        }, 0);
      }

      send(raw: string): void {
        let message: SentMessage;
        try {
          message = JSON.parse(raw) as SentMessage;
        } catch {
          return;
        }
        (window as any).__agentFocusCycleSent.push(message);

        if (message.kind === "activate_window_tab") {
          const target = windows.find(
            (windowData) => windowData.id === message.id,
          );
          if (target?.tab_group_id) {
            if (
              target.id === "agent-starting-hidden" &&
              !(window as any).__agentFocusCycleActivationReleased
            ) {
              return;
            }
            for (const candidate of windows) {
              if (candidate.tab_group_id === target.tab_group_id) {
                candidate.tab_group_active = candidate.id === target.id;
              }
            }
            this.emit(workspaceState());
          }
          return;
        }

        if (message.kind === "focus_window") {
          const target = windows.find(
            (windowData) => windowData.id === message.id,
          );
          if (target) {
            zCounter += 1;
            target.z_index = zCounter;
            this.emit(workspaceState());
          }
        }
      }

      close(): void {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: unknown): Promise<void> {
        return new Promise((resolve) => setTimeout(() => {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
          resolve();
        }, 0));
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });

    // Retain a reference so the fixture socket is not eligible for collection
    // in engines that aggressively collect detached EventTargets.
    (window as any).__agentFocusCycleSocket = () => socket;
  }, agentNames);
}
