import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  branchMain,
  branchDevelop,
  branchFeature,
  prStatusFixture,
  openRecentProject,
  setMockCommandResponses,
  waitForMenuActionListener,
  emitTauriEvent,
  getInvokeLog,
  waitForEventListener,
} from "./support/helpers";

const prBranchResponses = {
  list_worktree_branches: [branchMain, branchDevelop, branchFeature],
  list_remote_branches: [],
  list_worktrees: [],
  fetch_pr_status: {
    statuses: {
      [branchFeature.name]: { number: 42 },
      [branchMain.name]: null,
      [branchDevelop.name]: null,
    },
    ghStatus: { available: true, authenticated: true },
  },
  fetch_pr_detail: prStatusFixture,
  get_branch_session_summary: {
    status: "ok",
    generating: false,
    toolId: null,
    sessionId: null,
    markdown: "",
    bulletPoints: [],
    error: null,
  },
};

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("shows PR badge in sidebar for branch with PR", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, prBranchResponses);
  await openRecentProject(page);

  await expect(page.locator(".pr-badge", { hasText: "#42" })).toBeVisible();
});

test("Summary > PR tab shows PR title", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, prBranchResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "PR", exact: true })
    .click();

  await expect(page.locator(".pr-title")).toBeVisible();
  await expect(page.locator(".pr-title")).toContainText(
    prStatusFixture.title,
  );
});

test("Summary > PR tab shows check suites section", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, prBranchResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "PR", exact: true })
    .click();

  const checksToggle = page.locator(".checks-section .checks-toggle");
  await expect(checksToggle).toBeVisible();
});

test("expanding checks shows CI Build and Lint items", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, prBranchResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "PR", exact: true })
    .click();

  await page.locator(".checks-section .checks-toggle").click();
  await expect(
    page.locator(".check-item .check-name", { hasText: "CI Build" }),
  ).toBeVisible();
  await expect(
    page.locator(".check-item .check-conclusion", { hasText: "Success" }),
  ).toBeVisible();
  await expect(
    page.locator(".check-item .check-conclusion", { hasText: "Running" }),
  ).toBeVisible();
});

test("clicking CI check item opens CI log tab", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, prBranchResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "PR", exact: true })
    .click();

  await page.locator(".checks-section .checks-toggle").click();
  await page.locator(".check-item", { hasText: "CI Build" }).click();

  await expect(page.locator(".tab.active .tab-label")).toHaveText("CI #100");
});

test("PR tab shows review comments", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, prBranchResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "PR", exact: true })
    .click();

  await expect(page.getByText("reviewer-1").first()).toBeVisible();
});

test("PR tab shows mergeable status", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, prBranchResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "PR", exact: true })
    .click();

  await expect(page.getByText("Mergeable")).toBeVisible();
});

test("PR tab shows changed files count", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, prBranchResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "PR", exact: true })
    .click();

  // Should show additions/deletions info
  await expect(page.getByText("+12")).toBeVisible();
});

test("Summary > Git tab shows git section", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, prBranchResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Git", exact: true })
    .click();

  await expect(page.locator(".git-section")).toBeVisible();
});

test("Summary tab navigation works back and forth", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, prBranchResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  // Navigate to Git
  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Git", exact: true })
    .click();
  await expect(page.locator(".git-section")).toBeVisible();

  // Navigate to PR
  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "PR", exact: true })
    .click();
  await expect(page.locator(".pr-title")).toBeVisible();

  // Navigate back to Summary
  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Summary", exact: true })
    .click();
  // Summary tab should be active again
  await expect(
    page
      .locator(".summary-tabs")
      .getByRole("button", { name: "Summary", exact: true }),
  ).toHaveClass(/active/);
});

test("PR badge shows open class for MERGEABLE state", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {
        [branchFeature.name]: {
          number: 42,
          state: "OPEN",
          mergeable: "MERGEABLE",
          headBranch: branchFeature.name,
          retrying: false,
        },
      },
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);

  const prBadge = page.locator(".pr-badge", { hasText: "#42" });
  await expect(prBadge).toBeVisible();
  await expect(prBadge).toHaveClass(/open/);
});

test("PR badge shows checking class for UNKNOWN retrying state", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {
        [branchFeature.name]: {
          number: 42,
          state: "OPEN",
          mergeable: "UNKNOWN",
          headBranch: branchFeature.name,
          retrying: true,
        },
      },
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);

  const prBadge = page.locator(".pr-badge", { hasText: "#42" });
  await expect(prBadge).toBeVisible();
  await expect(prBadge).toHaveClass(/checking/);
});

test("pr-status-updated event updates badge", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {
        [branchFeature.name]: {
          number: 42,
          state: "OPEN",
          mergeable: "UNKNOWN",
          headBranch: branchFeature.name,
          retrying: true,
        },
      },
      ghStatus: { available: true, authenticated: true },
      repoKey: "/tmp/gwt-playwright",
    },
  });
  await openRecentProject(page);

  const prBadge = page.locator(".pr-badge", { hasText: "#42" });
  await expect(prBadge).toHaveClass(/pulse/);

  await waitForEventListener(page, "pr-status-updated");

  await emitTauriEvent(page, "pr-status-updated", {
    repoKey: "/tmp/gwt-playwright",
    branch: branchFeature.name,
    status: {
      number: 42,
      state: "OPEN",
      mergeable: "MERGEABLE",
      headBranch: branchFeature.name,
      retrying: false,
    },
  });

  await expect(prBadge).not.toHaveClass(/pulse/);
  await expect(prBadge).toHaveClass(/open/);
});

test("branch without PR shows no badge", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {
        [branchFeature.name]: null,
        [branchMain.name]: null,
        [branchDevelop.name]: null,
      },
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);

  await expect(page.locator(".pr-badge")).toBeHidden();
});

test("Summary > Summary tab shows AI Summary when available", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...prBranchResponses,
    get_branch_session_summary: {
      status: "ok",
      generating: false,
      toolId: "codex",
      sessionId: "session-1",
      markdown: "## AI Summary\n- workflow verified",
      bulletPoints: ["workflow verified"],
      error: null,
    },
  });
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Summary", exact: true })
    .click();

  await expect(page.getByText("AI Summary")).toBeVisible();
});

test("PR merged badge styling", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {
        [branchFeature.name]: {
          number: 42,
          state: "MERGED",
          mergeable: "UNKNOWN",
          headBranch: branchFeature.name,
          retrying: false,
        },
      },
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);

  const prBadge = page.locator(".pr-badge", { hasText: "#42" });
  await expect(prBadge).toBeVisible();
  await expect(prBadge).toHaveClass(/merged/);
});

test("PR closed badge styling", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {
        [branchFeature.name]: {
          number: 42,
          state: "CLOSED",
          mergeable: "UNKNOWN",
          headBranch: branchFeature.name,
          retrying: false,
        },
      },
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);

  const prBadge = page.locator(".pr-badge", { hasText: "#42" });
  await expect(prBadge).toBeVisible();
  await expect(prBadge).toHaveClass(/closed/);
});
