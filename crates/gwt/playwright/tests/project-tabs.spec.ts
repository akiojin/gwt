import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Project tabs", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("tab switching stays responsive while streamed WebSocket output is backlogged", async ({
    page,
  }) => {
    const burstSize = 500;
    const streamedStateBoundary = 32;
    await installEmbeddedRoutes(page);
    await installProjectTabsBackend(page, projectTabsFixture(12, {
      hotAgentWindowId: "agent-burst",
    }));

    await page.goto(APP_URL);
    await expect(page.locator(".project-tab")).toHaveCount(12, {
      timeout: 10_000,
    });
    const first = page.locator(".project-tab").nth(0);
    const second = page.locator(".project-tab").nth(1);
    await expect(first).toHaveAttribute("aria-current", "page");

    expect(burstSize / streamedStateBoundary).toBeGreaterThanOrEqual(10);
    await page.evaluate(
      ({ count, windowId }) => {
        const socket = window.__gwtProjectTabsFixtureSocket;
        if (
          !socket ||
          typeof socket.emitTerminalOutputBurstSync !== "function"
        ) {
          throw new Error("project tabs fixture socket burst helper is missing");
        }
        socket.emitTerminalOutputBurstSync({ count, windowId });
      },
      { count: burstSize, windowId: "agent-burst" },
    );

    const start = await page.evaluate(() => performance.now());
    await second.click();
    await expect(second).toHaveAttribute("aria-current", "page", {
      timeout: 1_000,
    });
    const latencyMs = await page.evaluate((startedAt) => {
      return performance.now() - startedAt;
    }, start);

    expect(latencyMs).toBeLessThan(1_000);
    test.info().annotations.push({
      type: "measurement",
      description:
        `tab switch latency under ${burstSize} streamed events: ` +
        `${latencyMs.toFixed(1)}ms`,
    });
    console.log(
      `[project-tabs] high-load tab switch latency=${latencyMs.toFixed(1)}ms ` +
        `burst=${burstSize} streamed_state_boundary=${streamedStateBoundary}`,
    );
  });

  test("tab switching under streamed output stays within CPU and heap budgets", async ({
    page,
  }) => {
    const burstSize = 500;
    const streamedStateBoundary = 32;
    const latencyBudgetMs = 1_000;
    const longTaskBudgetMs = 100;
    const rafGapBudgetMs = 250;
    const heapDriftBudgetBytes = 32 * 1024 * 1024;
    await installEmbeddedRoutes(page);
    await installProjectTabsBackend(page, projectTabsFixture(12, {
      hotAgentWindowId: "agent-burst",
    }));

    await page.goto(APP_URL);
    await expect(page.locator(".project-tab")).toHaveCount(12, {
      timeout: 10_000,
    });
    const first = page.locator(".project-tab").nth(0);
    const second = page.locator(".project-tab").nth(1);
    await expect(first).toHaveAttribute("aria-current", "page");

    const heapBefore = await sampleBrowserHeap(page);
    await runPaletteCommand(page, "Start UI Trace");
    expect(burstSize / streamedStateBoundary).toBeGreaterThanOrEqual(10);
    await page.evaluate(
      ({ count, windowId }) => {
        const socket = window.__gwtProjectTabsFixtureSocket;
        if (
          !socket ||
          typeof socket.emitTerminalOutputBurstSync !== "function"
        ) {
          throw new Error("project tabs fixture socket burst helper is missing");
        }
        socket.emitTerminalOutputBurstSync({ count, windowId });
      },
      { count: burstSize, windowId: "agent-burst" },
    );

    const start = await page.evaluate(() => performance.now());
    await second.click();
    await expect(second).toHaveAttribute("aria-current", "page", {
      timeout: latencyBudgetMs,
    });
    const latencyMs = await page.evaluate((startedAt) => {
      return performance.now() - startedAt;
    }, start);
    await page.waitForTimeout(100);
    const tracePayload = await stopUiTraceViaPalette(page);
    const heapAfter = await sampleBrowserHeap(page);
    const trace = tracePayload?.trace;
    expect(
      trace,
      "fixture socket should capture the UI trace save payload",
    ).toBeTruthy();

    const entries = trace.entries ?? [];
    const terminalMessages = entries.filter(
      (entry) =>
        entry.kind === "ws_message" &&
        entry.event_kind === "terminal_output",
    );
    const overBudgetLongTasks = entries.filter(
      (entry) =>
        entry.kind === "long_task" &&
        Number(entry.duration_ms ?? 0) > longTaskBudgetMs,
    );
    const overBudgetRafGaps = entries.filter(
      (entry) =>
        entry.kind === "raf_gap" &&
        Number(entry.gap_ms ?? 0) > rafGapBudgetMs,
    );
    const heapDriftBytes =
      heapBefore.supported && heapAfter.supported
        ? heapAfter.usedJSHeapSize - heapBefore.usedJSHeapSize
        : null;

    expect(latencyMs).toBeLessThan(latencyBudgetMs);
    expect(terminalMessages.length).toBeGreaterThanOrEqual(burstSize);
    expect(overBudgetLongTasks).toEqual([]);
    expect(overBudgetRafGaps).toEqual([]);
    if (heapDriftBytes !== null) {
      expect(heapDriftBytes).toBeLessThan(heapDriftBudgetBytes);
    }

    const memorySummary =
      heapDriftBytes === null
        ? "memory=unsupported"
        : `heap_drift=${heapDriftBytes}`;
    test.info().annotations.push({
      type: "measurement",
      description:
        `tab switch latency=${latencyMs.toFixed(1)}ms ` +
        `long_tasks=${overBudgetLongTasks.length} ` +
        `raf_gaps=${overBudgetRafGaps.length} ${memorySummary}`,
    });
    console.log(
      `[project-tabs] budget latency=${latencyMs.toFixed(1)}ms ` +
        `ws_terminal_messages=${terminalMessages.length} ` +
        `long_tasks_over_${longTaskBudgetMs}ms=${overBudgetLongTasks.length} ` +
        `raf_gaps_over_${rafGapBudgetMs}ms=${overBudgetRafGaps.length} ` +
        `${memorySummary} burst=${burstSize} ` +
        `streamed_state_boundary=${streamedStateBoundary}`,
    );
  });

  test("many project tabs keep project actions visible and remain switchable", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installProjectTabsBackend(page, 12);

    await page.goto(APP_URL);
    await expect(page.locator(".project-tab")).toHaveCount(12, {
      timeout: 10_000,
    });
    await expect(page.locator("#app-version")).toBeVisible();
    await expect(page.locator("#open-project-button")).toBeVisible();

    const layout = await page.evaluate(() => {
      const rectOf = (selector: string) => {
        const element = document.querySelector(selector);
        if (!element) return null;
        const rect = element.getBoundingClientRect();
        return {
          x: rect.x,
          y: rect.y,
          width: rect.width,
          height: rect.height,
          right: rect.right,
        };
      };
      const tabs = document.querySelector("#project-tabs");
      return {
        viewportWidth: window.innerWidth,
        tabs: rectOf("#project-tabs"),
        actions: rectOf(".project-actions"),
        openProject: rectOf("#open-project-button"),
        version: rectOf("#app-version"),
        tabsClientWidth: tabs?.clientWidth ?? 0,
        tabsScrollWidth: tabs?.scrollWidth ?? 0,
      };
    });

    expect(layout.actions?.right).toBeLessThanOrEqual(layout.viewportWidth);
    expect(layout.openProject?.right).toBeLessThanOrEqual(layout.viewportWidth);
    expect(layout.version?.right).toBeLessThanOrEqual(layout.viewportWidth);
    expect(layout.tabs?.right).toBeLessThanOrEqual(layout.actions?.x ?? 0);
    expect(layout.tabsScrollWidth).toBeGreaterThan(layout.tabsClientWidth);

    const first = page.locator(".project-tab").nth(0);
    const second = page.locator(".project-tab").nth(1);
    await first.click();
    await expect(first).toHaveAttribute("aria-current", "page");
    await second.click();
    await expect(second).toHaveAttribute("aria-current", "page");
    await expect(first).not.toHaveAttribute("aria-current", "page");
  });

  test("project tab dot blinks only when the project has a running agent", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installProjectTabsBackend(page, [
      {
        id: "tab-running",
        title: "Running Agent",
        project_root: "/fixture/running-agent",
        kind: "git",
        workspace: {
          viewport: { x: 0, y: 0, zoom: 1 },
          windows: [{ id: "agent-running", preset: "codex", status: "running" }],
        },
      },
      {
        id: "tab-no-agent",
        title: "Shell Only",
        project_root: "/fixture/shell-only",
        kind: "git",
        workspace: {
          viewport: { x: 0, y: 0, zoom: 1 },
          windows: [{ id: "shell-running", preset: "shell", status: "running" }],
        },
      },
    ]);

    await page.goto(APP_URL);

    const runningDot = page.locator(
      '[data-project-tab-id="tab-running"] [data-role="project-tab-dot"]',
    );
    const shellOnlyDot = page.locator(
      '[data-project-tab-id="tab-no-agent"] [data-role="project-tab-dot"]',
    );

    await expect(runningDot).toHaveAttribute("data-state", "running");
    await expect(shellOnlyDot).toHaveAttribute("data-state", "");
    await expect(runningDot).toHaveCSS(
      "animation-name",
      "project-tab-agent-running-pulse",
    );
  });
});

