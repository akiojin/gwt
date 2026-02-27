import { expect, test, type Page } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";

const defaultRecentProject = {
  path: "/tmp/gwt-playwright",
  lastOpened: "2026-02-13T00:00:00.000Z",
};

const branchMain = {
  name: "main",
  commit: "aaa0000",
  is_current: true,
  ahead: 0,
  behind: 0,
  divergence_status: "UpToDate",
  last_tool_usage: null,
  is_agent_running: false,
  commit_timestamp: 1_700_000_100,
};

const branchFeature = {
  name: "feature/unknown-retry",
  commit: "bbb1111",
  is_current: false,
  ahead: 1,
  behind: 0,
  divergence_status: "Ahead",
  last_tool_usage: null,
  is_agent_running: false,
  commit_timestamp: 1_700_000_050,
};

const branchDevelop = {
  name: "develop",
  commit: "ccc2222",
  is_current: false,
  ahead: 0,
  behind: 0,
  divergence_status: "UpToDate",
  last_tool_usage: null,
  is_agent_running: false,
  commit_timestamp: 1_700_000_090,
};

/** PR status with retrying=true (UNKNOWN state, backend is retrying) */
const prStatusRetrying = {
  number: 99,
  state: "OPEN",
  mergeable: "UNKNOWN",
  headBranch: "feature/unknown-retry",
  retrying: true,
};

/** PR status with retrying=false (resolved to MERGEABLE) */
const prStatusResolved = {
  number: 99,
  state: "OPEN",
  mergeable: "MERGEABLE",
  headBranch: "feature/unknown-retry",
  retrying: false,
};

/** PR detail fixture for the Summary > PR tab */
const prDetailFixture = {
  number: 99,
  title: "Unknown Retry Test PR",
  state: "OPEN",
  url: "https://github.com/example/repo/pull/99",
  mergeable: "UNKNOWN",
  mergeStateStatus: "UNKNOWN",
  author: "e2e-user",
  baseBranch: "main",
  headBranch: "feature/unknown-retry",
  labels: [],
  assignees: [],
  milestone: null,
  linkedIssues: [],
  checkSuites: [],
  reviews: [],
  reviewComments: [],
  changedFilesCount: 1,
  additions: 5,
  deletions: 0,
};

async function setMockCommandResponses(
  page: Page,
  commandResponses: Record<string, unknown>,
) {
  await page.evaluate((responses) => {
    (window as unknown as {
      __GWT_MOCK_COMMAND_RESPONSES__?: Record<string, unknown>;
    }).__GWT_MOCK_COMMAND_RESPONSES__ = responses;
  }, commandResponses);
}

async function dismissSkillRegistrationScopeDialogIfPresent(page: Page) {
  const dialog = page.getByRole("dialog", {
    name: "Skill registration scope",
  });
  const visible = await dialog
    .isVisible({ timeout: 500 })
    .catch(() => false);
  if (!visible) return;
  await dialog.getByRole("button", { name: "Skip for now" }).click();
  await expect(dialog).toBeHidden();
}

async function openRecentProject(page: Page) {
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  const recentItem = page.locator("button.recent-item").first();
  await expect(recentItem).toBeVisible();
  await recentItem.click();
}

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("sidebar PR badge shows pulse animation when retrying=true", async ({
  page,
}) => {
  await page.goto("/");

  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {
        [branchFeature.name]: prStatusRetrying,
        [branchMain.name]: null,
        [branchDevelop.name]: null,
      },
      ghStatus: { available: true, authenticated: true },
      repoKey: "/tmp/gwt-playwright",
    },
    fetch_pr_detail: prDetailFixture,
    get_branch_session_summary: {
      status: "ok",
      generating: false,
      toolId: null,
      sessionId: null,
      markdown: "",
      bulletPoints: [],
      error: null,
    },
  });

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await openRecentProject(page);

  // Wait for the PR badge to appear in the sidebar
  const prBadge = page.locator(".pr-badge", { hasText: "#99" });
  await expect(prBadge).toBeVisible();

  // Verify pulse class is applied (retrying=true)
  await expect(prBadge).toHaveClass(/pulse/);
  // Verify checking styling class
  await expect(prBadge).toHaveClass(/checking/);
});

