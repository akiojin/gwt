import { expect, test, type Page } from "@playwright/test";
import { APP_URL, APP_PROJECT_KEY, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { liveGwtProjectUrl } from "./_helpers/live-gwt";

const liveBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
const appUrl = liveBase
  ? new URL(liveBase).pathname === "/" ? liveGwtProjectUrl(liveBase, APP_PROJECT_KEY) : liveBase
  : APP_URL;
const errors = new WeakMap<Page, string[]>();

test.describe("Agent title role and worktree badges", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test.beforeEach(async ({ page }, info) => {
    const captured: string[] = [];
    errors.set(page, captured);
    page.on("pageerror", error => captured.push(error.message));
    page.on("console", message => { if (message.type() === "error") captured.push(message.text()); });
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme),
      info.project.name.includes("light") ? "light" : "dark");
  });
  test.afterEach(async ({ page }, info) => {
    await expect(page.locator("html")).toHaveAttribute("data-theme",
      info.project.name.includes("light") ? "light" : "dark");
    expect(errors.get(page), "zero console/page errors").toEqual([]);
  });

  test("Issue #5478 review title stays distinct from its waiting owner", async ({ page }, info) => {
    if (!liveBase) await installEmbeddedRoutes(page);
    await installAgentTitleBadgeBackend(page, undefined, false, true);
    await page.goto(appUrl);
    await page.locator('.op-rail__surface[data-surface="works"]').click();
    const owner = page.locator('.work-row[data-window-id="agent-1"]');
    const review = page.locator('.work-row[data-window-id="agent-2"]');
    await expect(owner.locator(".work-purpose")).toHaveText("Deliver Issue #42");
    await expect(review.locator(".work-purpose")).toHaveText("Review · Issue #42");
    await review.locator(".work-open").click();
    await expect(page.locator(".works-full-meta")).toContainText("Review · Issue #42");
    // The legacy titlebar retains the shared semantic adapter while Works owns navigation.
    await expect(page.locator('.workspace-window[data-id="agent-2"] .window-worktree-badge'))
      .toHaveAttribute("data-worktree-form", "ephemeral");
    await page.screenshot({ path: info.outputPath("independent-review.png") });
  });

  test("Board authors and replies retain assigned names in both themes", async ({ page }, info) => {
    if (!liveBase) await installEmbeddedRoutes(page);
    await installAgentTitleBadgeBackend(page, undefined, true);
    await page.goto(appUrl);
    await page.locator('.op-rail__surface[data-surface="board"]').click();
    const board = page.locator(".workspace-window[data-id='board-1']");
    await expect(board).toBeVisible();
    const authored = board.locator("[data-board-entry-id='named-post']");
    await expect(authored.locator(".board-message-meta")).toContainText("ミオ (Codex)");
    await expect(board.locator(".board-reply-quote")).toContainText("Reply to ミオ (Codex)");
    await authored.locator(".board-reply-button").click();
    await expect(board.locator(".board-reply-banner-text")).toContainText("Replying to ミオ (Codex)");
    await authored.locator(".board-message-meta").scrollIntoViewIfNeeded();
    const screenshot = info.outputPath("board-agent-names.png");
    await board.screenshot({ path: screenshot });
    await info.attach("Board agent names", { path: screenshot, contentType: "image/png" });
  });

  test("late names reach Kanban and live purpose updates reach Works in both themes", async ({ page }, info) => {
    if (!liveBase) await installEmbeddedRoutes(page);
    await installAgentTitleBadgeBackend(page, undefined, true);
    await page.goto(appUrl);
    const assignedName = info.project.name.includes("light") ? "Maya Clark" : "ミオ・アサノ";
    await page.evaluate(name => (window as any).__assignAgentName(name), assignedName);
    // The persisted inline placement restores its owning Agent Kanban through
    // the normal workspace catalog route; its renderer survives the retired list.
    await expect(page.locator("#fleet-minimap")).toHaveCount(0);
    const kanban = page.locator(".workspace-window[data-id='kanban-1']");
    await expect(kanban).toBeVisible();
    await expect(kanban.locator(".agent-kanban-card[data-window-id='agent-3'] .agent-kanban-card-title"))
      .toHaveText("Lyra");
    await expect(kanban.locator(".agent-kanban-card[data-window-id='agent-3'] .agent-kanban-card-meta"))
      .toContainText("Review cache");
    const screenshot = info.outputPath("kanban-agent-names.png");
    await kanban.screenshot({ path: screenshot });
    await info.attach("Kanban agent names", { path: screenshot, contentType: "image/png" });
    await page.locator('.op-rail__surface[data-surface="works"]').click();
    await expect(page.locator(".work-row[data-window-id='agent-1'] .work-name")).toHaveText(assignedName);
    await expect(page.locator(".work-row[data-window-id='agent-1'] .work-purpose")).toHaveText("Fix cache");
    await expect(page.locator(".work-row[data-window-id='agent-2'] .work-purpose")).toHaveText("Intake Agent");
    const frameResults = [];
    for (const suspended of [false, true]) {
      const result = await page.evaluate(async ({ suspended, assignedName }) => {
        const title = document.querySelector<HTMLElement>(".works-surface:not(.works-surface--preview) .work-row[data-window-id='agent-1'] .work-purpose")!;
        const before = title.textContent;
        const label = suspended ? "Fallback" : "Frame";
        const expected = `${label} 49`;
        const originalFrame = window.requestAnimationFrame;
        let renders = 0;
        let completed: () => void;
        const changed = new Promise<void>(resolve => { completed = resolve; });
        const observer = new MutationObserver(records => {
          renders += records.length;
          if (title.textContent === expected) completed();
        });
        observer.observe(title, { childList: true });
        if (suspended) window.requestAnimationFrame = () => 0;
        const started = performance.now();
        let deadline: ReturnType<typeof setTimeout> | undefined;
        try {
          for (let index = 0; index < 50; index++) (window as any).__injectAgentTitle(`${label} ${index}`);
          const beforeFlush = title.textContent;
          await Promise.race([changed, new Promise<never>((_, reject) => {
            deadline = setTimeout(() => reject(new Error("Works frame did not render within one second")), 1000);
          })]);
          return { before, beforeFlush, renders, elapsed: performance.now() - started,
            title: `${title.closest(".work-row")!.querySelector(".work-name")!.textContent} — ${title.textContent}`, expected: `${assignedName} — ${expected}` };
        } finally {
          clearTimeout(deadline);
          observer.disconnect();
          window.requestAnimationFrame = originalFrame;
        }
      }, { suspended, assignedName });
      expect(result.beforeFlush, "receipts leave the displayed title intact until the frame").toBe(result.before);
      expect(result.renders, "the burst renders the latest title once").toBe(1);
      expect(result.title).toBe(result.expected);
      expect(result.elapsed).toBeLessThan(1000);
      frameResults.push({ suspended, ...result });
    }
    await info.attach("Works frame results", { body: JSON.stringify(frameResults, null, 2), contentType: "application/json" });
    const agentTitle = page.locator('.work-row[data-window-id="agent-1"]');
    await agentTitle.focus();
    await agentTitle.hover();
    await expect(agentTitle).toBeFocused();
    await expect(agentTitle).toHaveAttribute("aria-label", new RegExp(`${assignedName}.*Fallback 49.*Open terminal`));
    await page.screenshot({ path: info.outputPath("agent-names.png") });
  });

});

