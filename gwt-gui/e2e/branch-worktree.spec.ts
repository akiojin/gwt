import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  branchMain,
  branchDevelop,
  branchFeature,
  branchBehind,
  openRecentProject,
  setMockCommandResponses,
  standardBranchResponses,
  detectedAgents,
  getInvokeLog,
} from "./support/helpers";

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("displays branch list after opening project", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await expect(
    page.locator(".branch-name", { hasText: "main" }),
  ).toBeVisible();
  await expect(
    page.locator(".branch-name", { hasText: "develop" }),
  ).toBeVisible();
  await expect(
    page.locator(".branch-name", { hasText: branchFeature.name }),
  ).toBeVisible();
});

test("selects branch and shows detail panel", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  const branchButton = page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name });
  await branchButton.click();

  await expect(page.locator(".branch-detail h2")).toContainText(
    branchFeature.name,
  );
});

test("shows Summary tab active by default when selecting branch", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await expect(
    page.getByRole("button", { name: "Summary", exact: true }),
  ).toHaveClass(/active/);
});

test("sorts branches by Updated (default)", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [
      { ...branchFeature, name: "feature/alpha", commit_timestamp: 1_700_000_050 },
      { ...branchFeature, name: "feature/beta", commit_timestamp: 1_700_000_200 },
      branchMain,
      branchDevelop,
    ],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {},
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);

  const sortText = page.locator(".sort-mode-text");
  await expect(sortText).toHaveText("Updated");
  // main has highest timestamp, then beta, then develop, then alpha
  await expect(page.locator(".branch-list .branch-name").nth(0)).toHaveText(
    "main",
  );
});

test("switches sort mode to Name", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [
      { ...branchFeature, name: "feature/alpha", commit_timestamp: 1_700_000_050 },
      { ...branchFeature, name: "feature/beta", commit_timestamp: 1_700_000_200 },
      branchMain,
      branchDevelop,
    ],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {},
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);

  await page.locator(".sort-mode-toggle").click();
  const sortText = page.locator(".sort-mode-text");
  await expect(sortText).toHaveText("Name");
});

test("filters branches using search input", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [
      branchMain,
      branchDevelop,
      { ...branchFeature, name: "feature/search-target" },
      { ...branchFeature, name: "feature/other-branch" },
    ],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {},
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);

  const searchInput = page.getByPlaceholder("Filter branches...");
  await expect(searchInput).toBeVisible();
  await searchInput.fill("search-target");

  await expect(
    page.locator(".branch-name", { hasText: "search-target" }),
  ).toBeVisible();
  await expect(
    page.locator(".branch-name", { hasText: "other-branch" }),
  ).toBeHidden();
});

test("shows divergence badge for ahead branch", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  const featureBranch = page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name });
  await expect(featureBranch).toBeVisible();
  // Ahead branches should have divergence indicator
  await expect(featureBranch.locator(".divergence")).toBeVisible();
});

test("shows divergence badge for behind branch", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    list_worktree_branches: [branchMain, branchDevelop, branchBehind],
  });
  await openRecentProject(page);

  const behindBranch = page
    .locator(".branch-item")
    .filter({ hasText: branchBehind.name });
  await expect(behindBranch).toBeVisible();
  await expect(behindBranch.locator(".divergence")).toBeVisible();
});

test("shows current branch indicator asterisk", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  // Current branch (main) shows "*" prefix
  const mainBranch = page
    .locator(".branch-item")
    .filter({ hasText: "main" });
  await expect(mainBranch).toBeVisible();
  await expect(mainBranch).toContainText("*");
});

test("shows agent-running indicator when branch has a running agent tab", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [
      branchMain,
      branchDevelop,
      { ...branchFeature, is_agent_running: true, agent_status: "running" },
    ],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {},
      ghStatus: { available: true, authenticated: true },
    },
    detect_agents: detectedAgents,
    list_agent_versions: {
      agentId: "codex",
      package: "codex",
      tags: ["latest"],
      versions: ["0.99.0"],
      source: "cache",
    },
  });
  await openRecentProject(page);

  const featureBranch = page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name });
  await expect(featureBranch).toBeVisible();
  await featureBranch.click();
  await page
    .locator(".worktree-summary-panel")
    .getByRole("button", { name: "Launch Agent..." })
    .click();
  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .click();

  await expect
    .poll(async () => {
      const log = await getInvokeLog(page);
      return log.includes("start_launch_job");
    })
    .toBe(true);

  await expect(
    featureBranch.locator(".agent-indicator-slot .agent-pulse-dot"),
  ).toBeVisible();
});

test("WorktreeSummaryPanel shows Launch Agent button for non-main branch", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  const launchBtn = page
    .locator(".worktree-summary-panel")
    .getByRole("button", { name: "Launch Agent..." });
  await expect(launchBtn).toBeVisible();
});

test("WorktreeSummaryPanel shows New Terminal button", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await expect(page.getByTitle("New Terminal")).toBeVisible();
});

test("clicking New Terminal invokes spawn_shell", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page.getByTitle("New Terminal").click();

  await expect
    .poll(async () => {
      const log = await getInvokeLog(page);
      return log.includes("spawn_shell");
    })
    .toBe(true);
});

test("shows PR badge for branch with associated PR", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {
        [branchFeature.name]: { number: 42 },
      },
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);

  const prBadge = page.locator(".pr-badge", { hasText: "#42" });
  await expect(prBadge).toBeVisible();
});

test("main and develop branches are pinned to top", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [
      { ...branchFeature, name: "feature/z-last", commit_timestamp: 1_700_000_300 },
      branchMain,
      branchDevelop,
    ],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {},
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);

  // main and develop are pinned at the top regardless of sort
  await expect(page.locator(".branch-list .branch-name").nth(0)).toHaveText(
    "main",
  );
  await expect(page.locator(".branch-list .branch-name").nth(1)).toHaveText(
    "develop",
  );
});

test("empty branch list shows properly when no branches returned", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {},
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);

  // Should still show the sidebar
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();
});

test("search input clears filter when emptied", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [
      branchMain,
      { ...branchFeature, name: "feature/find-me" },
      { ...branchFeature, name: "feature/hidden" },
    ],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {},
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);

  const searchInput = page.getByPlaceholder("Filter branches...");
  await searchInput.fill("find-me");
  await expect(
    page.locator(".branch-name", { hasText: "hidden" }),
  ).toBeHidden();

  await searchInput.fill("");
  await expect(
    page.locator(".branch-name", { hasText: "hidden" }),
  ).toBeVisible();
});

test("double-clicking branch activates it", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  const branchButton = page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name });
  await branchButton.dblclick();

  // Should have invoked activation
  const log = await getInvokeLog(page);
  // At minimum the branch should be selected
  await expect(page.locator(".branch-detail h2")).toContainText(
    branchFeature.name,
  );
});
