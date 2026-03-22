import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  branchMain,
  branchDevelop,
  branchFeature,
  branchBehind,
  captureUxSnapshot,
  openRecentProject,
  setMockCommandResponses,
  standardBranchResponses,
  selectBranchInBrowser,
  openBranchBrowser,
  expectAgentCanvasVisible,
  waitForMenuActionListener,
  emitTauriEvent,
  getInvokeLog,
} from "./support/helpers";

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("displays branch list after opening project", async ({ page }, testInfo) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await openBranchBrowser(page);
  const listPanel = page.locator(".branch-list-panel");
  const detailPanel = page.locator('[data-testid="branch-browser-detail"]');
  const listBox = await listPanel.boundingBox();
  const detailBox = await detailPanel.boundingBox();
  if (!listBox || !detailBox) throw new Error("branch browser layout boxes missing");
  expect(Math.abs(listBox.x - detailBox.x)).toBeLessThan(24);
  expect(listBox.y).toBeGreaterThan(detailBox.y);
  expect(detailBox.height).toBeGreaterThan(40);
  await expect(page.locator(".branch-name", { hasText: "main" })).toBeVisible();
  await expect(page.locator(".branch-name", { hasText: "develop" })).toBeVisible();
  await expect(
    page.locator(".branch-name", { hasText: branchFeature.name }),
  ).toBeVisible();
  await captureUxSnapshot(page, testInfo, "branch-browser-default-layout");
});

test("selects branch and shows branch-browser detail panel", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await selectBranchInBrowser(page, branchFeature.name);
  await expect(page.getByTestId("branch-browser-detail")).toContainText(
    branchFeature.name,
  );
});

test("sorts branches by Updated by default", async ({ page }) => {
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

  await openBranchBrowser(page);
  await expect(page.locator(".branch-row .branch-name").nth(0)).toHaveText("main");
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

  await openBranchBrowser(page);
  const searchInput = page.getByPlaceholder("Filter branches...");
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

  await openBranchBrowser(page);
  const featureBranch = page.locator(".branch-row").filter({ hasText: branchFeature.name });
  await expect(featureBranch).toBeVisible();
  await expect(featureBranch.locator(".divergence-pill")).toBeVisible();
});

test("shows divergence badge for behind branch", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    list_worktree_branches: [branchMain, branchDevelop, branchBehind],
  });
  await openRecentProject(page);

  await openBranchBrowser(page);
  const behindBranch = page.locator(".branch-row").filter({ hasText: branchBehind.name });
  await expect(behindBranch).toBeVisible();
  await expect(behindBranch.locator(".divergence-pill")).toBeVisible();
});

test("materializes a worktree from Branch Browser into Agent Canvas", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await selectBranchInBrowser(page, branchFeature.name);
  await page.getByRole("button", { name: "Create Worktree" }).click();
  await expectAgentCanvasVisible(page);
  const worktreeCard = page.locator('[data-testid^="agent-canvas-worktree-card-"]', {
    hasText: branchFeature.name,
  });
  await expect(worktreeCard).toBeVisible();
  const cardBox = await worktreeCard.boundingBox();
  if (!cardBox) throw new Error("worktree card bounding box missing");
  expect(cardBox.width).toBeGreaterThan(240);
  expect(cardBox.height).toBeGreaterThan(150);
  await captureUxSnapshot(page, testInfo, "branch-browser-materialized-worktree-card");
});

test("new terminal can be launched after worktree materialization", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await selectBranchInBrowser(page, branchFeature.name);
  await page.getByRole("button", { name: "Create Worktree" }).click();
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  await expect
    .poll(async () => {
      const log = await getInvokeLog(page);
      return log.includes("spawn_shell");
    })
    .toBe(true);
});
