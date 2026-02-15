import { expect, test, type Page } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";

const defaultRecentProject = {
  path: "/tmp/gwt-playwright",
  lastOpened: "2026-02-13T00:00:00.000Z",
};

const branchA = {
  name: "feature/workflow-demo",
  commit: "aaa1111",
  is_current: false,
  ahead: 1,
  behind: 0,
  divergence_status: "Ahead",
  last_tool_usage: null,
  is_agent_running: false,
  commit_timestamp: 1_700_000_012,
};
const branchB = {
  name: "feature/ui-polish",
  commit: "bbb2222",
  is_current: false,
  ahead: 0,
  behind: 1,
  divergence_status: "Behind",
  last_tool_usage: null,
  is_agent_running: false,
  commit_timestamp: 1_700_000_020,
};
const branchMain = {
  ...branchA,
  name: "main",
  is_current: true,
  is_agent_running: false,
  divergence_status: "UpToDate",
  commit_timestamp: 1_700_000_100,
};
const branchDevelop = {
  ...branchA,
  name: "develop",
  is_current: false,
  ahead: 0,
  behind: 0,
  divergence_status: "UpToDate",
  commit_timestamp: 1_700_000_090,
};

const prStatusFixture = {
  number: 42,
  title: "Workflow Demo PR",
  state: "OPEN",
  url: "https://github.com/example/workflow-demo/pull/42",
  mergeable: "MERGEABLE",
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

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("launches and completes open-project -> agent send smoke flow", async ({
  page,
}) => {
  await page.goto("/");

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  const recentItem = page.locator("button.recent-item").first();
  await expect(recentItem).toBeVisible();

  await recentItem.click();

  const prompt = page.getByPlaceholder("Type a task and press Enter...");
  await expect(prompt).toBeVisible();

  const message = "smoke message";
  await prompt.fill(message);
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText(`Echo: ${message}`)).toBeVisible();

  const invokeCommands = await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
    };
    return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).map(
      (entry) => entry.cmd,
    );
  });

  expect(invokeCommands).toContain("open_project");
  expect(invokeCommands).toContain("send_agent_mode_message");
});

test("shows terminal stream error and closes errored terminal tab on Enter", async ({
  page,
}) => {
  await page.goto("/");

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await page.locator("button.recent-item").first().click();
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  await expect
    .poll(async () => {
      return page.evaluate(() => {
        const raw = window.localStorage.getItem("gwt.projectTabs.v2");
        if (!raw) return false;
        try {
          const parsed = JSON.parse(raw) as {
            byProjectPath?: Record<string, { activeTabId?: string | null }>;
          };
          return (
            parsed.byProjectPath?.["/tmp/gwt-playwright"]?.activeTabId ===
            "agentMode"
          );
        } catch {
          return false;
        }
      });
    })
    .toBe(true);

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

  await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_MOCK_SET_NEXT_SPAWN_ERROR__?: (enabled: boolean) => void;
      __GWT_MOCK_EMIT_EVENT__?: (event: string, payload: unknown) => void;
    };
    globalWindow.__GWT_MOCK_SET_NEXT_SPAWN_ERROR__?.(true);
    globalWindow.__GWT_MOCK_EMIT_EVENT__?.("menu-action", {
      action: "new-terminal",
    });
  });

  await expect
    .poll(async () => {
      return page.evaluate(() => {
        const globalWindow = window as unknown as {
          __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
        };
        return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).some(
          (entry) => entry.cmd === "spawn_shell",
        );
      });
    })
    .toBe(true);

  const terminalState = await page.evaluate(async () => {
    const globalWindow = window as unknown as {
      __TAURI_INTERNALS__?: {
        invoke: (
          cmd: string,
          args?: unknown,
          options?: unknown,
        ) => Promise<unknown>;
      };
      __GWT_MOCK_LAST_SPAWNED_PANE_ID__?: () => string | null;
    };

    const paneId = globalWindow.__GWT_MOCK_LAST_SPAWNED_PANE_ID__?.() ?? "";
    if (!paneId || !globalWindow.__TAURI_INTERNALS__) {
      return {
        paneId,
        scrollback: "",
        terminalsAfterClose: [] as Array<{ pane_id: string }>,
      };
    }

    const scrollback = (await globalWindow.__TAURI_INTERNALS__.invoke(
      "capture_scrollback_tail",
      {
        paneId,
        maxBytes: 64 * 1024,
      },
    )) as string;

    await globalWindow.__TAURI_INTERNALS__.invoke("write_terminal", {
      paneId,
      data: [13],
    });

    const terminalsAfterClose = (await globalWindow.__TAURI_INTERNALS__.invoke(
      "list_terminals",
      {},
    )) as Array<{ pane_id: string }>;

    return { paneId, scrollback, terminalsAfterClose };
  });

  expect(terminalState.paneId.length).toBeGreaterThan(0);
  expect(terminalState.scrollback).toContain(
    "PTY stream error: mocked read failure",
  );
  expect(terminalState.scrollback).toContain("Press Enter to close this tab.");
  expect(
    terminalState.terminalsAfterClose.some(
      (term) => term.pane_id === terminalState.paneId,
    ),
  ).toBe(false);

  const invokeCommands = await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
    };
    return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).map(
      (entry) => entry.cmd,
    );
  });

  expect(invokeCommands).toContain("spawn_shell");
  expect(invokeCommands).toContain("write_terminal");
  expect(invokeCommands).toContain("capture_scrollback_tail");
});

