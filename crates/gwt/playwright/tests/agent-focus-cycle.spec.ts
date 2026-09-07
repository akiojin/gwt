import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

type SentMessage = {
  kind?: string;
  id?: string;
};

test.describe("Agent-prioritized focus cycling", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("cycles only Agents by runtime priority and activates hidden tabs before focus", async ({
    page,
  }) => {
    // Issue #4069 AC-2: the headed run must finish with zero console / page
    // errors in both themes, not just the right focus targets.
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    await installEmbeddedRoutes(page);
    await installFocusCycleBackend(page);
    await page.goto(APP_URL);

    const shell = windowById(page, "shell-topmost");
    const runningAgent = windowById(page, "agent-running");
    const hiddenStartingAgent = windowById(page, "agent-starting-hidden");
    const stoppedTabPeer = windowById(page, "agent-stopped-tab-peer");

    // The fixture starts on the non-Agent Shell. In workspace order the next
    // visible window is the waiting Agent, which is exactly where the legacy
    // all-window cycle goes. The Agent-priority contract instead starts at the
    // first active candidate: agent-running.
    await expect(shell).toBeVisible({ timeout: 10_000 });
    await expect(shell).toHaveClass(/focused/);
    await expect(runningAgent).toBeVisible();
    await expect(hiddenStartingAgent).toBeHidden();
    await page.evaluate(() => {
      (window as any).__emitAgentFocusCycleFutureState();
    });

    const stage = page.locator("#canvas-stage");
    const initialTransform = await stage.evaluate(
      (element) => (element as HTMLElement).style.transform,
    );
    await settleAndClearMessages(page);

    await pressFocusCycle(page, "ArrowRight");

    // RED on the legacy implementation: actual is agent-waiting because it
    // walks every visible window in workspace order.
    await expect.poll(() => focusTargets(page)).toEqual(["agent-running"]);
    await expect(runningAgent).toHaveClass(/focused/);
    await expect
      .poll(() =>
        stage.evaluate((element) => (element as HTMLElement).style.transform),
      )
      .not.toBe(initialTransform);

    // starting shares the active bucket with running, so the hidden starting
    // tab must be visited before the waiting Agent despite its later workspace
    // position.
    await pressFocusCycle(page, "ArrowRight");

    // Activation is asynchronous. Do not focus/frame with hidden layout
    // metrics; wait until the backend projection mounts the selected tab.
    await expect
      .poll(async () =>
        (await sentMessages(page))
          .filter((message) => message.kind === "activate_window_tab")
          .map((message) => message.id),
      )
      .toEqual(["agent-starting-hidden"]);
    await page.evaluate(() => {
      (window as any).__emitAgentFocusCycleUnrelatedWorkspace();
    });
    await expect.poll(() => focusTargets(page)).toEqual(["agent-running"]);
    await expect(hiddenStartingAgent).toBeHidden();

    await page.evaluate(() => {
      (window as any).__releaseAgentFocusCycleActivation();
    });
    await expect.poll(() => focusTargets(page)).toEqual([
      "agent-running",
      "agent-starting-hidden",
    ]);

    const hiddenSelectionMessages = await sentMessages(page);
    const activationIndex = hiddenSelectionMessages.findIndex(
      (message) =>
        message.kind === "activate_window_tab" &&
        message.id === "agent-starting-hidden",
    );
    const focusIndex = hiddenSelectionMessages.findIndex(
      (message) =>
        message.kind === "focus_window" &&
        message.id === "agent-starting-hidden",
    );
    expect(activationIndex).toBeGreaterThanOrEqual(0);
    expect(focusIndex).toBeGreaterThan(activationIndex);
    await expect(hiddenStartingAgent).toBeVisible();
    await expect(hiddenStartingAgent).toHaveClass(/focused/);
    await expect(stoppedTabPeer).toBeHidden();

    await pressFocusCycle(page, "ArrowRight");
    await expect.poll(() => focusTargets(page)).toEqual([
      "agent-running",
      "agent-starting-hidden",
      "agent-waiting",
    ]);

    // Backward traversal uses the same prioritized list. It returns through
    // starting and running, then wraps to the final Agent without ever landing
    // on the Shell that remains between those Agents in workspace order.
    await pressFocusCycle(page, "ArrowLeft");
    await pressFocusCycle(page, "ArrowLeft");
    await pressFocusCycle(page, "ArrowLeft");
    await expect.poll(() => focusTargets(page)).toEqual([
      "agent-running",
      "agent-starting-hidden",
      "agent-waiting",
      "agent-starting-hidden",
      "agent-running",
      "agent-stopped-tab-peer",
    ]);

    const messages = await sentMessages(page);
    expect(focusTargetsFrom(messages)).not.toContain("shell-topmost");

    // Focus cycling is navigation-only. The existing frameWindow path may
    // persist its camera and frame-clamp geometry, but it must not emit any
    // Agent/process/window lifecycle command.
    const navigationKinds = new Set([
      "activate_window_tab",
      "focus_window",
      "update_viewport",
      "update_window_geometry",
    ]);
    expect(
      messages
        .map((message) => message.kind ?? "")
        .filter((kind) => !navigationKinds.has(kind)),
    ).toEqual([]);

    expect(pageErrors).toEqual([]);
    expect(consoleErrors).toEqual([]);
  });
});

