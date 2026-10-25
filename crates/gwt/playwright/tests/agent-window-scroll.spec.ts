import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { APP_URL, APP_PROJECT_KEY, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { liveGwtProjectUrl } from "./_helpers/live-gwt";

const liveBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
const appUrl = liveBase
  ? new URL(liveBase).pathname === "/" ? liveGwtProjectUrl(liveBase, APP_PROJECT_KEY) : liveBase
  : APP_URL;

test.describe("Agent window scrollback lifecycle", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("keeps wheel scrollback alive after hidden-visible cycles and Plan-mode idle", async ({ page }, info) => {
    const errors = await openAgentsFixture(page, info.project.name);

    await expect(agentTile(page, "agent-1")).toBeVisible();
    await emitTerminalText(page, "agent-1", numberedLines("plan-cycle", 180));
    await emitTerminalStatus(page, "agent-1", "ready", "Plan mode input wait");
    await waitForScrollableTerminal(page, "agent-1");

    for (let index = 0; index < 5; index += 1) {
      await setActiveAgentTab(page, "agent-2");
      await expect(agentTile(page, "agent-1")).toBeHidden();
      await emitTerminalText(page, "agent-1", numberedLines(`hidden-${index}`, 20));
      // Inert native caches retain a measured grid while another work is full.
      // Output must parse immediately so visible Issue mirrors remain live.
      await expect.poll(() => page.evaluate(() => window.__gwtTerminalTestApi!.bufferText("agent-1")))
        .toContain(`hidden-${index} 020`);
      await expectPendingRefresh(page, "agent-1", false);

      await setActiveAgentTab(page, "agent-1");
      await expect(agentTile(page, "agent-1")).toBeVisible();
      await expectPendingRefresh(page, "agent-1", false);
      await waitForScrollableTerminal(page, "agent-1");
      await expectWheelMovesScrollback(page, "agent-1");
    }
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("keeps wheel scrollback alive after a terminal_snapshot reset while hidden", async ({ page }, info) => {
    const errors = await openAgentsFixture(page, info.project.name);

    await expect(agentTile(page, "agent-1")).toBeVisible();
    await emitTerminalText(page, "agent-1", numberedLines("before-snapshot", 120));
    await waitForScrollableTerminal(page, "agent-1");

    await setActiveAgentTab(page, "agent-2");
    await expect(agentTile(page, "agent-1")).toBeHidden();
    await emitTerminalSnapshot(page, "agent-1", numberedLines("snapshot-replay", 220));
    await expect.poll(() => page.evaluate(() => window.__gwtTerminalTestApi!.bufferText("agent-1")))
      .toContain("snapshot-replay 220");
    await expectPendingRefresh(page, "agent-1", false);

    await setActiveAgentTab(page, "agent-1");
    await expect(agentTile(page, "agent-1")).toBeVisible();
    await expectPendingRefresh(page, "agent-1", false);
    await waitForScrollableTerminal(page, "agent-1");
    await expectWheelMovesScrollback(page, "agent-1");
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("surfaces runtime error detail without covering the terminal with an overlay", async ({ page }, info) => {
    const errors = await openAgentsFixture(page, info.project.name);

    await expect(agentTile(page, "agent-1")).toBeVisible();
    await emitTerminalStatus(page, "agent-1", "error", "Stop-block hit an error");

    // SPEC-2014 US-36 (FR-120/121): the raw TTY must never be covered by a
    // foreground status/error overlay. Error detail surfaces in the window
    // status strip and the attention toast instead of an xterm overlay.
    const overlay = agentTile(page, "agent-1").locator(".terminal-overlay");
    await expect(overlay).toBeHidden();
    await expect(page.locator("#op-strip-error")).toHaveText("1");
    // SPEC #3206: attention renders in the shared alerts stack.
    await expect(page.locator(".toast-alerts__item").first()).toContainText("Stop-block hit an error");
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("drag selection follows rendered rows before and after scrollback and snapshot replay", async ({ page }, info) => {
    if (liveBase) await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
    else await page.addInitScript(() => {
      (window as any).__selectionCopy = "";
      (window as any).__nativeCopyEvents = [];
      // Xterm's native Cmd+C uses ClipboardEvent, while menu/drag copy uses
      // navigator.clipboard. Feed both real transports into the fixture shim.
      document.addEventListener("copy", event => {
        const text = event.clipboardData?.getData("text/plain") ?? "";
        (window as any).__nativeCopyEvents.push(text);
        (window as any).__selectionCopy = text;
      });
      Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
        writeText: async (text: string) => { (window as any).__selectionCopy = text; },
        readText: async () => (window as any).__selectionCopy,
      } });
    });
    const errors = await openAgentsFixture(page, info.project.name);
    await emitTerminalText(page, "agent-1", numberedLines("selection", 240));
    await waitForScrollableTerminal(page, "agent-1");
    await expectDragCopiesRenderedRows(page, info, "bottom");
    await expectWheelMovesScrollback(page, "agent-1");
    await expectDragCopiesRenderedRows(page, info, "scrollback");
    await setActiveAgentTab(page, "agent-2");
    await emitTerminalSnapshot(page, "agent-1", numberedLines("snapshot", 240));
    await setActiveAgentTab(page, "agent-1");
    await expect(agentTile(page, "agent-1").locator(".xterm-rows")).toContainText("snapshot");
    await waitForScrollableTerminal(page, "agent-1");
    await expectDragCopiesRenderedRows(page, info, "snapshot");
    expect(errors, "zero console/page errors").toEqual([]);
  });
});

async function expectDragCopiesRenderedRows(page: Page, info: TestInfo, phase: string): Promise<void> {
  const selection = await agentTile(page, "agent-1").locator(".xterm-screen").evaluate(screen => {
    const rows = [...screen.querySelectorAll<HTMLElement>(".xterm-rows > div")];
    const start = rows[2].getBoundingClientRect();
    const end = rows[7].getBoundingClientRect();
    const cols = (window.__gwtTerminalTestApi!.metrics("agent-1") as any).cols;
    const cellWidth = screen.getBoundingClientRect().width / cols;
    return { x1: start.x + 1, y1: start.y + start.height / 2,
      x2: end.x + (rows[7].textContent!.trimEnd().length + 0.1) * cellWidth,
      y2: end.y + end.height / 2,
      top: start.top, bottom: end.bottom,
      text: rows.slice(2, 8).map(row => row.textContent!.trimEnd()).join("\n") };
  });
  await page.mouse.move(selection.x1, selection.y1);
  await page.mouse.down();
  await page.mouse.move(selection.x2, selection.y2, { steps: 12 });
  await page.mouse.up();
  const highlight = agentTile(page, "agent-1").locator(".xterm-selection div");
  await expect(highlight.first()).toBeVisible();
  const first = await highlight.first().boundingBox();
  const last = await highlight.last().boundingBox();
  expect(Math.abs(first!.y - selection.top), "selection starts on dragged row").toBeLessThan(1);
  expect(Math.abs(last!.y + last!.height - selection.bottom), "selection ends on dragged row").toBeLessThan(1);
  await agentTile(page, "agent-1").screenshot({ path: info.outputPath(`selection-${phase}.png`) });
  await page.evaluate(() => navigator.clipboard.writeText("clipboard reset"));
  await page.mouse.click(selection.x1, selection.y1, { button: "right" });
  await page.getByRole("menuitem", { name: "Copy", exact: true }).click();
  await expect.poll(() => page.evaluate(async () => (await navigator.clipboard.readText()).replace(/\r\n/g, "\n")))
    .toBe(selection.text);
  await page.evaluate(() => navigator.clipboard.writeText("clipboard reset"));
  // Match the browser platform used by classifyTerminalCopyKeyEvent. macOS
  // intentionally leaves Ctrl-based keys on the PTY path and copies with Cmd+C.
  const platform = await page.evaluate(() => (navigator as any).userAgentData?.platform || navigator.platform || "");
  const shortcut = /mac|iphone|ipad|ipod/i.test(platform) ? "Meta+c" : /win/i.test(platform) ? "Control+c" : "Control+Shift+c";
  await info.attach(`copy-shortcut-${phase}`, { body: JSON.stringify({ platform, shortcut }), contentType: "application/json" });
  const nativeCopiesBefore = await page.evaluate(() => (window as any).__nativeCopyEvents?.length || 0);
  await page.keyboard.press(shortcut);
  if (!liveBase && /mac|iphone|ipad|ipod/i.test(platform)) {
    await expect.poll(() => page.evaluate(() => (window as any).__nativeCopyEvents.length)).toBeGreaterThan(nativeCopiesBefore);
    expect(await page.evaluate(() => (window as any).__nativeCopyEvents.at(-1))).toBe(selection.text);
  }
  await expect.poll(() => page.evaluate(async () => (await navigator.clipboard.readText()).replace(/\r\n/g, "\n")))
    .toBe(selection.text);
}

function agentTile(page: Page, windowId: string) {
  return page.locator(`.works-surface:not(.works-surface--preview) .works-terminal[data-window-id='${windowId}']`);
}

async function openAgentsFixture(page: Page, projectName: string): Promise<string[]> {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = projectName.includes("light") ? "light" : "dark";
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  if (!liveBase) await installEmbeddedRoutes(page);
  await installAgentScrollBackend(page);
  await page.goto(appUrl);
  await page.getByRole("button", { name: "Works", exact: true }).click();
  await setActiveAgentTab(page, "agent-1");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  return errors;
}

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

  const root = agentTile(page, windowId);
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
    const tile = document.querySelector<HTMLElement>(`.works-surface:not(.works-surface--preview) .works-terminal[data-window-id='${id}']`);
    const metrics = window.__gwtTerminalTestApi?.metrics(id);
    const baseY = metrics?.baseY ?? 0;
    const rows = metrics?.rows ?? 0;
    return {
      hidden: !tile || !tile.getClientRects().length || getComputedStyle(tile).visibility === "hidden",
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
  const back = page.locator('.works-full:not([hidden]) [data-action="back"]');
  if (await back.isVisible()) await back.click();
  await page.locator(`.work-row[data-window-id="${windowId}"] .work-open`).click();
  await expect(agentTile(page, windowId)).toBeVisible();
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
        agent_name: id === "agent-1" ? "Codex Plan" : "Claude Review",
        session_id: `${id}-session`,
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
              project_key: location.pathname.split("/").filter(Boolean).at(-1),
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
          this.emit({ kind: "active_work_projection", projection: { id: "fixture-local-live", active_works: [], local_agent_activity: { project_key: location.pathname.split("/").filter(Boolean).at(-1), host_id: "scroll-fixture", observed_at: new Date().toISOString(), agents: [
          { window_id: 'agent-1', session_id: 'agent-1-session', runtime_incarnation: 1, is_pm: false, issue_number: null },
          { window_id: 'agent-2', session_id: 'agent-2-session', runtime_incarnation: 1, is_pm: false, issue_number: null }
        ] } } });
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
    };
    __gwtPlaywrightTestBridge?: boolean;
    __gwtTerminalTestApi?: {
      bufferText(windowId: string): string;
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
