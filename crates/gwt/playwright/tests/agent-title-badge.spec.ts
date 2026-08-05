import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Agent title role and worktree badges", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("fallback runtime title still displays the Agent runtime badge", async ({
    page,
  }, testInfo) => {
    await installEmbeddedRoutes(page);
    await installAgentTitleBadgeBackend(page);

    await page.goto(APP_URL);

    const agentWindow = page.locator(".workspace-window[data-id='agent-1']");
    await expect(agentWindow).toBeVisible({ timeout: 10_000 });
    await expect(agentWindow.locator(".title-text")).toHaveText("Codex");

    const badge = agentWindow.locator(".window-role-badge").first();
    await expect(badge).toBeVisible();
    await expect(badge).toHaveText("Codex");

    await agentWindow.screenshot({
      path:
        process.env.GWT_AGENT_BADGE_SCREENSHOT_PATH ||
        testInfo.outputPath("agent-title-badge.png"),
    });
  });

  test("legacy wire forms render semantic badges across agent chrome", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installAgentTitleBadgeBackend(page);

    await page.goto(APP_URL);

    const branchBackedWindow = page.locator(
      ".workspace-window[data-id='agent-1']",
    );
    const ephemeralWindow = page.locator(".workspace-window[data-id='agent-2']");
    const unknownWindow = page.locator(".workspace-window[data-id='agent-3']");
    await expect(branchBackedWindow).toBeVisible({ timeout: 10_000 });
    await expect(ephemeralWindow).toBeVisible();
    await expect(unknownWindow).toBeVisible();

    const branchBackedBadge = branchBackedWindow.locator(
      ".window-worktree-badge",
    );
    await expect(branchBackedBadge).toBeVisible();
    await expect(branchBackedBadge).toHaveText("Branch-backed");
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
      "Branch-backed worktree",
    );
    await expect(branchBackedBadge).toHaveAttribute(
      "title",
      "Branch-backed worktree",
    );

    const ephemeralBadge = ephemeralWindow.locator(".window-worktree-badge");
    await expect(ephemeralBadge).toBeVisible();
    await expect(ephemeralBadge).toHaveText("Ephemeral");
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
      "Ephemeral branchless worktree",
    );

    const unknownBadge = unknownWindow.locator(".window-worktree-badge");
    await expect(unknownBadge).toBeVisible();
    await expect(unknownBadge).toHaveText("?");
    await expect(unknownBadge).toHaveAttribute("data-worktree-form", "unknown");
    await expect(unknownBadge).toHaveAttribute(
      "data-worktree-label",
      "Unknown worktree form",
    );
    await expect(unknownBadge).toHaveAttribute("data-worktree-symbol", "?");
    await expect(unknownBadge).toHaveAttribute(
      "aria-label",
      "Unknown worktree form",
    );

    const minimap = page.locator("#fleet-minimap");
    await expect(minimap).toBeVisible();
    const minimapBranchBacked = minimap.locator(
      '.fleet-minimap__cell[data-window-id="agent-1"]',
    );
    const minimapEphemeral = minimap.locator(
      '.fleet-minimap__cell[data-window-id="agent-2"]',
    );
    const minimapUnknown = minimap.locator(
      '.fleet-minimap__cell[data-window-id="agent-3"]',
    );
    await expect(minimapBranchBacked).toHaveAttribute(
      "data-worktree-form",
      "branch-backed",
    );
    await expect(minimapBranchBacked).toHaveAttribute(
      "data-worktree-label",
      "Branch-backed",
    );
    await expect(minimapBranchBacked).toHaveAttribute(
      "data-worktree-symbol",
      "B",
    );
    await expect(minimapEphemeral).toHaveAttribute(
      "data-worktree-form",
      "ephemeral",
    );
    await expect(minimapEphemeral).toHaveAttribute(
      "data-worktree-label",
      "Ephemeral",
    );
    await expect(minimapEphemeral).toHaveAttribute(
      "data-worktree-symbol",
      "Ø",
    );
    await expect(minimapUnknown).toHaveAttribute("data-worktree-form", "unknown");
    await expect(minimapUnknown).toHaveAttribute(
      "data-worktree-label",
      "Unknown worktree form",
    );
    await expect(minimapUnknown).toHaveAttribute("data-worktree-symbol", "?");

    await page.locator("#window-list-button").click();
    const panel = page.locator("#window-list-panel");
    await expect(panel).toBeVisible();
    const branchBackedListBadge = panel
      .locator(".window-list-row", { hasText: "Codex" })
      .first()
      .locator(".window-list-worktree");
    const ephemeralListBadge = panel
      .locator(".window-list-row", { hasText: "Intake Agent" })
      .first()
      .locator(".window-list-worktree");
    const unknownListBadge = panel
      .locator(".window-list-row", { hasText: "Restored Agent" })
      .first()
      .locator(".window-list-worktree");
    await expect(branchBackedListBadge).toHaveText("Branch-backed");
    await expect(branchBackedListBadge).toHaveAttribute(
      "data-worktree-form",
      "branch-backed",
    );
    await expect(ephemeralListBadge).toHaveText("Ephemeral");
    await expect(ephemeralListBadge).toHaveAttribute(
      "data-worktree-form",
      "ephemeral",
    );
    await expect(unknownListBadge).toHaveText("?");
    await expect(unknownListBadge).toHaveAttribute(
      "data-worktree-label",
      "Unknown worktree form",
    );

    await expectWorktreeChromeNotToOverflow(page);
    await page.setViewportSize({ width: 640, height: 720 });
    await expectWorktreeChromeNotToOverflow(page);
  });
});

async function expectWorktreeChromeNotToOverflow(page) {
  await expect
    .poll(() =>
      page.locator(".workspace-window .window-titlebar").evaluateAll((nodes) =>
        nodes.every((node) => node.scrollWidth <= node.clientWidth + 1),
      ),
    )
    .toBe(true);
  await expect
    .poll(() =>
      page.locator("#window-list-panel .window-list-row").evaluateAll((nodes) =>
        nodes.every((node) => node.scrollWidth <= node.clientWidth + 1),
      ),
    )
    .toBe(true);
  const panelBox = await page.locator("#window-list-panel").boundingBox();
  const viewport = page.viewportSize();
  expect(panelBox).not.toBeNull();
  expect(viewport).not.toBeNull();
  expect(panelBox.x).toBeGreaterThanOrEqual(0);
  expect(panelBox.x + panelBox.width).toBeLessThanOrEqual(viewport.width + 1);
}

async function installAgentTitleBadgeBackend(page) {
  await page.addInitScript(() => {
    const windows = [
      {
        id: "agent-1",
        title: "Codex",
        preset: "agent",
        geometry: { x: 180, y: 120, width: 720, height: 360 },
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
        agent_id: "codex",
        agent_color: "cyan",
        lane_kind: "execution",
        tab_group_id: null,
        tab_group_active: false,
      },
      {
        id: "agent-2",
        title: "Intake Agent",
        preset: "agent",
        geometry: { x: 940, y: 120, width: 600, height: 320 },
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
        geometry: { x: 180, y: 520, width: 720, height: 300 },
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

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
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

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}