async function installAgentTitleBadgeBackend(
  page,
  primaryAgent = { agentId: "codex", color: "cyan", title: "Codex" },
  identityScenario = false,
  reviewScenario = false,
) {
  await page.addInitScript(({ primaryAgentFixture, identityScenario, reviewScenario }) => {
    (window as any).__gwtPlaywrightTestBridge = true;
    const windows = [
      {
        id: "agent-1",
        session_id: "agent-1-session",
        title: primaryAgentFixture.title,
        preset: "agent",
        geometry: { x: 20, y: 640, width: 520, height: 280 },
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
        agent_id: primaryAgentFixture.agentId,
        agent_color: primaryAgentFixture.color,
        lane_kind: "execution",
        tab_group_id: null,
        tab_group_active: false,
      },
      {
        id: "agent-2",
        session_id: "agent-2-session",
        title: "Intake Agent",
        preset: "agent",
        geometry: { x: 30, y: 680, width: 500, height: 280 },
        geometry_revision: 0,
        z_index: 2,
        status: "idle",
        minimized: false,
        maximized: false,
        pre_maximize_geometry: null,
        persist: true,
        purpose_title: null,
        dynamic_title: null,
        dynamic_title_detail: null,
        agent_id: "claude",
        agent_color: "cyan",
        lane_kind: "intake",
        tab_group_id: null,
        tab_group_active: false,
      },
      {
        id: "agent-3",
        session_id: "agent-3-session",
        title: "Restored Agent",
        preset: "agent",
        geometry: { x: 40, y: 720, width: 480, height: 280 },
        geometry_revision: 0,
        z_index: 3,
        status: "idle",
        minimized: false,
        maximized: false,
        pre_maximize_geometry: null,
        persist: true,
        purpose_title: null,
        dynamic_title: null,
        dynamic_title_detail: null,
        agent_id: "claude",
        agent_color: "cyan",
        lane_kind: "unknown",
        tab_group_id: null,
        tab_group_active: false,
      },
    ];
    if (reviewScenario) {
      windows[0].purpose_title = "Deliver Issue #42";
      windows[1].title = "Review · Issue #42";
      windows[1].purpose_title = "Review · Issue #42";
    }
    if (identityScenario) {
      Object.assign(windows[0], { dynamic_title: "Fix cache" });
      Object.assign(windows[2], { agent_name: "Lyra", dynamic_title: "Review cache",
        placement: { kind: "agent_kanban", board_id: "kanban-1", lane_id: "active", order: 0, collapsed: true } });
      windows.push({ ...windows[0], id: "kanban-1", preset: "agent_kanban", title: "Agent Kanban",
        agent_id: null, dynamic_title: null, geometry: { x: 600, y: 640, width: 720, height: 420 } });
      windows.push({ ...windows[0], id: "board-1", preset: "board", title: "Board",
        agent_id: null, agent_name: null, dynamic_title: null,
        geometry: { x: 780, y: 20, width: 620, height: 540 } });
    }
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Agent Badge Fixture",
            project_key: location.pathname.split("/").filter(Boolean).at(-1),
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
    };
    const windowListState = {
      kind: "window_list",
      windows: windows.map((windowData) => ({ ...windowData })),
    };

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
          this.emit({ kind: "active_work_projection", projection: { id: "fixture-local-live", active_works: [], local_agent_activity: { project_key: location.pathname.split("/").filter(Boolean).at(-1), host_id: "title-badge-fixture", observed_at: new Date().toISOString(), agents: [
          { window_id: 'agent-1', session_id: 'agent-1-session', runtime_incarnation: 1, is_pm: false, issue_number: null },
          { window_id: 'agent-2', session_id: 'agent-2-session', runtime_incarnation: 1, is_pm: false, issue_number: null },
          { window_id: 'agent-3', session_id: 'agent-3-session', runtime_incarnation: 1, is_pm: false, issue_number: null }
        ] } } });
        }, 0);
      }

      send(data) {
        let msg;
        try {
          msg = JSON.parse(data);
        } catch {
          return;
        }
        if (msg.kind === "list_windows") {
          this.emit(windowListState);
        }
        if (msg.kind === "load_board") {
          const post = { id: "named-post", author_kind: "agent", author: "Codex",
            origin_agent_name: "ミオ", origin_agent_id: "codex", origin_session_id: "closed-session",
            origin_branch: "work/issue-3824", kind: "status", body: "対応中です。",
            created_at: "2026-10-15T00:00:00Z", updated_at: "2026-10-15T00:00:00Z",
            mentions: [], audience: [] };
          this.emit({ kind: "board_entries", id: msg.id, has_more_before: false,
            entries: [post, { ...post, id: "reply-post", author_kind: "user", author: "You",
              origin_agent_name: null, origin_session_id: null, parent_id: post.id,
              body: "了解です。", created_at: "2026-10-15T00:01:00Z", updated_at: "2026-10-15T00:01:00Z" }] });
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

    (window as any).__assignAgentName = (name: string) => {
      workspaceState.workspace.tabs[0].workspace.windows[0].agent_name = name;
      windowListState.windows[0].agent_name = name;
      for (const socket of FixtureWebSocket.instances) socket.emit(workspaceState);
    };
    // Use the existing synchronous receive bridge to separate receipt from its
    // presentation frame; websocket event batching is tested independently.
    (window as any).__injectAgentTitle = (title: string) => {
      workspaceState.workspace.tabs[0].workspace.windows[0].dynamic_title = title;
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: workspaceState }));
    };
    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  }, { primaryAgentFixture: primaryAgent, identityScenario, reviewScenario });
}
