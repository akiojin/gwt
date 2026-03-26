import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  captureUxSnapshot,
  defaultRecentProject,
  detectedAgents,
  emitTauriEvent,
  openRecentProject,
  saveE2ECoverage,
  settingsFixture,
  profilesFixture,
  standardBranchResponses,
  waitForMenuActionListener,
} from "./support/helpers";

const issuesFixture = [
  {
    number: 101,
    title: "Fix login flow",
    body: "Login flow is broken in the current shell.",
    state: "open",
    updatedAt: "2026-03-20T10:00:00.000Z",
    htmlUrl: "https://github.com/example/gwt/issues/101",
    labels: [{ name: "bug", color: "d73a4a" }],
    assignees: [],
    commentsCount: 2,
  },
  {
    number: 202,
    title: "Spec: Improve branch browser flow",
    body: "Spec body",
    state: "open",
    updatedAt: "2026-03-21T10:00:00.000Z",
    htmlUrl: "https://github.com/example/gwt/issues/202",
    labels: [
      { name: "gwt-spec", color: "0075ca" },
      { name: "enhancement", color: "a2eeef" },
    ],
    assignees: [],
    commentsCount: 0,
  },
];

const issueDetailFixture = {
  ...issuesFixture[0],
  body: "## Investigation\n\nLogin flow is broken in the current shell and needs remediation.",
  assignees: [
    {
      login: "dev-1",
      avatarUrl: "https://example.invalid/dev-1.png",
    },
  ],
  milestone: {
    title: "Shell Stabilization",
  },
};

const specDetailFixture = {
  ...issuesFixture[1],
  body: "Spec detail body",
};

function issueResponses() {
  return {
    ...standardBranchResponses(),
    get_startup_diagnostics: {
      startupTrace: false,
      disableTray: false,
      disableLoginShellCapture: false,
      disableHeartbeatWatchdog: false,
      disableSessionWatcher: false,
      disableStartupUpdateCheck: false,
      disableProfiling: false,
      disableTabRestore: false,
      disableWindowSessionRestore: false,
    },
    check_gh_cli_status: { available: true, authenticated: true },
    fetch_github_issues: {
      issues: issuesFixture,
      hasNextPage: false,
    },
    fetch_github_issue_detail: issueDetailFixture,
    find_existing_issue_branches_bulk: [],
    search_github_issue_catalog: [issuesFixture[1]],
    search_github_issues_cmd: [
      {
        number: 202,
        title: issuesFixture[1].title,
        state: "open",
        labels: ["gwt-spec", "enhancement"],
        url: issuesFixture[1].htmlUrl,
        distance: 0.12,
      },
    ],
    index_github_issues_cmd: {
      issuesIndexed: 12,
      durationMs: 45,
    },
    detect_agents: detectedAgents,
    list_agent_versions: {
      agentId: "codex",
      package: "codex",
      tags: ["latest"],
      versions: ["0.99.0"],
      source: "cache",
    },
    get_settings: settingsFixture,
    get_profiles: profilesFixture,
  };
}

async function openIssuesWorkspace(page: import("@playwright/test").Page) {
  await openRecentProject(page);
  await page.evaluate((projectPath) => {
    window.localStorage.setItem(
      "gwt.projectTabs.v2",
      JSON.stringify({
        version: 3,
        byProjectPath: {
          [`${projectPath}::window=main`]: {
            tabs: [
              { type: "agentCanvas", id: "agentCanvas", label: "Agent Canvas" },
              { type: "branchBrowser", id: "branchBrowser", label: "Branch Browser" },
              { type: "issues", id: "issues", label: "Issues" },
            ],
            activeTabId: "issues",
          },
        },
      }),
    );
  }, defaultRecentProject.path);

  await page.reload();
}

test.afterEach(async ({ page }, testInfo) => {
  await saveE2ECoverage(page, testInfo);
});

test("opens Issues as a top-level tab in the current shell", async ({
  page,
}, testInfo) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
      ...issueResponses(),
    },
  });
  await page.goto("/");
  await openIssuesWorkspace(page);

  await expect(page.locator('[data-tab-id="issues"]')).toHaveClass(/active/);
  await expect(page.getByRole("heading", { name: "Issues" })).toBeVisible();
  await expect(page.locator(".ilp-issue-title", { hasText: "Fix login flow" })).toBeVisible();
  await captureUxSnapshot(page, testInfo, "issues-top-level-tab");
});

test("opens issue detail and renders metadata in the current shell", async ({
  page,
}, testInfo) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
      ...issueResponses(),
    },
  });
  await page.goto("/");
  await openIssuesWorkspace(page);
  await expect(page.locator('[data-tab-id="issues"]')).toHaveClass(/active/);
  await page.locator(".ilp-issue-row", { hasText: "Fix login flow" }).click();
  await expect(page.locator(".ilp-detail-title")).toContainText("Fix login flow");
  await expect(page.locator(".ilp-detail-comments")).toContainText("2 comments");
  await expect(page.locator(".ilp-detail-body")).toContainText("Investigation");
  await captureUxSnapshot(page, testInfo, "issues-detail-view");
});

test("issue detail can switch to an existing worktree", async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
      ...issueResponses(),
      find_existing_issue_branches_bulk: [
        { issueNumber: 101, branchName: "feature/workflow-demo" },
      ],
    },
  });
  await page.goto("/");
  await openIssuesWorkspace(page);
  await expect(page.locator('[data-tab-id="issues"]')).toHaveClass(/active/);
  await page.locator(".ilp-issue-row", { hasText: "Fix login flow" }).click();
  await page.getByRole("button", { name: "Switch to Worktree" }).click();

  await expect(page.locator('[data-tab-id="branchBrowser"]')).toHaveClass(/active/);
  await expect(page.getByTestId("branch-browser-detail")).toContainText("feature/workflow-demo");
});

test("issue detail launches the agent workflow when no worktree exists yet", async ({
  page,
}) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
      ...issueResponses(),
    },
  });
  await page.goto("/");
  await openIssuesWorkspace(page);
  await expect(page.locator('[data-tab-id="issues"]')).toHaveClass(/active/);
  await page.locator(".ilp-issue-row", { hasText: "Fix login flow" }).click();
  await page.getByRole("button", { name: "Work on this" }).click();

  await expect(page.getByRole("dialog", { name: "Launch Agent" })).toBeVisible();
});

test("issue search and spec index refresh stay usable in the current shell", async ({
  page,
}) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
      ...issueResponses(),
      fetch_github_issue_detail: specDetailFixture,
    },
  });
  await page.goto("/");
  await openIssuesWorkspace(page);
  await expect(page.locator('[data-tab-id="issues"]')).toHaveClass(/active/);
  await page.locator(".ilp-search-input").fill("spec");
  await expect(page.locator(".ilp-issue-title", { hasText: "Spec: Improve branch browser flow" })).toBeVisible();

  await page.getByRole("button", { name: "Update Spec Index" }).click();
  await expect(page.locator(".ilp-status")).toContainText("12 specs indexed");
  await waitForInvokeCommand(page, "index_github_issues_cmd");
});

test("issue row label filter narrows the current shell list", async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
      ...issueResponses(),
    },
  });
  await page.goto("/");
  await openIssuesWorkspace(page);
  await expect(page.locator('[data-tab-id="issues"]')).toHaveClass(/active/);
  await page.getByRole("button", { name: "bug" }).click();
  await expect(page.locator(".ilp-issue-row")).toHaveCount(1);
  await expect(page.locator(".ilp-issue-title")).toContainText("Fix login flow");
});
