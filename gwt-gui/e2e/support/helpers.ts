import type { Page, TestInfo } from "@playwright/test";
import { expect } from "@playwright/test";

// ── Common branch fixtures ──

export const branchMain = {
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

export const branchDevelop = {
  ...branchMain,
  name: "develop",
  is_current: false,
  commit_timestamp: 1_700_000_090,
};

export const branchFeature = {
  name: "feature/workflow-demo",
  commit: "bbb1111",
  is_current: false,
  ahead: 1,
  behind: 0,
  divergence_status: "Ahead",
  last_tool_usage: null,
  is_agent_running: false,
  commit_timestamp: 1_700_000_050,
};

export const branchBehind = {
  ...branchFeature,
  name: "feature/behind-branch",
  commit: "ccc3333",
  ahead: 0,
  behind: 2,
  divergence_status: "Behind",
  commit_timestamp: 1_700_000_030,
};

export const defaultRecentProject = {
  path: "/tmp/gwt-playwright",
  lastOpened: "2026-02-13T00:00:00.000Z",
};

export const prStatusFixture = {
  number: 42,
  title: "Workflow Demo PR",
  state: "OPEN",
  url: "https://github.com/example/workflow-demo/pull/42",
  mergeable: "MERGEABLE",
  mergeStateStatus: "CLEAN",
  author: "e2e",
  baseBranch: "main",
  headBranch: "feature/workflow-demo",
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
  reviewComments: [
    {
      author: "reviewer-2",
      body: "Looks good",
      filePath: "README.md",
      line: 4,
      codeSnippet: "foo",
    },
  ],
  changedFilesCount: 2,
  additions: 12,
  deletions: 3,
};

export const settingsFixture = {
  protected_branches: ["main", "develop"],
  default_base_branch: "main",
  worktree_root: "/tmp/worktrees",
  debug: false,
  log_retention_days: 30,
  agent_default: "codex",
  agent_auto_install_deps: true,
  docker_force_host: true,
  ui_font_size: 13,
  terminal_font_size: 13,
  ui_font_family:
    'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
  terminal_font_family:
    '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace',
  app_language: "auto",
  voice_input: {
    enabled: false,
    engine: "qwen3-asr",
    language: "auto",
    quality: "balanced",
    model: "Qwen/Qwen3-ASR-1.7B",
  },
  default_shell: null,
};

export const profilesFixture = {
  version: 1,
  active: "default",
  profiles: {
    default: {
      name: "default",
      description: "",
      env: {},
      disabled_env: [],
      ai_enabled: false,
      ai: null,
    },
  },
};

export const detectedAgents = [
  {
    id: "codex",
    name: "Codex",
    version: "0.99.0",
    authenticated: true,
    available: true,
  },
];

// ── Common operations ──

export async function setMockCommandResponses(
  page: Page,
  commandResponses: Record<string, unknown>,
): Promise<void> {
  await page.evaluate((responses) => {
    (
      window as unknown as {
        __GWT_MOCK_COMMAND_RESPONSES__?: Record<string, unknown>;
      }
    ).__GWT_MOCK_COMMAND_RESPONSES__ = responses;
  }, commandResponses);
}

/**
 * No-op: the startup skill registration scope dialog has been removed.
 * This function is kept for backward compatibility with existing E2E tests.
 */
export async function dismissSkillRegistrationScopeDialogIfPresent(
  _page: Page,
): Promise<void> {
  // Dialog no longer exists after scope simplification.
}

export async function openRecentProject(page: Page): Promise<void> {
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  const recentItem = page.locator("button.recent-item").first();
  await expect(recentItem).toBeVisible();
  await recentItem.click();
}

export async function waitForMenuActionListener(page: Page): Promise<void> {
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
            entry.args?.event === "menu-action",
        );
      });
    })
    .toBe(true);
}

export async function emitTauriEvent(
  page: Page,
  event: string,
  payload: unknown,
): Promise<void> {
  await page.evaluate(
    ({ event, payload }) => {
      const globalWindow = window as unknown as {
        __GWT_MOCK_EMIT_EVENT__?: (event: string, payload: unknown) => void;
      };
      globalWindow.__GWT_MOCK_EMIT_EVENT__?.(event, payload);
    },
    { event, payload },
  );
}

