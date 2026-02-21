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

async function dismissSkillRegistrationScopeDialogIfPresent(page: Page) {
  const dialog = page.getByRole("dialog", {
    name: "Skill registration scope",
  });
  const visible = await dialog
    .isVisible({ timeout: 500 })
    .catch(() => false);
  if (!visible) {
    return;
  }

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

test("launches and completes open-project -> project mode send smoke flow", async ({
  page,
}) => {
  await page.goto("/");

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await openRecentProject(page);

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
  expect(invokeCommands).toContain("send_project_mode_message_cmd");
});

test("launches agent from Launch Agent dialog and opens agent terminal tab", async ({
  page,
}) => {
  await page.goto("/");

  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchA],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {},
      ghStatus: { available: true, authenticated: true },
    },
    detect_agents: [
      {
        id: "codex",
        name: "Codex",
        version: "0.99.0",
        authenticated: true,
        available: true,
      },
    ],
    list_agent_versions: {
      agentId: "codex",
      package: "codex",
      tags: ["latest"],
      versions: ["0.99.0"],
      source: "cache",
    },
  });

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await openRecentProject(page);

  const branchButton = page
    .locator(".branch-item")
    .filter({ hasText: branchA.name });
  await expect(branchButton).toBeVisible();
  await branchButton.click();

  const launchAgentButton = page
    .locator(".worktree-summary-panel")
    .getByRole("button", { name: "Launch Agent..." });
  await expect(launchAgentButton).toBeVisible();
  await launchAgentButton.click();

  const launchDialog = page.getByRole("dialog", { name: "Launch Agent" });
  await expect(launchDialog).toBeVisible();
  await expect(launchDialog.locator("select#agent-select")).toHaveValue("codex");

  await launchDialog.getByRole("button", { name: "Launch", exact: true }).click();

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
          (entry) => entry.cmd === "start_launch_job",
        );
      });
    })
    .toBe(true);

  await expect(page.locator(".tab.active .tab-label")).toHaveText(branchA.name);
  const activeTerminalContainer = page.locator(
    ".terminal-wrapper.active .terminal-container",
  );
  await expect(activeTerminalContainer).toBeVisible();
  await expect(activeTerminalContainer.locator(".xterm")).toBeVisible();

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
            entry.args?.event === "terminal-output",
        );
      });
    })
    .toBe(true);

  const paneId = await activeTerminalContainer.getAttribute("data-pane-id");
  expect(paneId).toBeTruthy();

  const readTerminalState = async () =>
    page.evaluate(({ targetPaneId }) => {
      const container = document.querySelector(
        `.terminal-wrapper.active .terminal-container[data-pane-id="${targetPaneId}"]`,
      ) as
        | (HTMLElement & {
            __gwtTerminal?: {
              rows?: number;
              cols?: number;
              buffer?: {
                active?: {
                  cursorX?: number;
                  cursorY?: number;
                  baseY?: number;
                  length?: number;
                };
              };
            };
          })
        | null;
      if (!container) {
        return null;
      }
      const terminal = container.__gwtTerminal;
      const activeBuffer = terminal?.buffer?.active;
      if (!terminal || !activeBuffer) {
        return null;
      }
      return {
        rows: terminal.rows ?? -1,
        cols: terminal.cols ?? -1,
        cursorX: activeBuffer.cursorX ?? -1,
        cursorY: activeBuffer.cursorY ?? -1,
        baseY: activeBuffer.baseY ?? -1,
        length: activeBuffer.length ?? -1,
      };
    }, { targetPaneId: paneId });

  await expect.poll(readTerminalState).not.toBeNull();
  const beforeState = await readTerminalState();
  expect(beforeState).not.toBeNull();

  await page.evaluate(
    ({ targetPaneId }) => {
      const globalWindow = window as unknown as {
        __GWT_MOCK_EMIT_EVENT__?: (event: string, payload: unknown) => void;
      };
      const bytes = [69, 50, 69, 45, 76, 65, 85, 78, 67, 72, 45, 79, 85, 84, 13, 10];
      globalWindow.__GWT_MOCK_EMIT_EVENT__?.("terminal-output", {
        pane_id: targetPaneId,
        data: bytes,
      });
    },
    { targetPaneId: paneId },
  );

  await expect
    .poll(async () => {
      const afterState = await readTerminalState();
      if (!beforeState || !afterState) return false;
      return (
        afterState.cursorX !== beforeState.cursorX ||
        afterState.cursorY !== beforeState.cursorY ||
        afterState.baseY !== beforeState.baseY ||
        afterState.length !== beforeState.length
      );
    })
    .toBe(true);
});

