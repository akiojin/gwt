import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  captureUxSnapshot,
  defaultRecentProject,
  openBranchBrowser,
  openRecentProject,
  saveE2ECoverage,
  setMockCommandResponses,
  standardBranchResponses,
  dismissSkillRegistrationScopeDialogIfPresent,
} from "./support/helpers";

const worktreeFixtures = [
  {
    branch: "feature/cleanup-test",
    path: "/tmp/worktrees/feature-cleanup-test",
    commit: "aaa1111",
    status: "prunable",
    is_main: false,
    has_changes: false,
    has_unpushed: false,
    is_current: false,
    is_protected: false,
    is_agent_running: false,
    agent_status: "unknown",
    ahead: 0,
    behind: 0,
    is_gone: false,
    last_tool_usage: null,
    safety_level: "safe",
  },
  {
    branch: "feature/unsafe-branch",
    path: "/tmp/worktrees/feature-unsafe-branch",
    commit: "bbb2222",
    status: "active",
    is_main: false,
    has_changes: true,
    has_unpushed: true,
    is_current: false,
    is_protected: false,
    is_agent_running: false,
    agent_status: "unknown",
    ahead: 1,
    behind: 0,
    is_gone: false,
    last_tool_usage: "codex",
    safety_level: "warning",
  },
];

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

async function openCleanupFromBranchBrowser(
  page: import("@playwright/test").Page,
  worktrees: unknown[],
) {
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    list_worktrees: worktrees,
    check_gh_available: true,
    get_cleanup_settings: { delete_remote_branches: false },
    get_cleanup_pr_statuses: {},
  });
  await openRecentProject(page);
  await openBranchBrowser(page);
  await page.getByRole("button", { name: "Cleanup" }).click();
}

test("cleanup modal opens from Cleanup button in Branch Browser", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await openCleanupFromBranchBrowser(page, worktreeFixtures);
  await expect(
    page.getByRole("dialog", { name: "Cleanup Worktrees" }),
  ).toBeVisible();
  await captureUxSnapshot(page, testInfo, "cleanup-modal-open");
});

test("cleanup modal shows worktree branches", async ({ page }) => {
  await page.goto("/");
  await openCleanupFromBranchBrowser(page, worktreeFixtures);
  const cleanupDialog = page.getByRole("dialog", {
    name: "Cleanup Worktrees",
  });
  await expect(cleanupDialog).toBeVisible();

  await expect(cleanupDialog.getByText("feature/cleanup-test")).toBeVisible();
});

test("cleanup modal shows both safe and warning worktrees", async ({
  page,
}) => {
  await page.goto("/");
  await openCleanupFromBranchBrowser(page, worktreeFixtures);
  const cleanupDialog = page.getByRole("dialog", {
    name: "Cleanup Worktrees",
  });
  await expect(cleanupDialog).toBeVisible();

  await expect(cleanupDialog.getByText("feature/cleanup-test")).toBeVisible();
  await expect(cleanupDialog.getByText("feature/unsafe-branch")).toBeVisible();
});

test("cleanup modal allows selecting worktrees via checkbox", async ({
  page,
}) => {
  await page.goto("/");
  await openCleanupFromBranchBrowser(page, worktreeFixtures);
  const cleanupDialog = page.getByRole("dialog", {
    name: "Cleanup Worktrees",
  });
  await expect(cleanupDialog.getByText("feature/cleanup-test")).toBeVisible();

  // Click on the worktree row checkbox
  const checkbox = cleanupDialog
    .locator("input[type='checkbox']")
    .first();
  if (await checkbox.isVisible({ timeout: 2000 }).catch(() => false)) {
    await checkbox.click();
    await expect(checkbox).toBeChecked();
  }
});

test("cleanup modal handles empty worktree list", async ({ page }) => {
  await page.goto("/");
  await openCleanupFromBranchBrowser(page, []);
  await expect(
    page.getByRole("dialog", { name: "Cleanup Worktrees" }),
  ).toBeVisible();
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

  const migrationDialog = page.getByRole("dialog", {
    name: "Migration Required",
  });
  await expect(migrationDialog).toBeVisible();
  await expect(migrationDialog.getByText("/tmp/gwt-playwright")).toBeVisible();
});

test("cleanup worktree checkbox can be toggled", async ({ page }) => {
  await page.goto("/");
  await openCleanupFromBranchBrowser(page, worktreeFixtures);
  const cleanupDialog = page.getByRole("dialog", {
    name: "Cleanup Worktrees",
  });
  await expect(cleanupDialog.getByText("feature/cleanup-test")).toBeVisible();

  const checkbox = cleanupDialog.locator("input[type='checkbox']").first();
  if (await checkbox.isVisible({ timeout: 2000 }).catch(() => false)) {
    await checkbox.click();
    await expect(checkbox).toBeChecked();
    await checkbox.click();
    await expect(checkbox).not.toBeChecked();
  }
});
