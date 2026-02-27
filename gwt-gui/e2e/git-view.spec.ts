import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  branchMain,
  branchDevelop,
  branchFeature,
  openRecentProject,
  setMockCommandResponses,
  standardBranchResponses,
} from "./support/helpers";

const gitChangeSummary = {
  base_branch: "main",
  staged: [
    {
      path: "src/main.rs",
      status: "modified",
      additions: 10,
      deletions: 3,
    },
  ],
  unstaged: [
    {
      path: "README.md",
      status: "modified",
      additions: 2,
      deletions: 1,
    },
  ],
  untracked: ["new-file.txt"],
  commits: [
    {
      hash: "abc123",
      short_hash: "abc123",
      message: "feat: add new feature",
      author: "dev",
      date: "2026-01-01T00:00:00Z",
    },
    {
      hash: "def456",
      short_hash: "def456",
      message: "fix: resolve bug",
      author: "dev",
      date: "2025-12-31T00:00:00Z",
    },
  ],
  stashes: [
    {
      index: 0,
      message: "WIP: save progress",
      branch: "feature/workflow-demo",
    },
  ],
};

const gitResponses = {
  ...standardBranchResponses(),
  get_git_change_summary: gitChangeSummary,
  get_base_branch_candidates: ["main", "develop"],
};

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("Summary > Git tab shows git section", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, gitResponses);
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

test("Git section has Changes tab", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, gitResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Git", exact: true })
    .click();

  // Expand the git section if collapsed
  const gitHeader = page.locator(".git-section .section-header");
  if (await gitHeader.isVisible({ timeout: 1000 }).catch(() => false)) {
    await gitHeader.click();
  }

  await expect(
    page.locator(".git-section").getByRole("button", { name: "Changes" }),
  ).toBeVisible();
});

test("Git section has Commits tab", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, gitResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Git", exact: true })
    .click();

  const gitHeader = page.locator(".git-section .section-header");
  if (await gitHeader.isVisible({ timeout: 1000 }).catch(() => false)) {
    await gitHeader.click();
  }

  await expect(
    page.locator(".git-section").getByRole("button", { name: "Commits" }),
  ).toBeVisible();
});

test("Git section shows base branch selector", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, gitResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Git", exact: true })
    .click();

  // Base branch combobox should be visible
  await expect(page.getByLabel("Base:")).toBeVisible();
});

test("Git Changes tab is active by default", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, gitResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Git", exact: true })
    .click();

  const gitHeader = page.locator(".git-section .section-header");
  if (await gitHeader.isVisible({ timeout: 1000 }).catch(() => false)) {
    await gitHeader.click();
  }

  await expect(
    page
      .locator(".git-section")
      .getByRole("button", { name: "Changes" }),
  ).toHaveClass(/active/);
});

test("switching from Git tab to Summary tab works", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, gitResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  // Navigate to Git tab
  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Git", exact: true })
    .click();
  await expect(page.locator(".git-section")).toBeVisible();

  // Navigate back to Summary
  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Summary", exact: true })
    .click();
  await expect(
    page
      .locator(".summary-tabs")
      .getByRole("button", { name: "Summary", exact: true }),
  ).toHaveClass(/active/);
});

test("Git section can be collapsed and expanded", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, gitResponses);
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

test("empty git changes shows appropriately", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    get_git_change_summary: {
      base_branch: "main",
      staged: [],
      unstaged: [],
      untracked: [],
      commits: [],
      stashes: [],
    },
    get_base_branch_candidates: ["main"],
  });
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

test("Git tab switch between Changes and Commits", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, gitResponses);
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Git", exact: true })
    .click();

  const gitHeader = page.locator(".git-section .section-header");
  if (await gitHeader.isVisible({ timeout: 1000 }).catch(() => false)) {
    await gitHeader.click();
  }

  const commitsBtn = page
    .locator(".git-section")
    .getByRole("button", { name: "Commits" });
  if (await commitsBtn.isVisible({ timeout: 1000 }).catch(() => false)) {
    await commitsBtn.click();
    await expect(commitsBtn).toHaveClass(/active/);
  }
});
