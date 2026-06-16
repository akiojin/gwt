import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Agent window scrollback lifecycle", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("keeps wheel scrollback alive after hidden-visible cycles and Plan-mode idle", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installAgentScrollBackend(page);
    await page.goto(APP_URL);

    await expect(page.locator(".workspace-window[data-id='agent-1']")).toBeVisible();
    await emitTerminalText(page, "agent-1", numberedLines("plan-cycle", 180));
    await emitTerminalStatus(page, "agent-1", "ready", "Plan mode input wait");
    await waitForScrollableTerminal(page, "agent-1");

    for (let index = 0; index < 5; index += 1) {
      await setActiveAgentTab(page, "agent-2");
      await expect(page.locator(".workspace-window[data-id='agent-1']")).toBeHidden();
      await emitTerminalText(page, "agent-1", numberedLines(`hidden-${index}`, 20));
      await expectPendingRefresh(page, "agent-1", true);

      await setActiveAgentTab(page, "agent-1");
      await expect(page.locator(".workspace-window[data-id='agent-1']")).toBeVisible();
      await expectPendingRefresh(page, "agent-1", false);
      await waitForScrollableTerminal(page, "agent-1");
      await expectWheelMovesScrollback(page, "agent-1");
    }
  });

  test("keeps wheel scrollback alive after a terminal_snapshot reset while hidden", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installAgentScrollBackend(page);
    await page.goto(APP_URL);

    await expect(page.locator(".workspace-window[data-id='agent-1']")).toBeVisible();
    await emitTerminalText(page, "agent-1", numberedLines("before-snapshot", 120));
    await waitForScrollableTerminal(page, "agent-1");

    await setActiveAgentTab(page, "agent-2");
    await expect(page.locator(".workspace-window[data-id='agent-1']")).toBeHidden();
    await emitTerminalSnapshot(page, "agent-1", numberedLines("snapshot-replay", 220));
    await expectPendingRefresh(page, "agent-1", true);

    await setActiveAgentTab(page, "agent-1");
    await expect(page.locator(".workspace-window[data-id='agent-1']")).toBeVisible();
    await expectPendingRefresh(page, "agent-1", false);
    await waitForScrollableTerminal(page, "agent-1");
    await expectWheelMovesScrollback(page, "agent-1");
  });
});

async function waitForScrollableTerminal(page: Page, windowId: string): Promise<void> {
  await expect.poll(() => terminalScrollMetrics(page, windowId)).toMatchObject({
    hasScrollback: true,
    hidden: false,
  });
  await waitForTerminalBufferIdle(page, windowId);
}

async function expectPendingRefresh(
  page: Page,
  windowId: string,
  expected: boolean,
): Promise<void> {
  await expect
    .poll(async () => {
      const metrics = await terminalScrollMetrics(page, windowId);
      return metrics.viewportRefreshPending;
    })
    .toBe(expected);
}

async function expectWheelMovesScrollback(page: Page, windowId: string): Promise<void> {
  await scrollTerminalToBottom(page, windowId);
  await expect
    .poll(async () => {
      const metrics = await terminalScrollMetrics(page, windowId);
      return metrics.viewportY === metrics.baseY;
    })
    .toBe(true);
  const before = await terminalScrollMetrics(page, windowId);
  expect(before.hasScrollback).toBe(true);

  const root = page.locator(`.workspace-window[data-id='${windowId}'] .terminal-root`);
  const box = await root.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(Math.round(box!.x + box!.width / 2), Math.round(box!.y + box!.height / 2));
  await page.mouse.wheel(0, -900);

  await expect
    .poll(async () => {
      const after = await terminalScrollMetrics(page, windowId);
      return after.viewportY;
    })
    .toBeLessThan(before.viewportY);
}

async function waitForTerminalBufferIdle(page: Page, windowId: string): Promise<void> {
  let previousKey = "";
  let stableSamples = 0;
  await expect
    .poll(async () => {
      const metrics = await terminalScrollMetrics(page, windowId);
      const key = [
        metrics.hidden,
        metrics.baseY,
        metrics.bufferLength,
        metrics.viewportRefreshPending,
      ].join(":");
      if (
        key === previousKey &&
        metrics.hidden === false &&
        metrics.hasScrollback === true &&
        metrics.viewportRefreshPending === false
      ) {
        stableSamples += 1;
      } else {
        stableSamples = 0;
      }
      previousKey = key;
      return stableSamples;
    })
    .toBeGreaterThanOrEqual(2);
}