async function runPaletteCommand(page, query: string) {
  await page.locator("#op-palette-button").click();
  const input = page.locator("#op-palette-input");
  await expect(input).toBeVisible();
  await input.fill(query);
  await page.keyboard.press("Enter");
  await expect(page.locator("#op-palette-backdrop")).not.toHaveAttribute(
    "data-open",
    "true",
  );
}

async function stopUiTraceViaPalette(page) {
  await runPaletteCommand(page, "Stop UI Trace");
  return await page.evaluate(() => {
    const socket = window.__gwtProjectTabsFixtureSocket;
    return socket?.savedUiTracePayload ?? null;
  });
}

async function sampleBrowserHeap(page) {
  return await page.evaluate(() => {
    const memory = performance.memory;
    if (!memory || typeof memory.usedJSHeapSize !== "number") {
      return { supported: false };
    }
    return {
      supported: true,
      usedJSHeapSize: memory.usedJSHeapSize,
    };
  });
}

function projectTabsFixture(
  count: number,
  { hotAgentWindowId }: { hotAgentWindowId?: string } = {},
) {
  return Array.from({ length: count }, (_, index) => {
    const number = String(index + 1).padStart(2, "0");
    return {
      id: `tab-${number}`,
      title: `known-project-${number}`,
      project_root: `/fixture/known-project-${number}`,
      kind: "git",
      workspace: {
        viewport: { x: 0, y: 0, zoom: 1 },
        windows:
          index === 0 && hotAgentWindowId
            ? [
                {
                  id: hotAgentWindowId,
                  title: "Burst Agent",
                  preset: "codex",
                  status: "running",
                  geometry: { x: 96, y: 96, width: 720, height: 420 },
                  z_index: 1,
                },
              ]
            : [],
      },
    };
  });
}

