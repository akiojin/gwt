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

  test("fallback runtime title still displays the Agent runtime badge", async ({
    page,
  }, testInfo) => {
    if (!liveBase) await installEmbeddedRoutes(page);
    await installAgentTitleBadgeBackend(page, {
      agentId: "grok",
      color: "gray",
      title: "Grok Build",
    });

    await page.goto(appUrl);
    await page.locator("#window-list-button").click();

    const agentWindow = page.locator(".window-list-row").first();
    await expect(agentWindow).toBeVisible({ timeout: 10_000 });
    await expect(agentWindow.locator(".window-list-title")).toHaveText("Grok Build");

    const badge = agentWindow.locator(".window-role-badge").first();
    await expect(badge).toBeVisible();
    await expect(badge).toHaveText("Grok Build");

    // list_windows may replace rows while screenshot prepares the capture.
    await page.locator("#window-list-panel").screenshot({
      path:
        process.env.GWT_AGENT_BADGE_SCREENSHOT_PATH ||
        testInfo.outputPath("agent-title-badge.png"),
    });
  });

  test("Issue #5478 review title stays distinct from its waiting owner", async ({ page }, info) => {
    if (!liveBase) await installEmbeddedRoutes(page);
    await installAgentTitleBadgeBackend(page, undefined, false, true);
    await page.goto(appUrl);
    await page.locator("#window-list-button").click();
    const owner = page.locator(".window-list-row").nth(0);
    const review = page.locator(".window-list-row").nth(1);
    await expect(owner.locator(".window-list-title")).toHaveText("Deliver Issue #42");
    await expect(review.locator(".window-list-title")).toHaveText("Review · Issue #42");
    await expect(review.locator(".window-worktree-badge")).toHaveAttribute("data-worktree-form", "ephemeral");
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

  test("late names reach badge, title and Kanban in both themes", async ({ page }, info) => {
    if (!liveBase) await installEmbeddedRoutes(page);
    await installAgentTitleBadgeBackend(page, undefined, true);
    await page.goto(appUrl);
    await page.locator("#window-list-button").click();
    const agent = page.locator(".window-list-row").first();
    await expect(agent.locator(".window-worktree-badge")).toHaveText("Fix cache");
    const assignedName = info.project.name.includes("light") ? "Maya Clark" : "ミオ・アサノ";
    await page.evaluate(name => (window as any).__assignAgentName(name), assignedName);
    await expect(agent.locator(".window-worktree-badge")).toHaveText(assignedName);
    await expect(agent.locator(".window-worktree-badge")).toHaveAccessibleName(`${assignedName} — Branch-backed worktree`);
    await expect(agent.locator(".window-list-title")).toHaveText(`${assignedName} — Fix cache`);
    await expect(agent.locator(".window-role-badge")).toHaveText("Codex");
    await expect(page.locator("#fleet-minimap")).toHaveCount(0);
    await expect(page.locator(".agent-kanban-card[data-window-id='agent-3'] .agent-kanban-card-title"))
      .toHaveText("Lyra");
    await expect(page.locator(".agent-kanban-card[data-window-id='agent-3'] .agent-kanban-card-meta"))
      .toContainText("Review cache");
    await expect(page.locator(".window-list-title", { hasText: `${assignedName} — Fix cache` })).toBeVisible();
    await page.locator(".window-list-row")
      .filter({ has: page.locator(".window-list-title", { hasText: "Agent Kanban" }) }).click();
    const kanban = page.locator(".workspace-window[data-id='kanban-1']");
    await expect(kanban.locator(".agent-kanban-card-title")).toHaveText("Lyra");
    const screenshot = info.outputPath("kanban-agent-names.png");
    await kanban.screenshot({ path: screenshot });
    await info.attach("Kanban agent names", { path: screenshot, contentType: "image/png" });
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    await expect(page.locator(".agent-tile[data-agent-id='agent-1'] h3")).toHaveText(`${assignedName} — Fix cache`);
    await expect(page.locator(".agent-tile[data-agent-id='agent-2'] h3")).toHaveText("Intake Agent");
    const frameResults = [];
    for (const suspended of [false, true]) {
      const result = await page.evaluate(async ({ suspended, assignedName }) => {
        const title = document.querySelector<HTMLElement>(".agents-surface:not(.agents-surface--preview) .agent-tile[data-agent-id='agent-1'] .agent-title__purpose")!;
        const before = title.textContent;
        const label = suspended ? "Fallback" : "Frame";
        const expected = ` — ${label} 49`;
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
            deadline = setTimeout(() => reject(new Error("Agents frame did not render within one second")), 1000);
          })]);
          return { before, beforeFlush, renders, elapsed: performance.now() - started,
            title: title.closest("h3")!.textContent, expected: `${assignedName}${expected}` };
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
    await info.attach("Agents frame results", { body: JSON.stringify(frameResults, null, 2), contentType: "application/json" });
    const agentTitle = page.getByRole("tab", { name: `${assignedName} — Fallback 49`, exact: true });
    await agentTitle.focus();
    await agentTitle.hover();
    await expect(agentTitle).toBeFocused();
    await expect(agentTitle).toHaveAttribute("aria-label", `${assignedName} — Fallback 49`);
    await page.screenshot({ path: info.outputPath("agent-names.png") });
  });

  test("legacy wire forms render semantic badges across agent chrome", async ({
    page,
  }) => {
    if (!liveBase) await installEmbeddedRoutes(page);
    await installAgentTitleBadgeBackend(page);

    await page.goto(appUrl);
    await page.locator("#window-list-button").click();

    const branchBackedWindow = page.locator(".window-list-row").nth(0);
    const ephemeralWindow = page.locator(".window-list-row").nth(1);
    const unknownWindow = page.locator(".window-list-row").nth(2);
    await expect(branchBackedWindow).toBeVisible({ timeout: 10_000 });
    await expect(ephemeralWindow).toBeVisible();
    await expect(unknownWindow).toBeVisible();

    const branchBackedBadge = branchBackedWindow.locator(
      ".window-worktree-badge",
    );
    await expect(branchBackedBadge).toBeVisible();
    await expect(branchBackedBadge).toHaveText("Codex");
    await expect(branchBackedBadge).toHaveAttribute(
      "data-worktree-form",
      "branch-backed",
    );
    await expect(branchBackedBadge).toHaveAttribute(
      "data-worktree-label",
      "Branch-backed",
    );
    await expect(branchBackedBadge).toHaveAttribute("data-worktree-symbol", "B");
    await expect(branchBackedBadge).toHaveAttribute(
      "aria-label",
      "Codex — Branch-backed worktree",
    );
    await expect(branchBackedBadge).toHaveAttribute("role", "img");
    await expect(branchBackedBadge).toHaveAccessibleName(
      "Codex — Branch-backed worktree",
    );
    await expect(branchBackedBadge).toHaveAttribute(
      "title",
      "Codex — Branch-backed worktree",
    );

    const ephemeralBadge = ephemeralWindow.locator(".window-worktree-badge");
    await expect(ephemeralBadge).toBeVisible();
    await expect(ephemeralBadge).toHaveText("Intake Agent");
    await expect(ephemeralBadge).toHaveAttribute(
      "data-worktree-form",
      "ephemeral",
    );
    await expect(ephemeralBadge).toHaveAttribute(
      "data-worktree-label",
      "Ephemeral",
    );
    await expect(ephemeralBadge).toHaveAttribute("data-worktree-symbol", "Ø");
    await expect(ephemeralBadge).toHaveAttribute(
      "aria-label",
      "Intake Agent — Ephemeral branchless worktree",
    );
    await expect(ephemeralBadge).toHaveAttribute("role", "img");
    await expect(ephemeralBadge).toHaveAccessibleName(
      "Intake Agent — Ephemeral branchless worktree",
    );

    const unknownBadge = unknownWindow.locator(".window-worktree-badge");
    await expect(unknownBadge).toBeVisible();
    await expect(unknownBadge).toHaveText("Restored Agent");
    await expect(unknownBadge).toHaveAttribute("data-worktree-form", "unknown");
    await expect(unknownBadge).toHaveAttribute(
      "data-worktree-label",
      "Unknown worktree form",
    );
    await expect(unknownBadge).toHaveAttribute("data-worktree-symbol", "?");
    await expect(unknownBadge).toHaveAttribute(
      "aria-label",
      "Restored Agent — Unknown worktree form",
    );
    await expect(unknownBadge).toHaveAttribute("role", "img");
    await expect(unknownBadge).toHaveAccessibleName("Restored Agent — Unknown worktree form");

    const panel = page.locator("#window-list-panel");
    await expect(panel).toBeVisible();
    const branchBackedListBadge = panel
      .locator(".window-list-row", { hasText: "Codex" })
      .first()
      .locator(".window-list-worktree");
    const ephemeralListBadge = panel
      .locator(".window-list-row").nth(1)
      .first()
      .locator(".window-list-worktree");
    const unknownListBadge = panel
      .locator(".window-list-row").nth(2)
      .first()
      .locator(".window-list-worktree");
    await expect(branchBackedListBadge).toHaveText("Codex");
    await expect(branchBackedListBadge).toHaveAttribute(
      "data-worktree-form",
      "branch-backed",
    );
    await expect(branchBackedListBadge).toHaveAttribute("role", "img");
    await expect(branchBackedListBadge).toHaveAccessibleName(
      "Codex — Branch-backed worktree",
    );
    await expect(ephemeralListBadge).toHaveText("Intake Agent");
    await expect(ephemeralListBadge).toHaveAttribute(
      "data-worktree-form",
      "ephemeral",
    );
    await expect(ephemeralListBadge).toHaveAttribute("role", "img");
    await expect(ephemeralListBadge).toHaveAccessibleName(
      "Intake Agent — Ephemeral branchless worktree",
    );
    await expect(unknownListBadge).toHaveText("Restored Agent");
    await expect(unknownListBadge).toHaveAttribute(
      "data-worktree-label",
      "Unknown worktree form",
    );
    await expect(unknownListBadge).toHaveAttribute("role", "img");
    await expect(unknownListBadge).toHaveAccessibleName(
      "Restored Agent — Unknown worktree form",
    );

    await expectWorktreeChromeNotToOverflow(page);
    await page.setViewportSize({ width: 640, height: 720 });
    // The list stays inside the viewport when the rail becomes a top strip.
    await expect(async () => {
      await expectWorktreeChromeNotToOverflow(page);
    }).toPass();
  });
});