function windowById(page: Page, id: string) {
  return page.locator(`.workspace-window[data-id='${id}']`);
}

async function pressFocusCycle(
  page: Page,
  key: "ArrowRight" | "ArrowLeft",
): Promise<void> {
  const modifier = process.platform === "darwin" ? "Meta" : "Control";
  await page.keyboard.press(`${modifier}+Shift+${key}`);
}

async function sentMessages(page: Page): Promise<SentMessage[]> {
  return page.evaluate(
    () => [...(((window as any).__agentFocusCycleSent ?? []) as SentMessage[])],
  );
}

function focusTargetsFrom(messages: SentMessage[]): string[] {
  return messages
    .filter((message) => message.kind === "focus_window")
    .map((message) => message.id)
    .filter((id): id is string => typeof id === "string");
}

async function focusTargets(page: Page): Promise<string[]> {
  return focusTargetsFrom(await sentMessages(page));
}

async function settleAndClearMessages(page: Page): Promise<void> {
  await page.evaluate(async () => {
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
    (window as any).__agentFocusCycleSent.length = 0;
  });
}

async function installFocusCycleBackend(page: Page): Promise<void> {
  await page.addInitScript(() => {
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
      agent_color: "cyan",
      tab_group_id: null,
      tab_group_active: false,
      placement: { kind: "canvas" },
      ...overrides,
    });

    // Workspace order is deliberate: from the initially focused Shell, the
    // old visible-window cycle chooses agent-waiting. Runtime-priority order is
    // running -> hidden starting -> waiting -> stopped tab peer.
    const windows = [
      canvasWindow("shell-topmost", {
        title: "Shell",
        preset: "shell",
        agent_id: null,
        status: "running",
        geometry: { x: 80, y: 80, width: 560, height: 340 },
        z_index: 40,
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
    const inactiveTabWorkspaceState = workspaceState();

    (window as any).__agentFocusCycleActivationReleased = false;
    (window as any).__emitAgentFocusCycleFutureState = () => {
      socket?.emit({
        kind: "terminal_status",
        id: "agent-live-future",
        status: "future-state",
        detail: "future runtime fixture",
      });
    };
    (window as any).__emitAgentFocusCycleUnrelatedWorkspace = () => {
      socket?.emit(inactiveTabWorkspaceState);
    };
    (window as any).__releaseAgentFocusCycleActivation = () => {
      (window as any).__agentFocusCycleActivationReleased = true;
      socket?.emit(workspaceState());
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
            for (const candidate of windows) {
              if (candidate.tab_group_id === target.tab_group_id) {
                candidate.tab_group_active = candidate.id === target.id;
              }
            }
            if (
              target.id === "agent-starting-hidden" &&
              !(window as any).__agentFocusCycleActivationReleased
            ) {
              return;
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

      emit(payload: unknown): void {
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

    // Retain a reference so the fixture socket is not eligible for collection
    // in engines that aggressively collect detached EventTargets.
    (window as any).__agentFocusCycleSocket = () => socket;
  });
}