async function installProjectTabsBackend(page, tabFixture: number | unknown[]) {
  await page.addInitScript((fixture) => {
    const tabs = Array.isArray(fixture)
      ? fixture
      : Array.from({ length: fixture }, (_, index) => {
          const number = String(index + 1).padStart(2, "0");
          return {
            id: `tab-${number}`,
            title: `known-project-${number}`,
            project_root: `/fixture/known-project-${number}`,
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [],
            },
          };
        });
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs,
        active_tab_id: tabs[0]?.id ?? null,
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
        window.__gwtProjectTabsFixtureSocket = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw) {
        let message;
        try {
          message = JSON.parse(raw);
        } catch {
          return;
        }
        if (message.kind === "frontend_ready") {
          this.emit(workspaceState);
          return;
        }
        if (message.kind === "save_ui_trace") {
          this.savedUiTracePayload = message;
          return;
        }
        if (
          message.kind === "select_project_tab" &&
          tabs.some((tab) => tab.id === message.tab_id)
        ) {
          workspaceState.workspace.active_tab_id = message.tab_id;
          this.emitSync(workspaceState);
        }
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload) {
        setTimeout(() => {
          this.emitSync(payload);
        }, 0);
      }

      emitSync(payload) {
        this.dispatchEvent(
          new MessageEvent("message", { data: JSON.stringify(payload) }),
        );
      }

      emitTerminalOutputBurstSync({ count, windowId }) {
        const data_base64 = btoa("gwt responsiveness burst\\r\\n");
        for (let i = 0; i < count; i += 1) {
          this.emitSync({
            kind: "terminal_output",
            id: windowId,
            data_base64,
          });
        }
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  }, tabFixture);
}
