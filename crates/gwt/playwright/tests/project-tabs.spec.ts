import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

test.describe("Project tabs", () => {
  const browserErrors = new WeakMap<Page, string[]>();
  test.beforeEach(async ({ page }) => {
    browserErrors.set(page, collectBrowserErrors(page));
  });
  test.afterEach(async ({ page }) => {
    expect(browserErrors.get(page)).toEqual([]);
  });
  test.use({ viewport: { width: 1440, height: 900 } });

  test("live checkout bootstraps a project-scoped connection and accepts terminal input", async ({ page }, testInfo) => {
    const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
    test.skip(!base, "requires browser-check isolated checkout server");
    await withLiveGwtBackendLock(base!, testInfo, async () => {
    const errors: string[] = [];
    const sockets: string[] = [];
    let projectKey: string | undefined;
    let scopedWorkspaceReceived = false;
    let backendWindowIds: string[] = [];
    const terminalOutputs = new Map<string, string>();
    const inputFrames: Array<{ scope: string | null; id: string; data: string }> = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    page.on("websocket", (socket) => {
      sockets.push(socket.url());
      socket.on("framesent", ({ payload }) => {
        const event = JSON.parse(String(payload));
        if (event.kind === "terminal_input") inputFrames.push({
          scope: new URL(socket.url()).searchParams.get("repo_hash"),
          id: event.id, data: event.data,
        });
      });
      socket.on("framereceived", ({ payload }) => {
        const event = JSON.parse(String(payload));
        if (event.kind === "terminal_output") {
          terminalOutputs.set(event.id, (terminalOutputs.get(event.id) ?? "")
            + Buffer.from(event.data_base64, "base64").toString("utf8"));
        }
        if (event.kind !== "workspace_state") return;
        const workspace = event.workspace;
        const tab = workspace.tabs.find((entry) => entry.id === workspace.active_tab_id)
          ?? workspace.tabs[0];
        projectKey = tab?.project_key;
        if (projectKey && new URL(socket.url()).searchParams.get("repo_hash") === projectKey) {
          backendWindowIds = (tab.workspace?.windows ?? []).map((entry) => entry.id);
          scopedWorkspaceReceived = true;
        }
      });
    });
    await gotoLiveGwt(page, base!, { enableTestBridge: true });
    await expect(page.locator(".project-tab").first()).toBeVisible();
    await page.locator(".project-tab").first().click();
    await expect.poll(() => scopedWorkspaceReceived).toBe(true);
    expect(projectKey).toMatch(/^[0-9a-f]{16}$/);
    expect(sockets).toHaveLength(2);
    expect(new URL(sockets[0]).searchParams.has("repo_hash")).toBe(false);
    expect(new URL(sockets[1]).searchParams.get("repo_hash")).toBe(projectKey);
    const shellSelector = '.workspace-window[data-preset="shell"]';
    const previousIds = new Set(backendWindowIds);
    await sendLiveGwtEvent(page, {
      kind: "create_window", preset: "shell",
      bounds: { x: 80, y: 80, width: 720, height: 420 },
    });
    const newIds = () => backendWindowIds.filter((id) => !previousIds.has(id));
    await expect.poll(newIds, { timeout: 30_000 }).toHaveLength(1);
    const id = newIds()[0]!;
    const shell = page.locator(`${shellSelector}[data-id="${id}"]`);
    try {
      await expect(shell).toBeVisible();
      await expect.poll(() => page.evaluate((id) =>
        window.__gwtTerminalTestApi.metrics(id).isReady, id)).toBe(true);
      // DOM/xterm readiness precedes shell startup. Wait for its actual prompt
      // before typing: shell initialization can discard earlier PTY input.
      await expect.poll(() => page.evaluate((id) =>
        window.__gwtTerminalTestApi.bufferText(id).trim(), id), { timeout: 30_000 })
        .toMatch(/[^\n]+[>$#%]$/);
      const terminal = shell.locator(".terminal-root");
      await terminal.click();
      await expect(terminal.locator(".xterm-helper-textarea")).toBeFocused();
      const command = "printf 'GWT_%s\\n' 'SCOPED_4536'\r";
      await page.keyboard.type(command);
      await expect.poll(() => inputFrames.filter((frame) => frame.id === id)
        .map((frame) => frame.data).join("")).toBe(command);
      expect(inputFrames.filter((frame) => frame.id === id)
        .every((frame) => frame.scope === projectKey)).toBe(true);
      await expect.poll(() => terminalOutputs.get(id) ?? "", { timeout: 15_000 }).toContain("GWT_SCOPED_4536");
    } finally {
      await sendLiveGwtEvent(page, { kind: "close_window", id });
    }
    expect(errors).toEqual([]);
    });
  });

  test("live projects stay independent while two clients mirror the same project", async ({ page, context }, testInfo) => {
    const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
    test.skip(!base, "requires browser-check isolated checkout server with two projects");
    await withLiveGwtBackendLock(base!, testInfo, async () => {
      const other = await context.newPage();
      const mirror = await context.newPage();
      const pages = [page, other, mirror];
      const errors = pages.map(collectBrowserErrors);
      const frames = pages.map(() => [] as Array<Record<string, any>>);
      const created: Array<{ page: Page; id: string }> = [];
      const theme = testInfo.project.name.includes("light") ? "light" : "dark";
      const catalogProjectPath = await mkdtemp(join(tmpdir(), "gwt-4537-catalog-"));
      let catalogProjectId: string | undefined;
      try {
        for (const [index, current] of pages.entries()) {
          current.on("websocket", (socket) => {
            socket.on("framereceived", ({ payload }) => {
              const event = JSON.parse(String(payload));
              if (event.kind === "hub_state" || event.kind === "workspace_state") {
                frames[index].push({ ...event, socketProjectKey: new URL(socket.url()).searchParams.get("repo_hash") });
              }
            });
          });
          await gotoLiveGwt(current, base!, { enableTestBridge: true });
          await expect.poll(() => frames[index].some((event) => event.kind === "hub_state")).toBe(true);
          await expect(current.locator('.project-tab[aria-current="page"]')).toHaveCount(0);
          await current.locator(`#op-theme-toggle [data-theme-value="${theme}"]`).click();
          await expect(current.locator("html")).toHaveAttribute("data-theme", theme);
        }
        const catalog = frames[0].find((event) => event.kind === "hub_state")!.hub.projects;
        expect(catalog.length, "isolated session must seed projects A and B").toBeGreaterThanOrEqual(2);
        expect(catalog.every((entry) => !("workspace" in entry))).toBe(true);
        const selected = [catalog[0], catalog[1], catalog[0]];
        for (const [index, current] of pages.entries()) {
          await current.locator(`[data-project-tab-id="${selected[index].id}"]`).click();
          await expect.poll(() => frames[index].some((event) =>
            event.kind === "workspace_state" && event.socketProjectKey === selected[index].project_key
          )).toBe(true);
          await expect(current.locator(`[data-project-tab-id="${selected[index].id}"]`))
            .toHaveAttribute("aria-current", "page");
        }

        // Hub navigation stays available while every browser is project-bound.
        await sendLiveGwtEvent(page, { kind: "reopen_recent_project", path: catalogProjectPath });
        await expect.poll(() => frames[0].filter((event) => event.kind === "hub_state")
          .at(-1)?.hub.projects.length).toBe(catalog.length + 1);
        catalogProjectId = frames[0].filter((event) => event.kind === "hub_state")
          .at(-1)!.hub.projects.find((entry) => !catalog.some((old) => old.id === entry.id)).id;
        for (const [index, current] of pages.entries()) {
          await expect(current.locator(`[data-project-tab-id="${catalogProjectId}"]`)).toBeVisible();
          await expect(current.locator(`[data-project-tab-id="${selected[index].id}"]`))
            .toHaveAttribute("aria-current", "page");
        }
        await sendLiveGwtEvent(page, { kind: "close_project_tab", tab_id: catalogProjectId });
        for (const current of pages) {
          await expect(current.locator(`[data-project-tab-id="${catalogProjectId}"]`)).toHaveCount(0);
        }
        catalogProjectId = undefined;

        const aWindow = await createLiveProjectShell(page);
        created.push({ page, id: aWindow });
        await expect(mirror.locator(`.workspace-window[data-id="${aWindow}"]`)).toBeVisible();
        await expect(other.locator(`.workspace-window[data-id="${aWindow}"]`)).toHaveCount(0);
        const bWindow = await createLiveProjectShell(other);
        created.push({ page: other, id: bWindow });
        await expect(page.locator(`.workspace-window[data-id="${bWindow}"]`)).toHaveCount(0);
        await expect(mirror.locator(`.workspace-window[data-id="${bWindow}"]`)).toHaveCount(0);
        await expect(page.locator(`[data-project-tab-id="${catalog[0].id}"]`)).toHaveAttribute("aria-current", "page");
        await expect(other.locator(`[data-project-tab-id="${catalog[1].id}"]`)).toHaveAttribute("aria-current", "page");
        await page.screenshot({ path: testInfo.outputPath(`project-isolation-${theme}-a.png`) });
        await other.screenshot({ path: testInfo.outputPath(`project-isolation-${theme}-b.png`) });

        // Closing through one A client updates its mirror without touching B.
        await sendLiveGwtEvent(page, { kind: "close_window", id: aWindow });
        await expect(mirror.locator(`.workspace-window[data-id="${aWindow}"]`)).toHaveCount(0);
        await expect(other.locator(`.workspace-window[data-id="${bWindow}"]`)).toBeVisible();
        for (const [index, received] of frames.entries()) {
          const snapshots = received.filter((event) => event.kind === "workspace_state");
          expect(snapshots.length).toBeGreaterThan(0);
          expect(snapshots.every((event) => event.socketProjectKey === selected[index].project_key
            && event.workspace.tabs[0].project_key === selected[index].project_key
            && event.workspace.tabs.length === 1
            && event.workspace.tabs[0].id === selected[index].id)).toBe(true);
        }
      } finally {
        for (const window of created) {
          await sendLiveGwtEvent(window.page, { kind: "close_window", id: window.id }).catch(() => {});
        }
        if (catalogProjectId) {
          await sendLiveGwtEvent(page, { kind: "close_project_tab", tab_id: catalogProjectId }).catch(() => {});
        }
        await other.close();
        await mirror.close();
        await rm(catalogProjectPath, { recursive: true, force: true });
      }
      expect(errors.flat()).toEqual([]);
    });
  });

  test("project switching reconnects with the selected immutable scope", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    await installEmbeddedRoutes(page);
    const tabs = projectTabsFixture(2).map((tab, index) => ({
      ...tab,
      project_key: index === 0 ? "0123456789abcdef" : "fedcba9876543210",
    }));
    await installProjectTabsBackend(page, tabs);
    await page.goto(APP_URL);
    const urls = () => page.evaluate(() => window.__gwtProjectTabsSocketUrls);
    await expect.poll(urls).toEqual(["ws://gwt-playwright.local/ws"]);
    await page.locator(".project-tab").first().click();
    await expect.poll(urls).toEqual([
      "ws://gwt-playwright.local/ws",
      "ws://gwt-playwright.local/ws?repo_hash=0123456789abcdef",
    ]);
    await page.locator(".project-tab").nth(1).click();
    await expect(page.locator(".project-tab").nth(1)).toHaveAttribute("aria-current", "page");
    await expect.poll(urls).toEqual([
      "ws://gwt-playwright.local/ws",
      "ws://gwt-playwright.local/ws?repo_hash=0123456789abcdef",
      "ws://gwt-playwright.local/ws?repo_hash=fedcba9876543210",
    ]);
    await page.locator(".project-tab").nth(0).click();
    await expect.poll(urls).toHaveLength(4);
    expect((await urls()).at(-1)).toContain("repo_hash=0123456789abcdef");
    expect(errors).toEqual([]);
  });

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
    await first.click();
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
    await first.click();
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

    await page.locator('[data-project-tab-id="tab-running"]').click();
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

async function installProjectTabsBackend(page, tabFixture: number | unknown[]) {
  await page.addInitScript((fixture) => {
    const tabs = (Array.isArray(fixture)
      ? fixture
      : Array.from({ length: fixture }, (_, index) => {
          const number = String(index + 1).padStart(2, "0");
          return {
            id: `tab-${number}`,
            title: `known-project-${number}`,
            project_root: `/fixture/known-project-${number}`,
            kind: "git",
            workspace: { viewport: { x: 0, y: 0, zoom: 1 }, windows: [] },
          };
        })).map((tab, index) => ({
          ...tab,
          project_key: tab.project_key ?? (index + 1).toString(16).padStart(16, "0"),
        }));
    const hubState = {
      kind: "hub_state",
      hub: {
        app_version: "playwright",
        projects: tabs.map(({ id, project_key, title, kind }) => ({ id, project_key, title, kind })),
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
        this.projectKey = new URL(url).searchParams.get("repo_hash");
        (window.__gwtProjectTabsSocketUrls ??= []).push(url);
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
          const tab = tabs.find((entry) => entry.project_key === this.projectKey);
          this.emit(tab ? {
            kind: "workspace_state",
            workspace: { app_version: "playwright", tabs: [tab], active_tab_id: tab.id, recent_projects: [] },
          } : hubState);
          return;
        }
        if (message.kind === "save_ui_trace") {
          this.savedUiTracePayload = message;
          return;
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
          new MessageEvent("message", { data: JSON.stringify(this.projectKey
            ? { ...payload, project_key: this.projectKey } : payload) }),
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

function collectBrowserErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  return errors;
}

async function createLiveProjectShell(page: Page): Promise<string> {
  const shells = page.locator('.workspace-window[data-preset="shell"]');
  const before = new Set(await shells.evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset.id)));
  await sendLiveGwtEvent(page, {
    kind: "create_window", preset: "shell",
    bounds: { x: 80, y: 80, width: 720, height: 420 },
  });
  const created = async () => (await shells.evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset.id!)))
    .filter((id) => !before.has(id));
  await expect.poll(created, { timeout: 30_000 }).toHaveLength(1);
  return (await created())[0];
}
