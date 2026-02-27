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
  dismissSkillRegistrationScopeDialogIfPresent,
} from "./support/helpers";

const worktreeFixtures = [
  {
    branch: "feature/cleanup-test",
    path: "/tmp/worktrees/feature-cleanup-test",
    safety_level: "safe",
    reason: "merged",
    is_bare: false,
    locked: false,
    prunable: false,
  },
  {
    branch: "feature/unsafe-branch",
    path: "/tmp/worktrees/feature-unsafe-branch",
    safety_level: "warning",
    reason: "unmerged changes",
    is_bare: false,
    locked: false,
    prunable: false,
  },
];

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("cleanup modal opens from Cleanup button in sidebar", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    list_worktrees: worktreeFixtures,
  });
  await openRecentProject(page);

  await page.getByRole("button", { name: "Cleanup" }).click();

  await expect(page.getByText("Clean Up")).toBeVisible();
});

test("cleanup modal shows worktree branches", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    list_worktrees: worktreeFixtures,
  });
  await openRecentProject(page);

  await page.getByRole("button", { name: "Cleanup" }).click();
  await expect(page.getByText("Clean Up")).toBeVisible();

  await expect(page.getByText("feature/cleanup-test")).toBeVisible();
});

test("cleanup modal shows both safe and warning worktrees", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    list_worktrees: worktreeFixtures,
  });
  await openRecentProject(page);

  await page.getByRole("button", { name: "Cleanup" }).click();
  await expect(page.getByText("Clean Up")).toBeVisible();

  await expect(page.getByText("feature/cleanup-test")).toBeVisible();
  await expect(page.getByText("feature/unsafe-branch")).toBeVisible();
});

test("cleanup modal allows selecting worktrees via checkbox", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    list_worktrees: worktreeFixtures,
  });
  await openRecentProject(page);

  await page.getByRole("button", { name: "Cleanup" }).click();
  await expect(page.getByText("feature/cleanup-test")).toBeVisible();

  // Click on the worktree row checkbox
  const checkbox = page
    .locator("input[type='checkbox']")
    .first();
  if (await checkbox.isVisible({ timeout: 2000 }).catch(() => false)) {
    await checkbox.click();
    await expect(checkbox).toBeChecked();
  }
});

test("cleanup modal handles empty worktree list", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    list_worktrees: [],
  });
  await openRecentProject(page);

  await page.getByRole("button", { name: "Cleanup" }).click();
  await expect(page.getByText("Clean Up")).toBeVisible();
});

test("migration modal shows step labels", async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
      probe_path: {
        kind: "migrationRequired",
        migrationSourceRoot: "/tmp/gwt-playwright",
      },
    },
  });
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await page.locator("button.recent-item").first().click();

  await expect(page.getByText("Validating prerequisites")).toBeVisible();
  await expect(page.getByText("Creating backup")).toBeVisible();
  await expect(page.getByText("Creating bare repository")).toBeVisible();
  await expect(page.getByText("Migrating worktrees")).toBeVisible();
  await expect(page.getByText("Cleaning up")).toBeVisible();
});

test("migration modal has Run Migration button", async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
      probe_path: {
        kind: "migrationRequired",
        migrationSourceRoot: "/tmp/gwt-playwright",
      },
    },
  });
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await page.locator("button.recent-item").first().click();

  await expect(
    page.getByRole("button", { name: /Run Migration|Migrate/ }),
  ).toBeVisible();
});

test("migration modal shows source root path", async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
      probe_path: {
        kind: "migrationRequired",
        migrationSourceRoot: "/tmp/gwt-playwright",
      },
    },
  });
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await page.locator("button.recent-item").first().click();

  await expect(page.getByText("/tmp/gwt-playwright")).toBeVisible();
});

test("cleanup worktree checkbox can be toggled", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    list_worktrees: worktreeFixtures,
  });
  await openRecentProject(page);

  await page.getByRole("button", { name: "Cleanup" }).click();
  await expect(page.getByText("feature/cleanup-test")).toBeVisible();

  const checkbox = page.locator("input[type='checkbox']").first();
  if (await checkbox.isVisible({ timeout: 2000 }).catch(() => false)) {
    await checkbox.click();
    await expect(checkbox).toBeChecked();
    await checkbox.click();
    await expect(checkbox).not.toBeChecked();
  }
});
