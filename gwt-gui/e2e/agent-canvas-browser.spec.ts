import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  branchDevelop,
  branchFeature,
  branchMain,
  defaultRecentProject,
  emitTauriEvent,
  waitForMenuActionListener,
  waitForInvokeCommand,
  openRecentProject,
  setMockCommandResponses,
} from "./support/helpers";

const existingWorktree = {
  path: "/tmp/gwt-playwright/.gwt/worktrees/feature-workflow-demo",
  branch: branchFeature.name,
  commit: branchFeature.commit,
  status: "active",
  is_main: false,
  has_changes: false,
  has_unpushed: false,
  is_current: false,
  is_protected: false,
  is_agent_running: false,
  agent_status: "unknown",
  ahead: 1,
  behind: 0,
  is_gone: false,
  last_tool_usage: null,
  safety_level: "warning",
};

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("Branch Browser can focus an existing worktree and create a remote one into Agent Canvas", async ({
  page,
}) => {
  await page.goto("/");
  await page.evaluate(() => {
    window.localStorage.setItem(
      "gwt.projectTabs.v2",
      JSON.stringify({
        version: 2,
        byProjectPath: {
          "/tmp/gwt-playwright": {
            tabs: [
              { type: "agentCanvas", id: "agentCanvas", label: "Agent Canvas" },
              { type: "branchBrowser", id: "branchBrowser", label: "Branch Browser" },
            ],
            activeTabId: "branchBrowser",
          },
        },
      }),
    );
  });
  await setMockCommandResponses(page, {
    list_branch_inventory: [
      {
        id: branchMain.name,
        canonical_name: branchMain.name,
        primary_branch: branchMain,
        local_branch: branchMain,
        remote_branch: null,
        has_local: true,
        has_remote: false,
        worktree: null,
        worktree_count: 0,
        resolution_action: "createWorktree",
      },
      {
        id: branchDevelop.name,
        canonical_name: branchDevelop.name,
        primary_branch: branchDevelop,
        local_branch: branchDevelop,
        remote_branch: null,
        has_local: true,
        has_remote: false,
        worktree: null,
        worktree_count: 0,
        resolution_action: "createWorktree",
      },
      {
        id: branchFeature.name,
        canonical_name: branchFeature.name,
        primary_branch: branchFeature,
        local_branch: branchFeature,
        remote_branch: null,
        has_local: true,
        has_remote: false,
        worktree: existingWorktree,
        worktree_count: 1,
        resolution_action: "focusExisting",
      },
      {
        id: "feature/new-browser-flow",
        canonical_name: "feature/new-browser-flow",
        primary_branch: {
          ...branchFeature,
          name: "origin/feature/new-browser-flow",
          commit: "remote123",
        },
        local_branch: null,
        remote_branch: {
          ...branchFeature,
          name: "origin/feature/new-browser-flow",
          commit: "remote123",
        },
        has_local: false,
        has_remote: true,
        worktree: null,
        worktree_count: 0,
        resolution_action: "createWorktree",
      },
    ],
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [
      { ...branchFeature, name: "origin/feature/new-browser-flow", commit: "remote123" },
    ],
    list_worktrees: [existingWorktree],
  });
  await openRecentProject(page);
  await waitForInvokeCommand(page, "list_branch_inventory");
  const visibleBrowser = page.locator('[data-testid="branch-browser-panel"]:visible');
  await expect(visibleBrowser).toBeVisible();
  const listPanel = page.locator(".branch-list-panel");
  const detailPanel = page.locator('[data-testid="branch-browser-detail"]');
  const listBox = await listPanel.boundingBox();
  const detailBox = await detailPanel.boundingBox();
  if (!listBox || !detailBox) throw new Error("branch browser layout boxes missing");
  expect(Math.abs(listBox.x - detailBox.x)).toBeLessThan(24);
  expect(listBox.y).toBeGreaterThan(detailBox.y);
  await expect(page.locator(".branch-row", { hasText: branchFeature.name })).toBeVisible();

  await page.locator(".branch-row", { hasText: branchFeature.name }).click();
  await page.getByRole("button", { name: "Focus Worktree" }).click();

  await expect(
    page.locator('[data-testid^="agent-canvas-worktree-card-"]', {
      hasText: branchFeature.name,
    }),
  ).toBeVisible();

  await page.getByRole("tab", { name: "Branch Browser" }).click();
  await expect(page.locator('[data-testid="branch-browser-panel"]:visible')).toBeVisible();
  await page.getByRole("button", { name: "Remote" }).click();
  await expect(page.locator(".branch-row", { hasText: "origin/feature/new-browser-flow" })).toBeVisible();
  await page.locator(".branch-row", { hasText: "origin/feature/new-browser-flow" }).click();
  await page.getByRole("button", { name: "Create Worktree" }).click();

  await expect(
    page.locator('[data-testid^="agent-canvas-worktree-card-"]', {
      hasText: "feature/new-browser-flow",
    }),
  ).toBeVisible();
});

