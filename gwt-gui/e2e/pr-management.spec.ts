import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  branchFeature,
  captureUxSnapshot,
  defaultRecentProject,
  openRecentProject,
  saveE2ECoverage,
  setMockCommandResponses,
  standardBranchResponses,
  waitForInvokeCommand,
} from "./support/helpers";

const openPrList = {
  items: [
    {
      number: 42,
      title: "Workflow Demo PR",
      state: "OPEN",
      isDraft: false,
      headRefName: branchFeature.name,
      baseRefName: "main",
      author: { login: "e2e" },
      labels: [{ name: "bugfix", color: "d73a4a" }],
      createdAt: "2026-03-19T10:00:00.000Z",
      updatedAt: "2026-03-22T10:00:00.000Z",
      url: "https://github.com/example/gwt/pull/42",
      body: "## Summary\n\nCurrent shell PR body.",
      reviewRequests: [{ login: "reviewer-1" }],
      assignees: [{ login: "reviewer-1" }],
    },
    {
      number: 77,
      title: "Draft shell cleanup",
      state: "OPEN",
      isDraft: true,
      headRefName: "feature/draft-shell-cleanup",
      baseRefName: "develop",
      author: { login: "dev-1" },
      labels: [],
      createdAt: "2026-03-18T10:00:00.000Z",
      updatedAt: "2026-03-21T10:00:00.000Z",
      url: "https://github.com/example/gwt/pull/77",
      body: "",
      reviewRequests: [],
      assignees: [],
    },
  ],
  ghStatus: { available: true, authenticated: true },
};

const mergedPrList = {
  items: [
    {
      number: 99,
      title: "Merged shell stabilization",
      state: "MERGED",
      isDraft: false,
      headRefName: "feature/merged-shell",
      baseRefName: "main",
      author: { login: "e2e" },
      labels: [],
      createdAt: "2026-03-10T10:00:00.000Z",
      updatedAt: "2026-03-20T10:00:00.000Z",
      url: "https://github.com/example/gwt/pull/99",
      body: "Merged body",
      reviewRequests: [],
      assignees: [],
    },
  ],
  ghStatus: { available: true, authenticated: true },
};

const prDetailFixture = {
  number: 42,
  title: "Workflow Demo PR",
  state: "OPEN",
  url: "https://github.com/example/gwt/pull/42",
  mergeable: "MERGEABLE",
  author: "e2e",
  baseBranch: "main",
  headBranch: branchFeature.name,
  labels: ["bugfix"],
  assignees: ["reviewer-1"],
  milestone: null,
  linkedIssues: [101],
  checkSuites: [
    {
      workflowName: "CI Build",
      runId: 100,
      status: "completed",
      conclusion: "success",
    },
    {
      workflowName: "Lint",
      runId: 101,
      status: "in_progress",
      conclusion: null,
    },
  ],
  reviews: [{ reviewer: "reviewer-1", state: "APPROVED" }],
  reviewComments: [],
  changedFilesCount: 2,
  additions: 12,
  deletions: 3,
  mergeStateStatus: "BEHIND",
};

function prResponses() {
  return {
    ...standardBranchResponses(),
    check_gh_cli_status: { available: true, authenticated: true },
    fetch_github_user: {
      login: "e2e",
      ghStatus: { available: true, authenticated: true },
    },
    fetch_pr_list: openPrList,
    fetch_pr_detail: prDetailFixture,
  };
}

async function seedPullRequestsTab(page: import("@playwright/test").Page) {
  await page.evaluate((projectPath) => {
    window.localStorage.setItem(
      "gwt.projectTabs.v2",
      JSON.stringify({
        version: 2,
        byProjectPath: {
          [projectPath]: {
            tabs: [
              { type: "agentCanvas", id: "agentCanvas", label: "Agent Canvas" },
              { type: "branchBrowser", id: "branchBrowser", label: "Branch Browser" },
              { type: "prs", id: "prs", label: "Pull Requests" },
            ],
            activeTabId: "prs",
          },
        },
      }),
    );
  }, defaultRecentProject.path);
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

test("opens Pull Requests as a top-level tab in the current shell", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await seedPullRequestsTab(page);
  await setMockCommandResponses(page, prResponses());
  await openRecentProject(page);

  await expect(page.locator('[data-tab-id="prs"]')).toHaveClass(/active/);
  await expect(page.getByRole("heading", { name: "Pull Requests" })).toBeVisible();
  await expect(page.locator(".plp-pr-title", { hasText: "Workflow Demo PR" })).toBeVisible();
  await captureUxSnapshot(page, testInfo, "prs-top-level-tab");
});

test("expanded PR row shows body, checks, and reviews", async ({ page }) => {
  await page.goto("/");
  await seedPullRequestsTab(page);
  await setMockCommandResponses(page, prResponses());
  await openRecentProject(page);

  await page.locator(".plp-pr-row", { hasText: "Workflow Demo PR" }).click();
  await expect(page.locator(".plp-pr-expanded")).toContainText("Current shell PR body");
  await expect(page.locator(".plp-check-name", { hasText: "CI Build" })).toBeVisible();
  await expect(page.locator(".plp-review-item")).toContainText("reviewer-1");
});

test("PR state filter can switch to merged items in the current shell", async ({
  page,
}) => {
  await page.goto("/");
  await seedPullRequestsTab(page);
  await setMockCommandResponses(page, prResponses());
  await openRecentProject(page);

  await setMockCommandResponses(page, {
    ...prResponses(),
    fetch_pr_list: mergedPrList,
  });
  await page.getByRole("button", { name: "Merged" }).click();

  await expect(page.locator(".plp-pr-title", { hasText: "Merged shell stabilization" })).toBeVisible();
});

test("PR row can switch back to Branch Browser worktree", async ({ page }) => {
  await page.goto("/");
  await seedPullRequestsTab(page);
  await setMockCommandResponses(page, prResponses());
  await openRecentProject(page);

  await page.locator(".plp-pr-row", { hasText: "Workflow Demo PR" }).getByRole("button", { name: "WT" }).click();

  await expect(page.locator('[data-tab-id="branchBrowser"]')).toHaveClass(/active/);
  await expect(page.getByTestId("branch-browser-panel")).toBeVisible();
});

test("behind PR can request update branch", async ({ page }) => {
  await page.goto("/");
  await seedPullRequestsTab(page);
  await setMockCommandResponses(page, prResponses());
  await openRecentProject(page);

  await page.locator(".plp-pr-row", { hasText: "Workflow Demo PR" }).getByRole("button", { name: "Update" }).click();
  await waitForInvokeCommand(page, "update_pr_branch");
});

test("draft PR can be marked ready", async ({ page }) => {
  await page.goto("/");
  await seedPullRequestsTab(page);
  await setMockCommandResponses(page, prResponses());
  await openRecentProject(page);

  await page.locator(".plp-pr-row", { hasText: "Draft shell cleanup" }).getByRole("button", { name: "Ready" }).click();
  await waitForInvokeCommand(page, "mark_pr_ready");
});

test("missing gh authentication shows PR availability message", async ({
  page,
}) => {
  await page.goto("/");
  await seedPullRequestsTab(page);
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    check_gh_cli_status: { available: false, authenticated: false },
  });
  await openRecentProject(page);

  await expect(page.locator(".plp-error")).toContainText("GitHub CLI");
});
