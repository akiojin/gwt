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
      const writeProjection = async () => {
        const staging = `${projectionPath}.watcher-test`;
        await writeFile(staging, JSON.stringify(projection));
        await rename(staging, projectionPath!);
      };
      try {
        for (let index = 1; index <= 10; index++) {
          projection.agents[0].title_summary = `Watcher purpose ${index}`;
          await writeProjection();
        }
        await expect(page.locator(`.workspace-window[data-id="${windowId}"] .title-text`)).toContainText("Watcher purpose 10");
        await expect.poll(() => patches.some((patch) => patch.agents.some((agent) =>
          agent.session_id === sessionId && agent.title_summary === "Watcher purpose 10"))).toBe(true);
        await page.screenshot({ path: testInfo.outputPath("watcher.png"), fullPage: true });
        expect(errors).toEqual([]);
      } finally {
        await writeFile(`${projectionPath}.watcher-test`, saved);
        await rename(`${projectionPath}.watcher-test`, projectionPath!);
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
