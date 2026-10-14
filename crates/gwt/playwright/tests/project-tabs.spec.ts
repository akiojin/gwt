import { expect, test, type Page } from "@playwright/test";
import { randomUUID } from "node:crypto";
import { readFile, rename, writeFile } from "node:fs/promises";
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

  test("live catalog keeps Window List and Fleet Minimap on the same revision", async ({ page }, testInfo) => {
    const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
    test.skip(!base, "requires browser-check isolated checkout server");
    await withLiveGwtBackendLock(base!, testInfo, async () => {
      let workspace: any;
      page.on("websocket", socket => socket.on("framereceived", ({ payload }) => {
        const event = JSON.parse(String(payload));
        if (event.kind === "workspace_state") workspace = event.workspace;
      }));
      await gotoLiveGwt(page, base!, { enableTestBridge: true });
      await expect.poll(() => workspace?.active_tab_id).toBeTruthy();
      const activeWindows = () => workspace.tabs.find(tab => tab.id === workspace.active_tab_id).workspace.windows;
      const previousIds = new Set(activeWindows().map(window => window.id));
      const createdIds = () => activeWindows().filter(window => !previousIds.has(window.id)).map(window => window.id) as string[];
      let ownedIds: string[] = [];
      try {
        for (const x of [120, 760]) {
          await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell",
            bounds: { x, y: 100, width: 520, height: 300 } });
          await expect.poll(createdIds).toHaveLength(ownedIds.length + 1);
          ownedIds = createdIds();
        }
        const [firstId, secondId] = ownedIds;
        const first = page.locator(`.workspace-window[data-id="${firstId}"]`);
        const cell = page.locator(`.fleet-minimap__cell[data-window-id="${firstId}"]`);
        await expect(first).toBeVisible();
        await expect(cell).toHaveCount(1);
        await expect(page.locator(`.fleet-minimap__cell[data-window-id="${secondId}"]`)).toHaveCount(1);
        await page.locator("#window-list-button").click();
        await expect(page.locator("#window-list-panel")).toBeVisible();

        // Read after the app's pointer handlers in the same browser turn, before
        // a backend echo can acknowledge local focus or the geometry commit.
        const before = await page.evaluate(id => {
          const mapCell = document.querySelector<HTMLElement>(`.fleet-minimap__cell[data-window-id="${id}"]`)!;
          const versions = () => ({
            list: document.getElementById("window-list-panel")!.dataset.catalogVersion,
            minimap: document.getElementById("fleet-minimap")!.dataset.catalogVersion,
          });
          (window as any).__catalogGesture = {};
          window.addEventListener("pointerdown", () => {
            (window as any).__catalogGesture.down = {
              ...versions(), focused: mapCell.classList.contains("is-focused"),
            };
          }, { once: true });
          window.addEventListener("pointerup", () => {
            (window as any).__catalogGesture.up = {
              ...versions(), left: parseFloat(mapCell.style.left), top: parseFloat(mapCell.style.top),
            };
          }, { once: true });
          return { ...versions(), focused: mapCell.classList.contains("is-focused"),
            left: parseFloat(mapCell.style.left), top: parseFloat(mapCell.style.top) };
        }, firstId);
        const titlebar = await first.locator(".titlebar").boundingBox();
        expect(titlebar).not.toBeNull();
        const startX = titlebar!.x + 40;
        const startY = titlebar!.y + titlebar!.height / 2;
        await page.mouse.move(startX, startY);
        await page.mouse.down();
        await page.mouse.move(startX + 80, startY + 60, { steps: 2 });
        await page.mouse.up();
        const gesture = await page.evaluate(() => (window as any).__catalogGesture);
        expect(before.list).toBeTruthy();
        expect(before.list).toBe(before.minimap);
        expect(before.focused).toBe(false);
        expect(gesture.down.focused).toBe(true);
        expect(gesture.down.list).toBe(gesture.down.minimap);
        expect(gesture.up.list).toBe(gesture.up.minimap);
        expect(Number(gesture.up.list)).toBeGreaterThan(Number(gesture.down.list));
        expect(gesture.up.left).toBeGreaterThan(before.left);
        expect(gesture.up.top).toBeGreaterThan(before.top);
        await page.locator("#window-list-button").click();
        await expect(page.locator("#window-list-panel")).toBeVisible();

        // Use the existing wire-event bridge, never a model/renderer test API.
        // Capture each result synchronously so live shell updates cannot race
        // the workspace, runtime, and List-only snapshot assertions.
        const result = await page.evaluate(({ workspace, firstId, secondId }) => {
          const inject = (detail: unknown) => window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail }));
          const list = document.getElementById("window-list-panel")!;
          const minimap = document.getElementById("fleet-minimap")!;
          const versions = () => ({ list: list.dataset.catalogVersion, minimap: minimap.dataset.catalogVersion });
          const current = structuredClone(workspace);
          const windows = current.tabs.find(tab => tab.id === current.active_tab_id).workspace.windows;
          for (const [id, title] of [[firstId, "Catalog primary 5016"], [secondId, "Catalog secondary 5016"]]) {
            Object.assign(windows.find(entry => entry.id === id), {
              title, dynamic_title: null, purpose_title: null, dynamic_title_detail: "Shared catalog update",
            });
          }
          inject({ kind: "workspace_state", workspace: current });
          const row = () => [...list.querySelectorAll<HTMLElement>(".window-list-row")]
            .find(entry => entry.querySelector(".window-list-title")?.textContent === "Catalog primary 5016")!;
          const cell = minimap.querySelector<HTMLElement>(`.fleet-minimap__cell[data-window-id="${firstId}"]`)!;
          const catalog = { ...versions(), title: row().querySelector(".window-list-title")!.textContent,
            tooltip: cell.title, cells: minimap.querySelectorAll(".fleet-minimap__cell").length };
          const cameraState = structuredClone(current);
          const cameraWorkspace = cameraState.tabs.find(tab => tab.id === cameraState.active_tab_id).workspace;
          cameraWorkspace.viewport = { ...cameraWorkspace.viewport, x: cameraWorkspace.viewport.x + 25 };
          inject({ kind: "workspace_state", workspace: cameraState });
          const camera = { ...versions(), cellRetained:
            minimap.querySelector(`.fleet-minimap__cell[data-window-id="${firstId}"]`) === cell };
          inject({ kind: "terminal_status", id: firstId, status: "error", detail: "Catalog runtime" });
          const runtime = { ...versions(), status: row().querySelector(".status-label")!.textContent,
            telemetry: cell.dataset.telemetry };
          inject({ kind: "window_list", windows: [{ id: firstId }] });
          const gate = { ...versions(), rows: list.querySelectorAll(".window-list-row").length,
            cells: minimap.querySelectorAll(".fleet-minimap__cell").length,
            secondRetained: Boolean(minimap.querySelector(`.fleet-minimap__cell[data-window-id="${secondId}"]`)) };
          return { catalog, camera, runtime, gate };
        }, { workspace, firstId, secondId });
        expect(result.catalog.list).toBe(result.catalog.minimap);
        expect(result.catalog.title).toBe("Catalog primary 5016");
        expect(result.catalog.tooltip).toBe("Catalog primary 5016 · Shared catalog update");
        expect(result.camera.list).toBe(result.catalog.list);
        expect(result.camera.minimap).toBe(result.catalog.minimap);
        expect(result.camera.cellRetained).toBe(true);
        expect(result.runtime.list).toBe(result.runtime.minimap);
        expect(Number(result.runtime.list)).toBeGreaterThan(Number(result.catalog.list));
        expect(result.runtime.status).toBe("Error");
        expect(result.runtime.telemetry).toBe("error");
        expect(result.gate.list).toBe(result.runtime.list);
        expect(result.gate.list).toBe(result.gate.minimap);
        expect(result.gate.rows).toBe(1);
        expect(result.gate.cells).toBe(result.catalog.cells);
        expect(result.gate.secondRetained).toBe(true);
        await page.screenshot({ path: testInfo.outputPath("shared-catalog.png"), fullPage: true });
      } finally {
        for (const id of ownedIds) {
          await sendLiveGwtEvent(page, { kind: "close_window", id });
          await expect(page.locator(`.workspace-window[data-id="${id}"]`)).toHaveCount(0);
        }
      }
    });
  });

  test("live watcher applies a burst of projection updates to the pane title", async ({ page }, testInfo) => {
    const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
    const projectionPath = process.env.GWT_PLAYWRIGHT_PROJECTION_PATH;
    const projectRoot = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT;
    test.skip(!base || !projectionPath || !projectRoot, "requires browser-check isolated watcher fixture");
    await withLiveGwtBackendLock(base!, testInfo, async () => {
      const errors = collectBrowserErrors(page);
      const patches: any[] = [];
      let windowId = "";
      page.on("websocket", (socket) => socket.on("framereceived", ({ payload }) => {
        const event = JSON.parse(String(payload));
        if (event.kind === "active_work_projection_patch") patches.push(event.projection);
        if (event.kind === "workspace_state") {
          for (const tab of event.workspace.tabs) {
            const window = (tab.workspace?.windows ?? []).find((entry) => entry.preset === "shell");
            if (window) windowId = window.id;
          }
        }
      }));
      await gotoLiveGwt(page, base!, { enableTestBridge: true });
      await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell", bounds: { x: 80, y: 80, width: 720, height: 420 } });
      await expect.poll(() => windowId).not.toBe("");
      const saved = await readFile(projectionPath!, "utf8");
      const projection = JSON.parse(saved);
      const sessionId = randomUUID();
      const now = new Date().toISOString();
      projection.agents = [{ session_id: sessionId, window_id: windowId, agent_id: "codex", display_name: "Codex",
        status_category: "active", current_focus: "Watcher preparation", title_summary: "Watcher purpose 0",
        worktree_path: projectRoot, branch: "work/watcher", last_board_entry_id: null,
        affiliation_status: "assigned", updated_at: now }];
      projection.status_category = "active";
      projection.updated_at = now;
      const writeProjection = async (contents: string) => {
        const staging = `${projectionPath}.watcher-test`;
        await writeFile(staging, contents);
        // Windows readers can briefly deny replacement. Retry only that file
        // operation; application assertions and the whole test run stay strict.
        await expect.poll(async () => {
          try {
            await rename(staging, projectionPath!);
            return null;
          } catch (error) {
            const code = (error as NodeJS.ErrnoException).code;
            if (process.platform !== "win32" || (code !== "EPERM" && code !== "EBUSY")) throw error;
            return code;
          }
        }, { timeout: 5000, intervals: [100, 200, 500] }).toBeNull();
      };
      try {
        for (let index = 1; index <= 10; index++) {
          projection.agents[0].title_summary = `Watcher purpose ${index}`;
          await writeProjection(JSON.stringify(projection));
        }
        await expect(page.locator(`.workspace-window[data-id="${windowId}"] .title-text`)).toContainText("Watcher purpose 10");
        await expect.poll(() => patches.some((patch) => patch.agents.some((agent) =>
          agent.session_id === sessionId && agent.title_summary === "Watcher purpose 10"))).toBe(true);
        await page.screenshot({ path: testInfo.outputPath("watcher.png"), fullPage: true });
        expect(errors).toEqual([]);
      } finally {
        await writeProjection(saved);
        await sendLiveGwtEvent(page, { kind: "close_window", id: windowId });
      }
    });
  });

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
    // Issue #4538: the live page is bound to its `/p/<key>` route.
    await gotoLiveGwt(page, base!, { enableTestBridge: true });
    await expect(page.locator("#close-project-button")).toBeVisible();
    await expect.poll(() => scopedWorkspaceReceived).toBe(true);
    expect(projectKey).toMatch(/^[0-9a-f]{16}$/);
    expect(new URL(page.url()).pathname).toBe(`/p/${projectKey}`);
    const scoped = sockets.filter((url) => new URL(url).searchParams.has("repo_hash"));
    expect(scoped.length).toBeGreaterThan(0);
    expect(scoped.every((url) => new URL(url).searchParams.get("repo_hash") === projectKey)).toBe(true);
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
      const command = "node -p \"'GWT_' + 'SCOPED_4536'\"\r";
      await page.keyboard.type(command);
      // Windows shells can elicit cursor-position and focus replies before
      // typing; those protocol frames share the terminal_input transport.
      await expect.poll(() => inputFrames.filter((frame) => frame.id === id)
        .map((frame) => frame.data).join("")).toContain(command);
      expect(inputFrames.filter((frame) => frame.id === id)
        .every((frame) => frame.scope === projectKey)).toBe(true);
      await expect.poll(() => terminalOutputs.get(id) ?? "", { timeout: 15_000 }).toContain("GWT_SCOPED_4536");
    } finally {
      await sendLiveGwtEvent(page, { kind: "close_window", id });
    }
    expect(errors).toEqual([]);
    });
  });

});

function collectBrowserErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  return errors;
}