test("Agent Canvas keeps compact detail visible and exposes zoom controls", async ({
  page,
}) => {
  await page.goto("/");
  await page.evaluate(() => {
    window.localStorage.setItem(
      "gwt.projectTabs.v2",
      JSON.stringify({
        version: 2,
        byProjectPath: {
          "/tmp/gwt-playwright": {
            tabs: [
              { type: "agentCanvas", id: "agentCanvas", label: "Agent Canvas" },
              { type: "branchBrowser", id: "branchBrowser", label: "Branch Browser" },
            ],
            activeTabId: "branchBrowser",
          },
        },
      }),
    );
  });
  await setMockCommandResponses(page, {
    list_branch_inventory: [
      {
        id: branchFeature.name,
        canonical_name: branchFeature.name,
        primary_branch: branchFeature,
        local_branch: branchFeature,
        remote_branch: null,
        has_local: true,
        has_remote: false,
        worktree: existingWorktree,
        worktree_count: 1,
        resolution_action: "focusExisting",
      },
    ],
    list_worktree_branches: [branchFeature],
    list_remote_branches: [],
    list_worktrees: [existingWorktree],
  });

  await openRecentProject(page);
  await waitForInvokeCommand(page, "list_branch_inventory");
  await expect(page.locator('[data-testid="branch-browser-panel"]:visible')).toBeVisible();
  await page.locator(".branch-row", { hasText: branchFeature.name }).click();
  await page.getByRole("button", { name: "Focus Worktree" }).click();
  await page
    .locator('[data-tab-id="agentCanvas"]')
    .evaluate((node) => (node as HTMLElement).click());

  const zoomLabel = page.locator('[data-testid="agent-canvas-zoom-label"]');
  const boardPanel = page.locator(".canvas-board-panel");
  const worktreeCard = page.locator('[data-testid^="agent-canvas-worktree-card-"]', {
    hasText: branchFeature.name,
  });
  await expect(page.getByRole("heading", { name: "Agent Canvas" })).toBeVisible();
  await expect(page.locator('[data-testid="agent-canvas-detail-overlay"]')).toHaveCount(0);
  await expect(worktreeCard).toBeVisible();
  const viewport = page.viewportSize();
  const boardBox = await boardPanel.boundingBox();
  if (!viewport || !boardBox) throw new Error("canvas board box missing");
  expect(boardBox.width).toBeGreaterThan(viewport.width * 0.7);

  await page.getByLabel("Zoom in").click();
  await expect(zoomLabel).toHaveText("110%");
  await page.getByTestId("agent-canvas-assistant-card").click();
  await expect(page.getByTestId("agent-canvas-detail-overlay")).toBeVisible();
});

test("Agent Canvas renders terminal session content directly inside the card", async ({
  page,
}) => {
  await page.goto("/");
  await page.evaluate(() => {
    window.localStorage.setItem(
      "gwt.projectTabs.v2",
      JSON.stringify({
        version: 2,
        byProjectPath: {
          "/tmp/gwt-playwright": {
            tabs: [
              { type: "agentCanvas", id: "agentCanvas", label: "Agent Canvas" },
              { type: "branchBrowser", id: "branchBrowser", label: "Branch Browser" },
            ],
            activeTabId: "agentCanvas",
          },
        },
      }),
    );
  });

  await setMockCommandResponses(page, {
    list_worktree_branches: [branchFeature],
    list_remote_branches: [],
    list_worktrees: [existingWorktree],
    terminal_ready: Array.from(new TextEncoder().encode("mock terminal output\n")),
  });

  await openRecentProject(page);
  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  const sessionSurface = page.getByTestId(
    "agent-canvas-session-surface-terminal-mock-pane-1",
  );
  await expect(sessionSurface).toBeVisible();
  await expect(page.getByText("mock terminal output")).toBeVisible();
  await expect(page.locator('[data-testid="agent-canvas-detail-overlay"]')).toHaveCount(0);
});