test("shows terminal stream error and closes errored terminal tab on Enter", async ({
  page,
}) => {
  await page.goto("/");

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await openRecentProject(page);
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
            "projectMode"
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
test("navigates Summary tabs and opens workflow run page from PR checks", async ({
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
  await openRecentProject(page);

  const branchButton = page
    .locator(".branch-item")
    .filter({ hasText: branchA.name });
  await expect(branchButton).toBeVisible();
  await branchButton.click();

  await expect(
    page.getByRole("button", { name: "Summary", exact: true }),
  ).toHaveClass(/active/);

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Git", exact: true })
    .click();
  await expect(page.locator(".git-section")).toBeVisible();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "PR", exact: true })
    .click();
  await expect(page.locator(".pr-title")).toBeVisible();

  const checksToggle = page.locator(".checks-section .checks-toggle");
  await expect(checksToggle).toBeVisible();
  await checksToggle.click();
  await expect(page.locator(".check-item .check-name", { hasText: "CI Build" })).toBeVisible();
  await expect(page.locator(".check-item .check-conclusion", { hasText: "Success" })).toBeVisible();
  await expect(page.locator(".check-item .check-conclusion", { hasText: "Running" })).toBeVisible();

  await page.locator(".check-item", { hasText: "CI Build" }).click();
  await expect(page.locator(".tab.active .tab-label")).toHaveText("CI #100");

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Summary", exact: true })
    .click();
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
  await openRecentProject(page);

  const sortText = page.locator(".sort-mode-text");
  await expect(sortText).toHaveText("Updated");
  await expect(page.locator(".branch-list .branch-name").nth(0)).toHaveText("main");
  await expect(page.locator(".branch-list .branch-name").nth(1)).toHaveText("develop");
  await expect(page.locator(".branch-list .branch-name").nth(2)).toHaveText(
    "feature/name-b",
  );

  await page.locator(".sort-mode-toggle").click();
  await expect(sortText).toHaveText("Name");
  await expect(page.locator(".branch-list .branch-name").nth(2)).toHaveText(
    "feature/name-a",
  );
});

test("restores saved window sessions on startup", async ({ page }) => {
  const savedSessions = [
    { label: "main", projectPath: "/tmp/project-main" },
    { label: "project-2", projectPath: "/tmp/project-second" },
  ];

  await page.addInitScript((sessionsJson) => {
    window.localStorage.setItem(
      "gwt.windowSessions.v1",
      JSON.stringify(sessionsJson),
    );
  }, savedSessions);

  await page.goto("/");

  await expect
    .poll(async () => {
      const raw = await page.evaluate(() => {
        const raw = (window as unknown as {
          __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
        }).__GWT_TAURI_INVOKE_LOG__;
        return raw;
      });
      const entries = Array.isArray(raw) ? raw : [];
      return entries.some((entry) => entry.cmd === "open_project");
    })
    .toBe(true);

  const invokeCommands = await page.evaluate(() => {
    const raw = (window as unknown as { __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }> })
      .__GWT_TAURI_INVOKE_LOG__;
    return raw ?? [];
  });

  expect(invokeCommands.map((entry) => entry.cmd)).toContain("open_gwt_window");
});
