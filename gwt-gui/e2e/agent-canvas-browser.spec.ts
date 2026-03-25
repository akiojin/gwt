import { expect, test, type Page } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  branchDevelop,
  branchFeature,
  branchMain,
  captureUxSnapshot,
  defaultRecentProject,
  expectAgentCanvasVisible,
  emitTauriEvent,
  waitForMenuActionListener,
  waitForInvokeCommand,
  openRecentProject,
  saveE2ECoverage,
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

async function activateTopLevelTab(page: Page, tabId: string) {
  await page.evaluate((targetTabId) => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: { activateTab: (tabId: string) => void };
      }
    ).__GWT_E2E_APP__?.activateTab(targetTabId);
  }, tabId);
  await expect(page.locator(`[data-tab-id="${tabId}"]`)).toHaveClass(/active/);
}

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test.afterEach(async ({ page }, testInfo) => {
  await saveE2ECoverage(page, testInfo);
});

test("Branch Browser can focus an existing worktree into Agent Canvas", async ({
  page,
}, testInfo) => {
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
        worktree_path: null,
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
        worktree_path: null,
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
        worktree_path: existingWorktree.path,
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
        worktree_path: null,
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
  const featureRow = visibleBrowser.locator(".branch-row").filter({
    hasText: branchFeature.name,
  }).first();
  await expect(featureRow).toBeVisible();
  await featureRow.dispatchEvent("click");
  await expect(page.getByTestId("branch-browser-detail")).toContainText(branchFeature.name);
  const focusWorktreeButton = page
    .getByTestId("branch-browser-detail")
    .getByRole("button", { name: "Focus Worktree" });
  await focusWorktreeButton.dispatchEvent("click");

  await expect(
    page.locator('[data-testid="agent-canvas-worktree-tile-feature-workflow-demo"]'),
  ).toHaveCount(1);
  await captureUxSnapshot(page, testInfo, "branch-browser-to-canvas-flow");
});

test("Agent Canvas keeps compact detail visible and exposes zoom controls", async ({
  page,
}, testInfo) => {
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
    list_branch_inventory: [
      {
        id: branchFeature.name,
        canonical_name: branchFeature.name,
        primary_branch: branchFeature,
        local_branch: branchFeature,
        remote_branch: null,
        has_local: true,
        has_remote: false,
        worktree_path: existingWorktree.path,
        worktree_count: 1,
        resolution_action: "focusExisting",
      },
    ],
    list_worktree_branches: [branchFeature],
    list_remote_branches: [],
    list_worktrees: [existingWorktree],
  });

  await openRecentProject(page);
  await activateTopLevelTab(page, "agentCanvas");
  await expectAgentCanvasVisible(page);

  const zoomLabel = page.locator('[data-testid="agent-canvas-zoom-label"]:visible');
  const worktreeTile = page.locator('[data-testid^="agent-canvas-worktree-tile-"]:visible', {
    hasText: branchFeature.name,
  });
  await expect(page.locator('[data-testid="agent-canvas-detail-overlay"]')).toHaveCount(0);
  await expect(worktreeTile).toBeVisible();
  await expect(page.getByTestId("agent-canvas-zoom-controls")).toBeVisible();
  await expect(page.getByTestId("agent-canvas-assistant-tile")).toBeVisible();

  await expect(zoomLabel).toHaveText("100%");
  await captureUxSnapshot(page, testInfo, "agent-canvas-ux-board");
});

test("Agent Canvas renders terminal session content directly inside the tile", async ({
  page,
}, testInfo) => {
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
  await activateTopLevelTab(page, "agentCanvas");
  await expectAgentCanvasVisible(page);
  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  const sessionTile = page.locator('[data-testid^="agent-canvas-session-terminal-"]:visible').first();
  const sessionSurface = page.locator(
    '[data-testid^="agent-canvas-session-surface-terminal-"]:visible',
  ).first();
  await expect(sessionTile).toBeVisible();
  await expect(sessionSurface).toBeVisible();
  await expect(page.locator('[data-testid="agent-canvas-detail-overlay"]')).toHaveCount(0);
  await captureUxSnapshot(page, testInfo, "agent-canvas-terminal-session-tile");
});