async function expectWorktreeChromeNotToOverflow(page) {
  const viewport = page.viewportSize();
  expect(viewport).not.toBeNull();
  if (!viewport) {
    throw new Error("page viewport is unavailable");
  }
  const viewportBox = {
    x: 0,
    y: 0,
    width: viewport.width,
    height: viewport.height,
  };

  const rows = page.locator("#window-list-panel .window-list-row");
  await expect(rows).toHaveCount(3);
  for (let index = 0; index < 3; index += 1) {
    const rowWidths = await rows.nth(index).evaluate((element) => ({
      clientWidth: element.clientWidth,
      scrollWidth: element.scrollWidth,
    }));
    expect(rowWidths.clientWidth).toBeGreaterThan(0);
    expect(
      rowWidths.scrollWidth,
      `window list row ${index} must not overflow horizontally`,
    ).toBeLessThanOrEqual(rowWidths.clientWidth + 1);
  }
  const panelBox = await requiredBoundingBox(
    page.locator("#window-list-panel"),
    "window list panel",
  );
  expectBoxInside(panelBox, viewportBox, "window list panel in viewport");

}

async function requiredBoundingBox(locator, label) {
  const box = await locator.boundingBox();
  expect(box, `${label} must have a bounding box`).not.toBeNull();
  if (!box) {
    throw new Error(`${label} has no bounding box`);
  }
  expect(box.width, `${label} width`).toBeGreaterThan(0);
  expect(box.height, `${label} height`).toBeGreaterThan(0);
  return box;
}

function expectBoxInside(inner, outer, label) {
  const tolerance = 1;
  expect(inner.x, `${label} left`).toBeGreaterThanOrEqual(outer.x - tolerance);
  expect(inner.y, `${label} top`).toBeGreaterThanOrEqual(outer.y - tolerance);
  expect(inner.x + inner.width, `${label} right`).toBeLessThanOrEqual(
    outer.x + outer.width + tolerance,
  );
  expect(inner.y + inner.height, `${label} bottom`).toBeLessThanOrEqual(
    outer.y + outer.height + tolerance,
  );
}

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