export async function getInvokeLog(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
    };
    return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).map(
      (entry) => entry.cmd,
    );
  });
}

export async function waitForInvokeCommand(
  page: Page,
  cmd: string,
): Promise<void> {
  await expect
    .poll(async () => {
      return page.evaluate((targetCmd) => {
        const globalWindow = window as unknown as {
          __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
        };
        return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).some(
          (entry) => entry.cmd === targetCmd,
        );
      }, cmd);
    })
    .toBe(true);
}

export async function waitForEventListener(
  page: Page,
  eventName: string,
): Promise<void> {
  await expect
    .poll(async () => {
      return page.evaluate((targetEvent) => {
        const globalWindow = window as unknown as {
          __GWT_TAURI_INVOKE_LOG__?: Array<{
            cmd: string;
            args?: { event?: string };
          }>;
        };
        return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).some(
          (entry) =>
            entry.cmd === "plugin:event|listen" &&
            entry.args?.event === targetEvent,
        );
      }, eventName);
    })
    .toBe(true);
}

export async function openProjectAndSelectBranch(
  page: Page,
  branchName: string,
  commandResponses: Record<string, unknown>,
): Promise<void> {
  await setMockCommandResponses(page, commandResponses);

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await openRecentProject(page);

  await page.locator('[data-tab-id="branchBrowser"]').click();
  const visibleBrowser = page.locator('[data-testid="branch-browser-panel"]:visible');
  await expect(visibleBrowser).toBeVisible();

  const branchButton = page
    .locator(".branch-row")
    .filter({ hasText: branchName });
  await expect(branchButton).toBeVisible();
  await branchButton.click();

  await expect(page.getByTestId("branch-browser-detail")).toContainText(
    branchName,
  );
}

export async function openBranchBrowser(page: Page): Promise<void> {
  await page.locator('[data-tab-id="branchBrowser"]').click();
  await expect(
    page.locator('[data-testid="branch-browser-panel"]:visible'),
  ).toBeVisible();
}

export async function expectAgentCanvasVisible(page: Page): Promise<void> {
  await expect(page.getByRole("heading", { name: "Agent Canvas" })).toBeVisible();
  await expect(page.getByTestId("agent-canvas-board")).toBeVisible();
}

export async function captureUxSnapshot(
  page: Page,
  testInfo: TestInfo,
  name: string,
): Promise<void> {
  const safeName = name.replace(/[^a-zA-Z0-9_-]+/g, "-");
  await page.screenshot({
    path: testInfo.outputPath(`${safeName}.png`),
    fullPage: true,
  });
}

export async function selectBranchInBrowser(
  page: Page,
  branchName: string,
): Promise<void> {
  await openBranchBrowser(page);
  const branchButton = page
    .locator(".branch-row")
    .filter({ hasText: branchName });
  await expect(branchButton).toBeVisible();
  await branchButton.click();
  await expect(page.getByTestId("branch-browser-detail")).toContainText(
    branchName,
  );
}

export async function openSettings(
  page: Page,
  commandResponses: Record<string, unknown>,
): Promise<void> {
  await openProjectAndSelectBranch(page, branchFeature.name, commandResponses);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "open-settings" });

  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
}

/** Standard command responses for branch-list scenarios */
export function standardBranchResponses(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {},
      ghStatus: { available: true, authenticated: true },
    },
    ...overrides,
  };
}

/** Standard command responses for settings scenarios */
export function standardSettingsResponses(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    ...standardBranchResponses(),
    get_settings: settingsFixture,
    get_profiles: profilesFixture,
    ...overrides,
  };
}

export function getInvokeArgs(page: Page, cmd: string) {
  return page.evaluate((targetCmd) => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{
        cmd: string;
        args?: Record<string, unknown>;
      }>;
    };
    const log = globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? [];
    const entry = [...log].reverse().find((item) => item.cmd === targetCmd);
    return entry?.args ?? null;
  }, cmd);
}