test("navigates Session Summary tabs and opens workflow run page", async ({
  page,
}) => {
  await page.goto("/");

  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchA, branchB],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {
        [branchA.name]: { number: 42 },
        [branchB.name]: null,
        [branchMain.name]: null,
        [branchDevelop.name]: null,
      },
      ghStatus: { available: true, authenticated: true },
    },
    fetch_pr_detail: prStatusFixture,
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

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await page.locator("button.recent-item").first().click();

  const branchButton = page
    .locator(".branch-item")
    .filter({ hasText: branchA.name });
  await expect(branchButton).toBeVisible();
  await branchButton.click();

  await expect(
    page.getByRole("heading", { level: 2, name: branchA.name }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Summary" })).toHaveClass(
    /active/,
  );

  await page.getByRole("button", { name: "Git" }).click();
  await expect(page.locator(".git-section")).toBeVisible();

  await page.getByRole("button", { name: "PR" }).click();
  await expect(page.locator(".pr-title")).toBeVisible();

  await page.getByRole("button", { name: "Workflow" }).click();
  await expect(page.locator(".workflow-status-text", { hasText: "Success" })).toBeVisible();
  await expect(page.locator(".workflow-status-text", { hasText: "Running" })).toBeVisible();
  await expect(page.getByText("CI Build")).toBeHidden();

  await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_MOCK_OPEN_URLS__?: string[];
    };
    globalWindow.__GWT_MOCK_OPEN_URLS__ = [];
    window.open = ((url: string | URL | null) => {
      if (typeof url === "string") {
        globalWindow.__GWT_MOCK_OPEN_URLS__?.push(url);
      }
      return null;
    }) as Window["open"];
  });

  await page.locator(".workflow-status-text", { hasText: "Success" }).click();
  await expect
    .poll(async () => {
      const globalWindow = window as unknown as {
        __GWT_MOCK_OPEN_URLS__?: string[];
      };
      return globalWindow.__GWT_MOCK_OPEN_URLS__?.includes(
        "https://github.com/example/workflow-demo/actions/runs/100",
      );
    })
    .toBe(true);

  await page.getByRole("button", { name: "AI" }).click();
  await expect(page.getByText("AI Summary")).toBeVisible();
});

test("switches sort mode on worktree list", async ({ page }) => {
  await page.goto("/");

  await setMockCommandResponses(page, {
    list_worktree_branches: [
      { ...branchA, name: "feature/name-a", commit_timestamp: 1_700_000_050 },
      { ...branchDevelop, name: "feature/name-c", commit_timestamp: 1_700_000_100 },
      { ...branchB, name: "feature/name-b", commit_timestamp: 1_700_000_200 },
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

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await page.locator("button.recent-item").first().click();

  const sortText = page.locator(".sort-mode-text");
  await expect(sortText).toHaveText("Name");
  await expect(page.locator(".branch-list .branch-name").nth(0)).toHaveText("develop");
  await expect(page.locator(".branch-list .branch-name").nth(1)).toHaveText("main");
  await expect(page.locator(".branch-list .branch-name").nth(2)).toHaveText(
    "feature/name-a",
  );

  await page.locator(".sort-mode-toggle").click();
  await expect(sortText).toHaveText("Updated");
  await expect(page.locator(".branch-list .branch-name").nth(2)).toHaveText(
    "feature/name-b",
  );
});