async function terminalScrollMetrics(page: Page, windowId: string): Promise<{
  hidden: boolean;
  viewportY: number;
  baseY: number;
  rows: number;
  bufferLength: number;
  hasScrollback: boolean;
  viewportRefreshPending: boolean;
}> {
  return page.evaluate((id) => {
    const windowElement = document.querySelector<HTMLElement>(`.workspace-window[data-id='${id}']`);
    const metrics = window.__gwtTerminalTestApi?.metrics(id);
    const baseY = metrics?.baseY ?? 0;
    const rows = metrics?.rows ?? 0;
    return {
      hidden: windowElement?.hidden ?? true,
      viewportY: metrics?.viewportY ?? 0,
      baseY,
      rows,
      bufferLength: metrics?.bufferLength ?? 0,
      hasScrollback: baseY > 0 && baseY + rows > rows,
      viewportRefreshPending: metrics?.viewportRefreshPending ?? false,
    };
  }, windowId);
}

async function scrollTerminalToBottom(page: Page, windowId: string): Promise<void> {
  await page.evaluate((id) => {
    window.__gwtTerminalTestApi?.scrollToBottom(id);
  }, windowId);
}

async function emitTerminalText(page: Page, windowId: string, text: string): Promise<void> {
  await emitBackendEvent(page, {
    kind: "terminal_output",
    id: windowId,
    data_base64: await base64(page, text),
  });
}

async function emitTerminalSnapshot(page: Page, windowId: string, text: string): Promise<void> {
  await emitBackendEvent(page, {
    kind: "terminal_snapshot",
    id: windowId,
    data_base64: await base64(page, text),
  });
}

async function emitTerminalStatus(
  page: Page,
  windowId: string,
  status: string,
  detail: string,
): Promise<void> {
  await emitBackendEvent(page, {
    kind: "terminal_status",
    id: windowId,
    status,
    detail,
  });
}

async function setActiveAgentTab(page: Page, windowId: "agent-1" | "agent-2"): Promise<void> {
  await page.evaluate((id) => {
    window.__agentScrollFixture?.setActiveWindow(id);
  }, windowId);
}

async function emitBackendEvent(page: Page, payload: unknown): Promise<void> {
  await page.evaluate((event) => {
    window.__agentScrollFixture?.emit(event);
  }, payload);
}

async function base64(page: Page, text: string): Promise<string> {
  return page.evaluate((value) => btoa(value), text);
}

function numberedLines(prefix: string, count: number): string {
  return Array.from({ length: count }, (_, index) => `${prefix} ${String(index + 1).padStart(3, "0")}`)
    .join("\r\n")
    .concat("\r\n");
}

async function installAgentScrollBackend(page: Page): Promise<void> {
  await page.addInitScript(() => {
    window.__gwtPlaywrightTestBridge = true;

    function agentWindow(id, active, x, zIndex) {
      return {
        id,
        title: id === "agent-1" ? "Codex Plan" : "Claude Review",
        preset: "agent",
        geometry: { x, y: 110, width: 680, height: 440 },
        geometry_revision: 0,
        z_index: zIndex,
        status: active ? "ready" : "running",
        minimized: false,
        maximized: false,
        pre_maximize_geometry: null,
        persist: true,
        purpose_title: null,
        dynamic_title: null,
        dynamic_title_detail: null,
        agent_id: id,
        agent_color: null,
        tab_group_id: "agent-scroll-group",
        tab_group_active: active,
      };
    }

    function workspaceState(activeId) {
      return {
        kind: "workspace_state",
        workspace: {
          app_version: "playwright",
          tabs: [
            {
              id: "tab-1",
              title: "Scroll Fixture",
              project_root: "/fixture",
              kind: "git",
              workspace: {
                viewport: { x: 0, y: 0, zoom: 1 },
                windows: [
                  agentWindow("agent-1", activeId === "agent-1", 120, activeId === "agent-1" ? 3 : 1),
                  agentWindow("agent-2", activeId === "agent-2", 120, activeId === "agent-2" ? 3 : 1),
                ],
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
        this.recordedSends = [];
        window.__agentScrollFixture.instance = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(workspaceState("agent-1"));
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

    window.__agentScrollFixture = {
      instance: null,
      emit(payload) {
        this.instance?.emit(payload);
      },
      setActiveWindow(id) {
        this.instance?.emit(workspaceState(id));
      },
    };

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}

declare global {
  interface Window {
    __agentScrollFixture?: {
      instance: unknown;
      emit(payload: unknown): void;
      setActiveWindow(id: "agent-1" | "agent-2"): void;
    };
    __gwtPlaywrightTestBridge?: boolean;
    __gwtTerminalTestApi?: {
      metrics(windowId: string): {
        baseY: number;
        rows: number;
        viewportY: number;
        viewportRefreshPending: boolean;
      };
      scrollToBottom(windowId: string): void;
    };
  }
}
