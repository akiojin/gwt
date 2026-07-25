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
    const terminalReceives = entries.filter(
      (entry) => entry.kind === "terminal_output_ws_receive",
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
    const retainedTerminalIngressMarkers =
      terminalMessages.length + terminalReceives.length;
    expect(
      retainedTerminalIngressMarkers,
      "the trace must retain content-free evidence of the emitted terminal burst",
    ).toBeGreaterThan(0);
    expect(
      retainedTerminalIngressMarkers + Number(trace.dropped_entries ?? 0),
      "retained plus explicitly dropped bounded-trace entries must cover the burst",
    ).toBeGreaterThanOrEqual(burstSize);
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
        `terminal_receive_markers=${terminalReceives.length} ` +
        `trace_dropped=${trace.dropped_entries ?? 0} ` +
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
    // SPEC-2013 Phase 8 (518f7a10b) — the Open Project split-button was retired
    // and project intake/switching consolidated into the `Projects ▾` switcher.
    // The project action that must stay reachable is now #project-switcher-button.
    await expect(page.locator("#project-switcher-button")).toBeVisible();

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
        projectSwitcher: rectOf("#project-switcher-button"),
        version: rectOf("#app-version"),
        tabsClientWidth: tabs?.clientWidth ?? 0,
        tabsScrollWidth: tabs?.scrollWidth ?? 0,
      };
    });

    expect(layout.actions?.right).toBeLessThanOrEqual(layout.viewportWidth);
    expect(layout.projectSwitcher?.right).toBeLessThanOrEqual(layout.viewportWidth);
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

  test("project tab becomes active locally while the backend result is held", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installProjectTabsBackend(page, 2);

    await page.goto(APP_URL);
    await expect(page.locator(".project-tab")).toHaveCount(2);
    await page.evaluate(() => {
      window.__gwtProjectTabsFixtureSocket.holdNavigation = true;
    });

    const second = page.locator('[data-project-tab-id="tab-02"]');
    await second.click();

    await expect(second).toHaveAttribute("aria-current", "page");
    const request = await page.evaluate(() => {
      return window.__gwtProjectTabsFixtureSocket.sentMessages
        .filter((message) => message.kind === "select_project_tab")
        .at(-1);
    });
    expect(request).toMatchObject({
      kind: "select_project_tab",
      tab_id: "tab-02",
    });
    expect(request.interaction_id).toMatch(/^navigation-/);
    await page.waitForTimeout(100);
    await expect(second).toHaveAttribute("aria-current", "page");
  });

  test("rapid project A to B to A never flickers when acknowledgements reverse", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installProjectTabsBackend(page, 2);

    await page.goto(APP_URL);
    await expect(page.locator(".project-tab")).toHaveCount(2);
    await page.evaluate(() => {
      window.__gwtProjectTabsFixtureSocket.holdNavigation = true;
    });

    const first = page.locator('[data-project-tab-id="tab-01"]');
    const second = page.locator('[data-project-tab-id="tab-02"]');
    await second.click();
    await first.click();
    await expect(first).toHaveAttribute("aria-current", "page");

    const requests = await page.evaluate(() => {
      return window.__gwtProjectTabsFixtureSocket.sentMessages.filter(
        (message) => message.kind === "select_project_tab",
      );
    });
    expect(requests).toHaveLength(2);
    expect(requests.map((request) => request.tab_id)).toEqual([
      "tab-02",
      "tab-01",
    ]);

    await page.evaluate((latestRequest) => {
      window.__gwtProjectTabsFixtureSocket.emitSync({
        kind: "navigation_result",
        interaction_id: latestRequest.interaction_id,
        revision: 2,
        scope: "project_tab",
        outcome: "accepted",
        canonical: {
          active_tab_id: "tab-01",
          target_id: "tab-01",
          window_updates: [],
        },
      });
    }, requests[1]);
    await page.evaluate(() => new Promise(requestAnimationFrame));
    await expect(
      first,
      "settling the latest A must not replay the older pending B",
    ).toHaveAttribute("aria-current", "page");

    await page.evaluate((olderRequest) => {
      window.__gwtProjectTabsFixtureSocket.emitSync({
        kind: "navigation_result",
        interaction_id: olderRequest.interaction_id,
        revision: 1,
        scope: "project_tab",
        outcome: "accepted",
        canonical: {
          active_tab_id: "tab-02",
          target_id: "tab-02",
          window_updates: [],
        },
      });
    }, requests[0]);
    await page.evaluate(() => new Promise(requestAnimationFrame));
    await expect(
      first,
      "the late lower-revision B acknowledgement must not roll A back",
    ).toHaveAttribute("aria-current", "page");
  });

  test("shared backend orders client B workspace before client A matching result", async ({
    context,
    page,
  }, testInfo) => {
    const sharedBackendId =
      `project-tabs-${testInfo.workerIndex}-${Date.now()}-${Math.random()}`;
    await installEmbeddedRoutes(page);
    await installProjectTabsBackend(page, 2, {
      sharedBackendId,
      clientId: "client-a",
    });
    const peer = await context.newPage();
    try {
      await installEmbeddedRoutes(peer);
      await installProjectTabsBackend(peer, 2, {
        sharedBackendId,
        clientId: "client-b",
      });
      await Promise.all([page.goto(APP_URL), peer.goto(APP_URL)]);
      await Promise.all([
        expect(page.locator(".project-tab")).toHaveCount(2),
        expect(peer.locator(".project-tab")).toHaveCount(2),
      ]);
      await page.evaluate(() => {
        window.__gwtProjectTabsFixtureSocket.holdNavigation = true;
      });

      await page.locator('[data-project-tab-id="tab-02"]').click();
      const request = await page.evaluate(() => {
        return window.__gwtProjectTabsFixtureSocket.sentMessages
          .filter((message) => message.kind === "select_project_tab")
          .at(-1);
      });
      expect(request).toMatchObject({
        kind: "select_project_tab",
        tab_id: "tab-02",
      });
      expect(request.interaction_id).toMatch(/^navigation-/);
      await expect(
        page.locator('[data-project-tab-id="tab-02"]'),
      ).toHaveAttribute("aria-current", "page");
      await page.evaluate(() => {
        const root = document.querySelector(".project-tabs");
        if (!root) {
          throw new Error("project tabs root is missing");
        }
        const activeTabId = () =>
          root.querySelector('[data-project-tab-id][aria-current="page"]')
            ?.getAttribute("data-project-tab-id") ?? null;
        window.__gwtProjectTabsActiveHistory = [activeTabId()];
        window.__gwtProjectTabsActiveObserver = new MutationObserver(() => {
          window.__gwtProjectTabsActiveHistory.push(activeTabId());
        });
        window.__gwtProjectTabsActiveObserver.observe(root, {
          attributes: true,
          attributeFilter: ["aria-current"],
          childList: true,
          subtree: true,
        });
      });

      await peer.locator('[data-project-tab-id="tab-02"]').click();
      await peer.locator('[data-project-tab-id="tab-01"]').click();
      await expect.poll(async () => {
        return await page.evaluate(() => {
          return window.__gwtProjectTabsFixtureSocket
            .receivedSharedWorkspaces.at(-1);
        });
      }).toMatchObject({
        source_client_id: "client-b",
        revision: 2,
        active_tab_id: "tab-01",
      });
      await page.evaluate(
        () =>
          new Promise((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(resolve)),
          ),
      );

      await expect(
        page.locator('[data-project-tab-id="tab-02"]'),
        "client B's newer workspace must not replace client A's pending target",
      ).toHaveAttribute("aria-current", "page");
      await expect(
        page.locator('[data-project-tab-id="tab-01"]'),
      ).not.toHaveAttribute("aria-current", "page");
      const beforeResult = await page.evaluate((interactionId) => {
        const messages =
          window.__gwtProjectTabsFixtureSocket.receivedMessages;
        return {
          remoteWorkspaceIndex: messages.findIndex(
            (message) =>
              message.kind === "workspace_state" &&
              message.revision === 2 &&
              message.workspace?.active_tab_id === "tab-01",
          ),
          matchingResultIndex: messages.findIndex(
            (message) =>
              message.kind === "navigation_result" &&
              message.interaction_id === interactionId,
          ),
          activeHistory: [
            ...window.__gwtProjectTabsActiveHistory,
          ],
        };
      }, request.interaction_id);
      expect(beforeResult.remoteWorkspaceIndex).toBeGreaterThanOrEqual(0);
      expect(beforeResult.matchingResultIndex).toBe(-1);
      expect(beforeResult.activeHistory).not.toContain("tab-01");

      await page.evaluate(() => {
        window.__gwtProjectTabsFixtureSocket.releaseHeldNavigation();
      });
      await expect.poll(async () => {
        return await page.evaluate((interactionId) => {
          return window.__gwtProjectTabsFixtureSocket.receivedMessages.some(
            (message) =>
              message.kind === "navigation_result" &&
              message.interaction_id === interactionId,
          );
        }, request.interaction_id);
      }).toBe(true);
      await page.evaluate(
        () =>
          new Promise((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(resolve)),
          ),
      );

      await Promise.all([
        expect(
          page.locator('[data-project-tab-id="tab-02"]'),
          "client A must keep the optimistic target through its matching result",
        ).toHaveAttribute("aria-current", "page"),
        expect(
          peer.locator('[data-project-tab-id="tab-02"]'),
          "client B must converge on client A's accepted shared state",
        ).toHaveAttribute("aria-current", "page"),
      ]);
      const afterResult = await page.evaluate((interactionId) => {
        window.__gwtProjectTabsActiveObserver.disconnect();
        const messages =
          window.__gwtProjectTabsFixtureSocket.receivedMessages;
        return {
          remoteWorkspaceIndex: messages.findIndex(
            (message) =>
              message.kind === "workspace_state" &&
              message.revision === 2 &&
              message.workspace?.active_tab_id === "tab-01",
          ),
          matchingResultIndex: messages.findIndex(
            (message) =>
              message.kind === "navigation_result" &&
              message.interaction_id === interactionId,
          ),
          activeHistory: [
            ...window.__gwtProjectTabsActiveHistory,
          ],
        };
      }, request.interaction_id);
      expect(afterResult.matchingResultIndex).toBeGreaterThan(
        afterResult.remoteWorkspaceIndex,
      );
      expect(afterResult.activeHistory).not.toContain("tab-01");
    } finally {
      await peer.close();
    }
  });

  test("grouped window tab reveals locally while the backend result is held", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installProjectTabsBackend(page, groupedWindowTabsFixture());

    await page.goto(APP_URL);
    const firstWindow = page.locator(
      '.workspace-window[data-id="window-b1"]',
    );
    const secondWindow = page.locator(
      '.workspace-window[data-id="window-b2"]',
    );
    await expect(firstWindow).toBeVisible();
    await expect(secondWindow).toBeHidden();
    await page.evaluate(() => {
      window.__gwtProjectTabsFixtureSocket.holdNavigation = true;
    });

    await firstWindow
      .locator('.window-tab[data-window-tab-id="window-b2"]')
      .click();

    await expect(secondWindow).toBeVisible();
    await expect(
      secondWindow.locator('.window-tab[data-window-tab-id="window-b2"]'),
    ).toHaveAttribute("aria-current", "page");
    const request = await page.evaluate(() => {
      return window.__gwtProjectTabsFixtureSocket.sentMessages
        .filter((message) => message.kind === "activate_window_tab")
        .at(-1);
    });
    expect(request).toMatchObject({
      kind: "activate_window_tab",
      id: "window-b2",
    });
    expect(request.interaction_id).toMatch(/^navigation-/);
    await page.waitForTimeout(100);
    await expect(secondWindow).toBeVisible();
  });

  test("authoritative window removal retires a pending grouped target", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installProjectTabsBackend(page, groupedWindowTabsFixture());

    await page.goto(APP_URL);
    const firstWindow = page.locator(
      '.workspace-window[data-id="window-b1"]',
    );
    const secondWindow = page.locator(
      '.workspace-window[data-id="window-b2"]',
    );
    await expect(firstWindow).toBeVisible();
    await expect(secondWindow).toBeHidden();
    await page.evaluate(() => {
      window.__gwtProjectTabsFixtureSocket.holdNavigation = true;
    });

    await firstWindow
      .locator('.window-tab[data-window-tab-id="window-b2"]')
      .click();
    await expect(secondWindow).toBeVisible();
    const request = await page.evaluate(() => {
      return window.__gwtProjectTabsFixtureSocket.sentMessages
        .filter((message) => message.kind === "activate_window_tab")
        .at(-1);
    });
    expect(request).toMatchObject({
      kind: "activate_window_tab",
      id: "window-b2",
    });
    expect(request.interaction_id).toMatch(/^navigation-/);

    await page.evaluate(() => {
      const socket = window.__gwtProjectTabsFixtureSocket;
      const authoritative = structuredClone(socket.workspaceState);
      authoritative.revision = 1;
      authoritative.workspace.tabs[0].workspace.windows = authoritative
        .workspace.tabs[0].workspace.windows
        .filter((windowData) => windowData.id !== "window-b2")
        .map((windowData) => ({
          ...windowData,
          tab_group_active: windowData.id === "window-b1",
        }));
      socket.emitSync(authoritative);
    });
    await page.evaluate(() => new Promise(requestAnimationFrame));

    await expect(secondWindow).toHaveCount(0);
    await expect(firstWindow).toBeVisible();
    await expect(
      firstWindow.locator('.window-tab[data-window-tab-id="window-b1"]'),
    ).toHaveAttribute("aria-current", "page");

    await page.evaluate((lateRequest) => {
      const socket = window.__gwtProjectTabsFixtureSocket;
      socket.emitSync({
        kind: "navigation_result",
        interaction_id: lateRequest.interaction_id,
        revision: 2,
        scope: "window_tab",
        outcome: "accepted",
        canonical: {
          active_tab_id: "tab-grouped",
          target_id: "window-b2",
          window_updates: [
            { id: "window-b1", tab_group_active: false },
            { id: "window-b2", tab_group_active: true },
          ],
        },
      });
      const stale = structuredClone(socket.workspaceState);
      stale.revision = 0;
      socket.emitSync(stale);
    }, request);
    await page.evaluate(() => new Promise(requestAnimationFrame));

    await expect(
      secondWindow,
      "late result and stale state must not resurrect the removed target",
    ).toHaveCount(0);
    await expect(firstWindow).toBeVisible();
  });

  test("project tab cue appears only when the project has a running agent", async ({
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

    const runningCue = page.locator(
      '[data-project-tab-id="tab-running"] [data-role="project-tab-state-cue"]',
    );
    const shellOnlyCue = page.locator(
      '[data-project-tab-id="tab-no-agent"] [data-role="project-tab-state-cue"]',
    );

    await expect(runningCue).toHaveAttribute("data-state", "run");
    await expect(runningCue).toHaveText("RUN");
    await expect(runningCue).toHaveAttribute("aria-label", "1 running agent");
    await expect(shellOnlyCue).toHaveAttribute("data-state", "");
    await expect(runningCue).toHaveCSS(
      "animation-name",
      "none",
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

function groupedWindowTabsFixture() {
  return [
    {
      id: "tab-grouped",
      title: "Grouped",
      project_root: "/fixture/grouped",
      kind: "git",
      workspace: {
        viewport: { x: 0, y: 0, zoom: 1 },
        windows: [
          {
            id: "window-b1",
            title: "Branches",
            preset: "branches",
            status: "running",
            geometry: { x: 96, y: 96, width: 720, height: 420 },
            geometry_revision: 1,
            z_index: 1,
            tab_group_id: "group-b",
            tab_group_active: true,
          },
          {
            id: "window-b2",
            title: "Board",
            preset: "board",
            status: "running",
            geometry: { x: 96, y: 96, width: 720, height: 420 },
            geometry_revision: 1,
            z_index: 1,
            tab_group_id: "group-b",
            tab_group_active: false,
          },
        ],
      },
    },
  ];
}

async function installProjectTabsBackend(
  page,
  tabFixture: number | unknown[],
  {
    sharedBackendId = "",
    clientId = "fixture-client",
  }: { sharedBackendId?: string; clientId?: string } = {},
) {
  await page.addInitScript(({ fixture, sharedBackendId, clientId }) => {
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
    let workspaceState = {
      kind: "workspace_state",
      revision: 0,
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
        this.holdNavigation = false;
        this.heldNavigationMessages = [];
        this.sentMessages = [];
        this.receivedMessages = [];
        this.receivedSharedWorkspaces = [];
        this.navigationRevision = 0;
        this.workspaceState = workspaceState;
        this.sharedChannel = sharedBackendId
          ? new BroadcastChannel(`gwt-project-tabs-${sharedBackendId}`)
          : null;
        this.sharedChannel?.addEventListener("message", (event) => {
          const shared = event.data;
          if (
            shared?.kind !== "workspace_state" ||
            shared.source_client_id === clientId
          ) {
            return;
          }
          const incoming = shared.payload;
          if (
            !Number.isSafeInteger(incoming?.revision) ||
            incoming.revision < this.navigationRevision
          ) {
            return;
          }
          workspaceState = structuredClone(incoming);
          this.workspaceState = workspaceState;
          this.navigationRevision = incoming.revision;
          this.receivedSharedWorkspaces.push({
            source_client_id: shared.source_client_id,
            revision: incoming.revision,
            active_tab_id: incoming.workspace?.active_tab_id ?? null,
          });
          this.emitSync(workspaceState);
        });
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
        this.sentMessages.push(message);
        if (message.kind === "frontend_ready") {
          this.emit(workspaceState);
          return;
        }
        if (message.kind === "save_ui_trace") {
          this.savedUiTracePayload = message;
          return;
        }
        if (
          this.holdNavigation &&
          ["select_project_tab", "activate_window_tab", "focus_window"].includes(
            message.kind,
          )
        ) {
          this.heldNavigationMessages.push(message);
          return;
        }
        this.processNavigation(message);
      }

      processNavigation(message) {
        if (
          message.kind === "select_project_tab" &&
          tabs.some((tab) => tab.id === message.tab_id)
        ) {
          const alreadyCurrent =
            workspaceState.workspace.active_tab_id === message.tab_id;
          if (!alreadyCurrent) {
            this.navigationRevision += 1;
            workspaceState.workspace.active_tab_id = message.tab_id;
            workspaceState.revision = this.navigationRevision;
          }
          this.emitSync({
            kind: "navigation_result",
            interaction_id: message.interaction_id,
            revision: this.navigationRevision,
            scope: "project_tab",
            outcome: alreadyCurrent ? "already_current" : "accepted",
            canonical: {
              active_tab_id: message.tab_id,
              target_id: message.tab_id,
              window_updates: [],
            },
          });
          if (alreadyCurrent) {
            return;
          }
          this.emitSync(workspaceState);
          this.sharedChannel?.postMessage({
            kind: "workspace_state",
            source_client_id: clientId,
            payload: structuredClone(workspaceState),
          });
        }
      }

      releaseHeldNavigation() {
        const message = this.heldNavigationMessages.shift();
        if (message) {
          this.processNavigation(message);
        }
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.sharedChannel?.close();
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload) {
        setTimeout(() => {
          this.emitSync(payload);
        }, 0);
      }

      emitSync(payload) {
        this.receivedMessages.push(structuredClone(payload));
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
  }, { fixture: tabFixture, sharedBackendId, clientId });
}