test("sidebar PR badge has no pulse when retrying=false", async ({ page }) => {
  await page.goto("/");

  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {
        [branchFeature.name]: prStatusResolved,
        [branchMain.name]: null,
        [branchDevelop.name]: null,
      },
      ghStatus: { available: true, authenticated: true },
      repoKey: "/tmp/gwt-playwright",
    },
  });

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await openRecentProject(page);

  const prBadge = page.locator(".pr-badge", { hasText: "#99" });
  await expect(prBadge).toBeVisible();

  // Verify pulse class is NOT applied (retrying=false)
  await expect(prBadge).not.toHaveClass(/pulse/);
  // Verify MERGEABLE styling class
  await expect(prBadge).toHaveClass(/open/);
});

test("pr-status-updated event resolves pulse animation to normal state", async ({
  page,
}) => {
  await page.goto("/");

  // Start with retrying=true (UNKNOWN state)
  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {
        [branchFeature.name]: prStatusRetrying,
        [branchMain.name]: null,
        [branchDevelop.name]: null,
      },
      ghStatus: { available: true, authenticated: true },
      repoKey: "/tmp/gwt-playwright",
    },
  });

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await openRecentProject(page);

  const prBadge = page.locator(".pr-badge", { hasText: "#99" });
  await expect(prBadge).toBeVisible();
  await expect(prBadge).toHaveClass(/pulse/);

  // Wait for event listeners to be registered
  await expect
    .poll(async () => {
      return page.evaluate(() => {
        const globalWindow = window as unknown as {
          __GWT_TAURI_INVOKE_LOG__?: Array<{
            cmd: string;
            args?: { event?: string };
          }>;
        };
        return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).some(
          (entry) =>
            entry.cmd === "plugin:event|listen" &&
            entry.args?.event === "pr-status-updated",
        );
      });
    })
    .toBe(true);

  // Emit pr-status-updated event with resolved status (retrying=false)
  await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_MOCK_EMIT_EVENT__?: (event: string, payload: unknown) => void;
    };
    globalWindow.__GWT_MOCK_EMIT_EVENT__?.("pr-status-updated", {
      repoKey: "/tmp/gwt-playwright",
      branch: "feature/unknown-retry",
      status: {
        number: 99,
        state: "OPEN",
        mergeable: "MERGEABLE",
        headBranch: "feature/unknown-retry",
        retrying: false,
      },
    });
  });

  // PR badge should now show resolved state without pulse
  await expect(prBadge).not.toHaveClass(/pulse/);
  await expect(prBadge).toHaveClass(/open/);
});

test("pr-status-updated event with retrying=true is ignored", async ({
  page,
}) => {
  await page.goto("/");

  // Start with retrying=true (UNKNOWN state)
  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {
        [branchFeature.name]: prStatusRetrying,
        [branchMain.name]: null,
        [branchDevelop.name]: null,
      },
      ghStatus: { available: true, authenticated: true },
      repoKey: "/tmp/gwt-playwright",
    },
  });

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await openRecentProject(page);

  const prBadge = page.locator(".pr-badge", { hasText: "#99" });
  await expect(prBadge).toBeVisible();
  await expect(prBadge).toHaveClass(/pulse/);

  // Wait for event listeners
  await expect
    .poll(async () => {
      return page.evaluate(() => {
        const globalWindow = window as unknown as {
          __GWT_TAURI_INVOKE_LOG__?: Array<{
            cmd: string;
            args?: { event?: string };
          }>;
        };
        return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).some(
          (entry) =>
            entry.cmd === "plugin:event|listen" &&
            entry.args?.event === "pr-status-updated",
        );
      });
    })
    .toBe(true);

  // Emit event with retrying=true — should be ignored
  await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_MOCK_EMIT_EVENT__?: (event: string, payload: unknown) => void;
    };
    globalWindow.__GWT_MOCK_EMIT_EVENT__?.("pr-status-updated", {
      repoKey: "/tmp/gwt-playwright",
      branch: "feature/unknown-retry",
      status: {
        number: 99,
        state: "OPEN",
        mergeable: "MERGEABLE",
        headBranch: "feature/unknown-retry",
        retrying: true,
      },
    });
  });

  // Wait a moment to ensure event processing has occurred
  await page.waitForTimeout(200);

  // Badge should still show pulse (retrying=true event was ignored)
  await expect(prBadge).toHaveClass(/pulse/);
  await expect(prBadge).toHaveClass(/checking/);
});
