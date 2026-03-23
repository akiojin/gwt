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
  saveE2ECoverage,
} from "./support/helpers";

async function seedBranchBrowserAsActive(
  page: import("@playwright/test").Page,
) {
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
}

async function expectBranchBrowserVisible(page: import("@playwright/test").Page) {
  await expect(page.locator('[data-testid="branch-browser-panel"]:visible')).toBeVisible();
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

test("displays branch list after opening project", async ({ page }, testInfo) => {
  await page.goto("/");
  await seedBranchBrowserAsActive(page);
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await expectBranchBrowserVisible(page);
  await expect(page.locator(".branch-name", { hasText: "main" })).toBeVisible();
  await expect(page.locator(".branch-name", { hasText: "develop" })).toBeVisible();
  await expect(
    page.locator(".branch-name", { hasText: branchFeature.name }),
  ).toBeVisible();
  await captureUxSnapshot(page, testInfo, "branch-browser-default-layout");
});

test("sorts branches by Updated by default", async ({ page }) => {
  await page.goto("/");
  await seedBranchBrowserAsActive(page);
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

  await expectBranchBrowserVisible(page);
  await expect(page.locator(".branch-row .branch-name").nth(0)).toHaveText("main");
});

test("shows divergence badge for ahead branch", async ({ page }) => {
  await page.goto("/");
  await seedBranchBrowserAsActive(page);
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await expectBranchBrowserVisible(page);
  const featureBranch = page.locator(".branch-row").filter({ hasText: branchFeature.name });
  await expect(featureBranch).toBeVisible();
  await expect(featureBranch.locator(".divergence-pill")).toBeVisible();
});

test("shows divergence badge for behind branch", async ({ page }) => {
  await page.goto("/");
  await seedBranchBrowserAsActive(page);
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    list_worktree_branches: [branchMain, branchDevelop, branchBehind],
  });
  await openRecentProject(page);

  await expectBranchBrowserVisible(page);
  const behindBranch = page.locator(".branch-row").filter({ hasText: branchBehind.name });
  await expect(behindBranch).toBeVisible();
  await expect(behindBranch.locator(".divergence-pill")).toBeVisible();
});
