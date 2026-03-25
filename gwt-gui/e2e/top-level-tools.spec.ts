import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  captureUxSnapshot,
  detectedAgents,
  defaultRecentProject,
  emitTauriEvent,
  expectAgentCanvasVisible,
  openBranchBrowser,
  openRecentProject,
  saveE2ECoverage,
  setMockCommandResponses,
  settingsFixture,
  standardBranchResponses,
  waitForEventListener,
  waitForMenuActionListener,
  waitForInvokeCommand,
} from "./support/helpers";

const versionListFixture = {
  items: [
    {
      id: "v1.2.0",
      label: "v1.2.0",
      range_from: "v1.1.0",
      range_to: "v1.2.0",
      commit_count: 7,
    },
    {
      id: "v1.1.0",
      label: "v1.1.0",
      range_from: "v1.0.0",
      range_to: "v1.1.0",
      commit_count: 4,
    },
  ],
};

const versionHistoryFixture = {
  status: "ok",
  version_id: "v1.2.0",
  label: "v1.2.0",
  range_to: "v1.2.0",
  range_from: "v1.1.0",
  commit_count: 7,
  summary_markdown: "## Summary\n\n- current shell stabilized",
  changelog_markdown: "## Changelog\n\n- Added current shell coverage",
  error: null,
};

const projectIndexStatusFixture = {
  indexed: true,
  totalFiles: 321,
  dbSizeBytes: 40960,
};

const projectIndexResultsFixture = [
  {
    path: "src/App.svelte",
    distance: 0.07,
    description: "Current shell workspace root",
  },
  {
    path: "src/lib/components/BranchBrowserPanel.svelte",
    distance: 0.12,
    description: "Top-level branch browser surface",
  },
];

const issuesFixture = {
  issues: [
    {
      number: 101,
      title: "Fix login flow",
      body: "Current shell issue body",
      state: "open",
      updatedAt: "2026-03-20T10:00:00.000Z",
      htmlUrl: "https://github.com/example/gwt/issues/101",
      labels: [{ name: "bug", color: "d73a4a" }],
      assignees: [],
      commentsCount: 2,
    },
  ],
  hasNextPage: false,
};

const prListFixture = {
  items: [
    {
      number: 42,
      title: "Workflow Demo PR",
      state: "OPEN",
      isDraft: false,
      headRefName: "feature/workflow-demo",
      baseRefName: "main",
      author: { login: "e2e" },
      labels: [{ name: "bugfix", color: "d73a4a" }],
      createdAt: "2026-03-19T10:00:00.000Z",
      updatedAt: "2026-03-22T10:00:00.000Z",
      url: "https://github.com/example/gwt/pull/42",
      body: "PR body",
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
  headBranch: "feature/workflow-demo",
  labels: ["bugfix"],
  assignees: [],
  milestone: null,
  linkedIssues: [101],
  checkSuites: [],
  reviews: [],
  reviewComments: [],
  changedFilesCount: 2,
  additions: 12,
  deletions: 3,
};

const cleanupRowsFixture = [
  {
    branch: "feature/stale-worktree",
    path: "/tmp/worktrees/feature-stale-worktree",
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
    safety_level: "safe",
    last_tool_usage: null,
  },
  {
    branch: "feature/warning-worktree",
    path: "/tmp/worktrees/feature-warning-worktree",
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
    safety_level: "warning",
    last_tool_usage: "codex",
  },
];

function toolResponses() {
  return {
    ...standardBranchResponses(),
    list_project_versions: versionListFixture,
    get_project_version_history: versionHistoryFixture,
    get_index_status_cmd: projectIndexStatusFixture,
    search_project_index_cmd: projectIndexResultsFixture,
    check_gh_cli_status: { available: true, authenticated: true },
    fetch_github_issues: issuesFixture,
    fetch_pr_list: prListFixture,
    fetch_pr_detail: prDetailFixture,
    fetch_github_user: {
      login: "e2e",
      ghStatus: { available: true, authenticated: true },
    },
    list_worktrees: cleanupRowsFixture,
    check_gh_available: true,
    get_cleanup_pr_statuses: {
      "feature/stale-worktree": "merged",
      "feature/warning-worktree": "open",
    },
    get_cleanup_branch_protection: ["main", "develop"],
    get_cleanup_settings: {
      delete_remote_branches: false,
    },
    check_app_update: {
      state: "up_to_date",
      checked_at: "2026-03-23T00:00:00.000Z",
    },
  };
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

test("version history opens as a top-level tab and renders summaries", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
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
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: {
          openVersionHistoryTab: () => void;
          getTabs: () => Array<{ id: string; type: string }>;
          getActiveTabId: () => string;
        };
      }
    ).__GWT_E2E_APP__?.openVersionHistoryTab();
  });
  const versionState = await page.evaluate(() => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          getTabs: () => Array<{ id: string; type: string }>;
          getActiveTabId: () => string;
        };
      }
    ).__GWT_E2E_APP__;
    return app ? { tabs: app.getTabs(), activeTabId: app.getActiveTabId() } : null;
  });
  expect(versionState).not.toBeNull();
  expect((versionState as any).tabs).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ id: "versionHistory", type: "versionHistory" }),
    ]),
  );
  await captureUxSnapshot(page, testInfo, "version-history-tab");
});

test("project index opens as a top-level tab and can search", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
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
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: {
          openProjectIndexTab: () => void;
          getTabs: () => Array<{ id: string; type: string }>;
          getActiveTabId: () => string;
        };
      }
    ).__GWT_E2E_APP__?.openProjectIndexTab();
  });
  const projectIndexState = await page.evaluate(() => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          getTabs: () => Array<{ id: string; type: string }>;
          getActiveTabId: () => string;
        };
      }
    ).__GWT_E2E_APP__;
    return app ? { tabs: app.getTabs(), activeTabId: app.getActiveTabId() } : null;
  });
  expect(projectIndexState).not.toBeNull();
  expect((projectIndexState as any).tabs).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ id: "projectIndex", type: "projectIndex" }),
    ]),
  );
  await captureUxSnapshot(page, testInfo, "project-index-tab");
});

test("cleanup modal opens from menu action with mixed safety rows", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "cleanup-worktrees" });

  await expect(
    page.getByRole("dialog", { name: /cleanup/i }),
  ).toBeVisible();
  const cleanupDialog = page.getByRole("dialog", { name: /cleanup/i });
  await expect(cleanupDialog.getByText("feature/stale-worktree")).toBeVisible();
  await expect(cleanupDialog.getByText("feature/warning-worktree")).toBeVisible();
  await captureUxSnapshot(page, testInfo, "cleanup-modal");
});

test("check updates menu action surfaces the current status toast", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "check-updates" });

  await expect(page.getByText("Up to date.")).toBeVisible();
});

test("check updates available with downloadable asset shows update action", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    check_app_update: {
      state: "available",
      current: "1.0.0",
      latest: "1.1.0",
      release_url: "https://github.com/example/gwt/releases/tag/v1.1.0",
      asset_url: "https://github.com/example/gwt/releases/download/v1.1.0/gwt.dmg",
      checked_at: "2026-03-23T00:00:00.000Z",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "check-updates" });

  await expect(page.getByText("Update available: v1.1.0 (click update)")).toBeVisible();
  await expect(page.getByRole("button", { name: "Update" })).toBeVisible();
});

test("check updates available without asset shows manual download message", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    check_app_update: {
      state: "available",
      current: "1.0.0",
      latest: "1.1.0",
      release_url: "https://github.com/example/gwt/releases/tag/v1.1.0",
      asset_url: null,
      checked_at: "2026-03-23T00:00:00.000Z",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "check-updates" });

  await expect(
    page.getByText("Update available: v1.1.0. Manual download required."),
  ).toBeVisible();
});

test("check updates failure shows toast error", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    check_app_update: {
      state: "failed",
      message: "network unreachable",
      failed_at: "2026-03-23T00:00:00.000Z",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "check-updates" });

  await expect(page.getByText("Update check failed: network unreachable")).toBeVisible();
});

test("about menu action opens the About dialog", async ({ page }, testInfo) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "about" });

  await expect(page.locator(".about-dialog")).toBeVisible();
  await expect(page.getByText("Git Worktree Manager")).toBeVisible();
  await captureUxSnapshot(page, testInfo, "about-dialog");
});

test("about dialog close button hides the modal", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "about" });
  await expect(page.locator(".about-dialog")).toBeVisible();
  await page.locator(".about-dialog .about-close").click();
  await expect(page.locator(".about-dialog")).toHaveCount(0);
});

test("report issue menu action opens the bug report dialog", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  await expect(page.getByRole("dialog", { name: "Report" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Bug Report" })).toBeVisible();
});

test("E2E hook can reuse the current shell PR tab directly", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: { openPullRequestsTab: () => void };
      }
    ).__GWT_E2E_APP__?.openPullRequestsTab();
  });

  const prState = await page.evaluate(() => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          getTabs: () => Array<{ id: string; type: string }>;
          getActiveTabId: () => string;
        };
      }
    ).__GWT_E2E_APP__;
    return app ? { tabs: app.getTabs(), activeTabId: app.getActiveTabId() } : null;
  });
  expect(prState).not.toBeNull();
  expect((prState as any).tabs).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ id: "prs", type: "prs" }),
    ]),
  );
});

test("E2E hook covers App helper normalization branches", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  const result = await page.evaluate(() => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          toErrorMessage: (value: unknown) => string;
          clampFontSize: (size: number) => number;
          normalizeVoiceInputSettings: (
            value: Record<string, unknown> | null | undefined,
          ) => Record<string, unknown>;
          normalizeAppLanguage: (value: string | null | undefined) => string;
          normalizeUiFontFamily: (value: string | null | undefined) => string;
          normalizeTerminalFontFamily: (
            value: string | null | undefined,
          ) => string;
          parseE1004BranchName: (message: string) => string | null;
          terminalTabLabel: (
            pathLike: string | null | undefined,
            fallback?: string,
          ) => string;
          agentTabLabel: (agentId: string) => string;
          getAgentTabRestoreDelayMs: (attempt: number) => number;
          persistAgentPasteHintDismissed: () => void;
          isTauriRuntimeAvailable: () => boolean;
          shouldHandleExternalLinkClickSample: (
            href: string,
            options?: Record<string, unknown>,
          ) => boolean;
        };
      }
    ).__GWT_E2E_APP__;
    if (!app) return null;

    app.persistAgentPasteHintDismissed();

    return {
      stringError: app.toErrorMessage("boom"),
      objectError: app.toErrorMessage({ message: "object-boom" }),
      jsonError: app.toErrorMessage({ other: 1 }),
      fontClampLow: app.clampFontSize(3),
      fontClampHigh: app.clampFontSize(99),
      fontClampRound: app.clampFontSize(12.6),
      voiceDefault: app.normalizeVoiceInputSettings(null),
      voiceInvalid: app.normalizeVoiceInputSettings({
        enabled: true,
        engine: "weird",
        language: "de",
        quality: "turbo",
        model: "",
      }),
      voiceFast: app.normalizeVoiceInputSettings({
        enabled: false,
        engine: "whisper",
        language: "ja",
        quality: "fast",
        model: "",
      }),
      appLanguageValid: app.normalizeAppLanguage("ja"),
      appLanguageInvalid: app.normalizeAppLanguage("fr"),
      uiFontDefault: app.normalizeUiFontFamily(""),
      terminalFontDefault: app.normalizeTerminalFontFamily(""),
      branchQuoted: app.parseE1004BranchName(
        "[E1004] Branch already exists: 'feature/demo'",
      ),
      branchRaw: app.parseE1004BranchName(
        "[E1004] Branch already exists: feature/raw",
      ),
      branchNone: app.parseE1004BranchName("different error"),
      terminalLabelEmpty: app.terminalTabLabel("", "Fallback"),
      terminalLabelRoot: app.terminalTabLabel("/", "Fallback"),
      terminalLabelNested: app.terminalTabLabel("/tmp/worktrees/feature-demo"),
      agentClaude: app.agentTabLabel("claude"),
      agentUnknown: app.agentTabLabel("custom-agent"),
      retryDelay0: app.getAgentTabRestoreDelayMs(0),
      retryDelayLarge: app.getAgentTabRestoreDelayMs(20),
      tauriAvailable: app.isTauriRuntimeAvailable(),
      externalAllow: app.shouldHandleExternalLinkClickSample(
        "https://example.com",
      ),
      externalMeta: app.shouldHandleExternalLinkClickSample(
        "https://example.com",
        { metaKey: true },
      ),
      externalDownload: app.shouldHandleExternalLinkClickSample(
        "https://example.com",
        { download: true },
      ),
      externalInvalid: app.shouldHandleExternalLinkClickSample(
        "mailto:test@example.com",
      ),
    };
  });

  expect(result).not.toBeNull();
  expect(result).toMatchObject({
    stringError: "boom",
    objectError: "object-boom",
    fontClampLow: 8,
    fontClampHigh: 24,
    fontClampRound: 13,
    appLanguageValid: "ja",
    appLanguageInvalid: "auto",
    branchQuoted: "feature/demo",
    branchRaw: "feature/raw",
    branchNone: null,
    terminalLabelEmpty: "Fallback",
    terminalLabelNested: "feature-demo",
    agentClaude: "Claude Code",
    agentUnknown: "custom-agent",
    tauriAvailable: true,
    externalAllow: true,
    externalMeta: false,
    externalDownload: false,
    externalInvalid: false,
  });
  expect((result as any).voiceInvalid).toMatchObject({
    enabled: true,
    engine: "qwen3-asr",
    language: "auto",
    quality: "balanced",
    model: "Qwen/Qwen3-ASR-1.7B",
  });
  expect((result as any).voiceFast).toMatchObject({
    engine: "qwen3-asr",
    language: "ja",
    quality: "fast",
    model: "Qwen/Qwen3-ASR-0.6B",
  });
  expect((result as any).uiFontDefault).toContain("system-ui");
  expect((result as any).terminalFontDefault).toContain("JetBrains Mono");
  expect((result as any).retryDelay0).toBeGreaterThan(0);
  expect((result as any).retryDelayLarge).toBeGreaterThanOrEqual(
    (result as any).retryDelay0,
  );
});

test("E2E hook can mutate Issues tab state safely on the launch screen", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());

  const state = await page.evaluate(() => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          openIssuesTab: () => void;
          setIssueCount: (count: number) => void;
          getTabs: () => Array<{ id: string; label: string; type: string }>;
          getActiveTabId: () => string;
        };
      }
    ).__GWT_E2E_APP__;
    if (!app) return null;
    app.openIssuesTab();
    app.setIssueCount(4);
    return {
      activeTabId: app.getActiveTabId(),
      tabs: app.getTabs(),
    };
  });

  expect(state).not.toBeNull();
  expect((state as any).activeTabId).toBe("issues");
  expect((state as any).tabs).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        id: "issues",
        type: "issues",
        label: "Issues (4)",
      }),
    ]),
  );
});

test("E2E hook covers App shell-state helper branches", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  const state = await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          getTabs: () => Array<{ id: string; label: string; type: string }>;
          forceActiveTabId: (tabId: string) => void;
          readVoiceFallbackTerminalPaneId: () => string | null;
          reorderTabs: (
            dragTabId: string,
            overTabId: string,
            position: "before" | "after",
          ) => void;
          openDocsEditor: (worktreePath: string) => Promise<void>;
          getDocsEditorAutoClosePaneIds: () => string[];
        };
      }
    ).__GWT_E2E_APP__;
    if (!app) return null;

    const terminalTab = app.getTabs().find((tab) => tab.type === "terminal");
    if (terminalTab) {
      app.forceActiveTabId(terminalTab.id);
    }
    const fallbackPaneId = app.readVoiceFallbackTerminalPaneId();

    app.reorderTabs("branchBrowser", "agentCanvas", "before");

    return {
      fallbackPaneId,
      tabs: app.getTabs(),
    };
  });

  expect(state).not.toBeNull();
  expect((state as any).tabs).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ id: "branchBrowser", type: "branchBrowser" }),
      expect.objectContaining({ id: "agentCanvas", type: "agentCanvas" }),
    ]),
  );
});

test("E2E hook covers App update, appearance, and window-session helpers", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    "plugin:dialog|confirm": false,
    get_current_window_label: "main",
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  const result = await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          getToastMessage: () => string | null;
          isReportDialogOpen: () => boolean;
          getTabs: () => Array<{ id: string; type: string }>;
          activateTab: (tabId: string) => void;
          getActiveAgentPaneId: () => string | null;
          showAvailableUpdateToast: (
            state: Record<string, unknown>,
            force?: boolean,
          ) => void;
          handleToastClick: () => Promise<void>;
          showReportDialog: (
            mode: "bug" | "feature",
            prefillError?: Record<string, unknown>,
          ) => void;
          applyAppearanceSettings: () => Promise<void>;
          resolveCurrentWindowLabel: () => Promise<string | null>;
          updateWindowSession: (projectPathForWindow: string | null) => Promise<void>;
        };
      }
    ).__GWT_E2E_APP__;
    if (!app) return null;

    const terminalTab = app.getTabs().find((tab) => tab.type === "terminal");
    if (terminalTab) {
      app.activateTab(terminalTab.id);
    }
    const activePaneId = app.getActiveAgentPaneId();
    app.showAvailableUpdateToast({
      state: "available",
      current: "1.0.0",
      latest: "2.0.0",
      release_url: "https://github.com/example/gwt/releases/tag/v2.0.0",
      asset_url: "https://github.com/example/gwt/releases/download/v2.0.0/gwt.dmg",
    }, true);
    await app.handleToastClick();

    app.showAvailableUpdateToast({
      state: "available",
      current: "1.0.0",
      latest: "2.0.1",
      release_url: "https://github.com/example/gwt/releases/tag/v2.0.1",
      asset_url: null,
    }, true);
    app.showReportDialog("feature", {
      severity: "error",
      message: "prefilled error",
      source: "e2e",
      timestamp: "2026-03-23T00:00:00.000Z",
    });
    await app.applyAppearanceSettings();
    const firstLabel = await app.resolveCurrentWindowLabel();
    const secondLabel = await app.resolveCurrentWindowLabel();
    await app.updateWindowSession("/tmp/gwt-playwright-alt");
    const sessionsAfterUpsert = window.localStorage.getItem("gwt.windowSessions.v1");
    await app.updateWindowSession(null);
    const sessionsAfterRemove = window.localStorage.getItem("gwt.windowSessions.v1");
    return {
      activePaneId,
      firstLabel,
      secondLabel,
      sessionsAfterUpsert,
      sessionsAfterRemove,
      uiFontBase: document.documentElement.style.getPropertyValue("--ui-font-base"),
      uiFontFamily: document.documentElement.style.getPropertyValue("--ui-font-family"),
      terminalFontFamily: document.documentElement.style.getPropertyValue(
        "--terminal-font-family",
      ),
      terminalFontSize: (window as any).__gwtTerminalFontSize ?? null,
      terminalFontFamilyValue: (window as any).__gwtTerminalFontFamily ?? null,
      toastMessage: app.getToastMessage(),
      reportDialogOpen: app.isReportDialogOpen(),
    };
  });

  expect(result).not.toBeNull();
  expect((result as any).activePaneId).toBeNull();
  expect((result as any).firstLabel).toBe("main");
  expect((result as any).secondLabel).toBe("main");
  expect((result as any).uiFontBase).toContain("13px");
  expect((result as any).uiFontFamily).toContain("system-ui");
  expect((result as any).terminalFontFamily).toContain("JetBrains Mono");
  expect((result as any).terminalFontSize).toBe(13);
  expect((result as any).terminalFontFamilyValue).toContain("JetBrains Mono");
  expect((result as any).sessionsAfterUpsert).toContain("/tmp/gwt-playwright-alt");
  expect((result as any).sessionsAfterRemove).not.toContain("/tmp/gwt-playwright-alt");
  expect((result as any).toastMessage).toBeNull();
  expect((result as any).reportDialogOpen).toBe(true);
});

test("E2E hook covers update-toast and window-label edge branches", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    "plugin:dialog|confirm": true,
    apply_app_update: { __error: "apply boom" },
    get_current_window_label: "",
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  const edge = await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          showAvailableUpdateToast: (
            state: Record<string, unknown>,
            force?: boolean,
          ) => void;
          getToastMessage: () => string | null;
          handleToastClick: () => Promise<void>;
          resetCurrentWindowLabelCache: () => void;
          resolveCurrentWindowLabel: () => Promise<string | null>;
        };
      }
    ).__GWT_E2E_APP__;
    if (!app) return null;

    app.showAvailableUpdateToast({
      state: "available",
      current: "1.0.0",
      latest: "3.0.0",
      release_url: "https://github.com/example/gwt/releases/tag/v3.0.0",
      asset_url: "https://github.com/example/gwt/releases/download/v3.0.0/gwt.dmg",
    }, true);
    const firstToast = app.getToastMessage();

    app.showAvailableUpdateToast({
      state: "available",
      current: "1.0.0",
      latest: "3.0.0",
      release_url: "https://github.com/example/gwt/releases/tag/v3.0.0",
      asset_url: null,
    }, false);
    const secondToast = app.getToastMessage();

    await app.handleToastClick();
    const afterApplyError = app.getToastMessage();

    app.resetCurrentWindowLabelCache();
    const emptyLabel = await app.resolveCurrentWindowLabel();
    return {
      firstToast,
      secondToast,
      afterApplyError,
      emptyLabel,
    };
  });

  expect(edge).not.toBeNull();
  expect((edge as any).firstToast).toContain("Update available: v3.0.0");
  expect((edge as any).secondToast).toBe((edge as any).firstToast);
  expect((edge as any).afterApplyError).toContain("Failed to apply update: apply boom");
  expect((edge as any).emptyLabel).toBeNull();

  await setMockCommandResponses(page, {
    ...toolResponses(),
    get_current_window_label: { __error: "label lookup failed" },
  });
  const failedLabel = await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          resetCurrentWindowLabelCache: () => void;
          resolveCurrentWindowLabel: () => Promise<string | null>;
        };
      }
    ).__GWT_E2E_APP__;
    if (!app) return null;
    app.resetCurrentWindowLabelCache();
    return app.resolveCurrentWindowLabel();
  });
  expect(failedLabel).toBeNull();
});

test("E2E hook can activate a terminal tab and route it back through Agent Canvas", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  const tabId = await expect
    .poll(async () => {
      return page.evaluate(() => {
        const app = (
          window as unknown as {
            __GWT_E2E_APP__?: {
              getTabs: () => Array<{ id: string; type: string }>;
            };
          }
        ).__GWT_E2E_APP__;
        return app?.getTabs().find((tab) => tab.type === "terminal")?.id ?? null;
      });
    })
    .toBeTruthy()
    .then(async () =>
      page.evaluate(() => {
        const app = (
          window as unknown as {
            __GWT_E2E_APP__?: {
              getTabs: () => Array<{ id: string; type: string }>;
            };
          }
        ).__GWT_E2E_APP__;
        return app?.getTabs().find((tab) => tab.type === "terminal")?.id ?? null;
      }),
    );
  if (!tabId) throw new Error("terminal tab id missing");

  const active = await page.evaluate((targetTabId) => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          activateTab: (tabId: string) => void;
          getActiveTabId: () => string;
        };
      }
    ).__GWT_E2E_APP__;
    app?.activateTab(targetTabId);
    return app?.getActiveTabId() ?? null;
  }, tabId);

  expect([tabId, "agentCanvas"]).toContain(active);
  await expectAgentCanvasVisible(page);
});

test("E2E hook can open Cleanup modal with a preselected branch", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: { requestCleanup: (branchName?: string) => void };
      }
    ).__GWT_E2E_APP__?.requestCleanup("feature/stale-worktree");
  });

  const dialog = page.getByRole("dialog", { name: /cleanup/i });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("feature/stale-worktree")).toBeVisible();
});

test("E2E hook surfaces worktree materialization errors as a toast", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    materialize_worktree_ref: { __error: "materialize failed" },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate((branch) => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: {
          activateBranch: (branch: {
            name: string;
            is_current: boolean;
            commit: string;
            ahead: number;
            behind: number;
            divergence_status: string;
            last_tool_usage: null;
            is_agent_running: boolean;
            commit_timestamp: number;
          }) => Promise<void>;
        };
      }
    ).__GWT_E2E_APP__?.activateBranch(branch);
  }, {
    name: "feature/workflow-demo",
    commit: "bbb1111",
    is_current: false,
    ahead: 1,
    behind: 0,
    divergence_status: "Ahead",
    last_tool_usage: null,
    is_agent_running: false,
    commit_timestamp: 1_700_000_050,
  });

  await expect(page.getByText("Failed to open worktree: materialize failed")).toBeVisible();
});

test("E2E hook can close a terminal tab even when backend close fails", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  const tabId = await page.evaluate(() => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: { getTabs: () => Array<{ id: string; type: string }> };
      }
    ).__GWT_E2E_APP__;
    return app?.getTabs().find((tab) => tab.type === "terminal")?.id ?? null;
  });
  if (!tabId) throw new Error("terminal tab id missing");

  await setMockCommandResponses(page, {
    ...toolResponses(),
    close_terminal: { __error: "close failed" },
  });

  await page.evaluate(async (targetTabId) => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: { closeTab: (tabId: string) => Promise<void> };
      }
    ).__GWT_E2E_APP__;
    await app?.closeTab(targetTabId);
  }, tabId);

  await expectAgentCanvasVisible(page);
});

test("suggest feature menu action opens the feature request dialog", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "suggest-feature" });

  await expect(page.getByRole("dialog", { name: "Report" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Feature Request" })).toBeVisible();
});

test("git pull requests menu action opens Pull Requests as a top-level tab", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: { openPullRequestsTab: () => void };
      }
    ).__GWT_E2E_APP__?.openPullRequestsTab();
  });
  const prMenuState = await page.evaluate(() => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          getTabs: () => Array<{ id: string; type: string }>;
          getActiveTabId: () => string;
        };
      }
    ).__GWT_E2E_APP__;
    return app ? { tabs: app.getTabs(), activeTabId: app.getActiveTabId() } : null;
  });
  expect(prMenuState).not.toBeNull();
  expect(Array.isArray((prMenuState as any).tabs)).toBe(true);
});

test("toggle sidebar switches the current shell to Branch Browser", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: { openBranchBrowserTab: () => void };
      }
    ).__GWT_E2E_APP__?.openBranchBrowserTab();
  });
  const branchState = await page.evaluate(() => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          getTabs: () => Array<{ id: string; type: string }>;
          getActiveTabId: () => string;
        };
      }
    ).__GWT_E2E_APP__;
    return app ? { tabs: app.getTabs(), activeTabId: app.getActiveTabId() } : null;
  });
  expect(branchState).not.toBeNull();
  expect((branchState as any).tabs).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ id: "branchBrowser", type: "branchBrowser" }),
    ]),
  );
});

test("keyboard shortcut opens Settings from the current shell", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: ",",
        metaKey: true,
        bubbles: true,
      }),
    );
  });

  await expect(page.locator('[data-tab-id="settings"]')).toHaveClass(/active/);
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
});

test("keyboard shortcut opens Cleanup modal from the current shell", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "K",
        metaKey: true,
        shiftKey: true,
        bubbles: true,
      }),
    );
  });

  await expect(page.getByRole("dialog", { name: /cleanup/i })).toBeVisible();
});

test("list terminals returns focus to Agent Canvas with the active terminal session", async ({
  page,
}) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toBeVisible();

  await emitTauriEvent(page, "menu-action", { action: "list-terminals" });
  await expect(page.locator('[data-tab-id="agentCanvas"]')).toHaveClass(/active/);
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toBeVisible();
});

test("new terminal failure keeps Agent Canvas stable without adding a session tile", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    spawn_shell: { __error: "spawn failed" },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toHaveCount(0);
  await expect(page.locator('[data-tab-id="agentCanvas"]')).toHaveClass(/active/);
});

test("os-env-fallback event shows a toast message", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForEventListener(page, "os-env-fallback");
  await emitTauriEvent(page, "os-env-fallback", "login shell failed");

  await expect(
    page.getByText(
      "Shell environment not loaded: login shell failed. Using process environment.",
    ),
  ).toBeVisible();
});

test("update toast can be dismissed from the current shell", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    check_app_update: {
      state: "available",
      current: "1.0.0",
      latest: "1.2.0",
      release_url: "https://github.com/example/gwt/releases/tag/v1.2.0",
      asset_url: "https://github.com/example/gwt/releases/download/v1.2.0/gwt.dmg",
      checked_at: "2026-03-23T00:00:00.000Z",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "check-updates" });

  await expect(page.getByText("Update available: v1.2.0 (click update)")).toBeVisible();
  await expect(page.getByRole("button", { name: "Update" })).toBeVisible();
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByText("Update available: v1.2.0 (click update)")).toBeHidden();
});

test("update toast can confirm and invoke apply_app_update", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    check_app_update: {
      state: "available",
      current: "1.0.0",
      latest: "1.2.1",
      release_url: "https://github.com/example/gwt/releases/tag/v1.2.1",
      asset_url: "https://github.com/example/gwt/releases/download/v1.2.1/gwt.dmg",
      checked_at: "2026-03-23T00:00:00.000Z",
    },
    "plugin:dialog|confirm": true,
    apply_app_update: null,
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "check-updates" });

  await page.getByRole("button", { name: "Update" }).click();
  await waitForInvokeCommand(page, "apply_app_update");
  await expect(page.getByText("Updating to v1.2.1...")).toBeVisible();
});

test("update apply failure shows an error toast", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    check_app_update: {
      state: "available",
      current: "1.0.0",
      latest: "1.2.2",
      release_url: "https://github.com/example/gwt/releases/tag/v1.2.2",
      asset_url: "https://github.com/example/gwt/releases/download/v1.2.2/gwt.dmg",
      checked_at: "2026-03-23T00:00:00.000Z",
    },
    "plugin:dialog|confirm": true,
    apply_app_update: {
      __error: "apply failed",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "check-updates" });

  await page.getByRole("button", { name: "Update" }).click();
  await expect(page.getByText("Failed to apply update: apply failed")).toBeVisible();
});

test("report-error toast can be dismissed without opening the dialog", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(async () => {
    const mod = await import("/src/lib/errorBus.ts");
    mod.errorBus.emit({
      severity: "error",
      code: "E9999",
      message: "Dismiss me",
      command: "dismiss_test",
      category: "test",
      suggestions: [],
      timestamp: "2026-03-23T00:00:00.000Z",
    });
  });

  await expect(page.getByText("Error: Dismiss me")).toBeVisible();
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByText("Error: Dismiss me")).toBeHidden();
});

test("debug os env menu action opens the captured environment dialog", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    get_captured_environment: {
      entries: [
        { key: "SHELL", value: "/bin/zsh" },
        { key: "TERM", value: "xterm-256color" },
      ],
      source: "login_shell",
      reason: null,
      ready: true,
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "debug-os-env" });

  await expect(page.getByText("Captured Environment")).toBeVisible();
  await expect(page.getByText("Login Shell")).toBeVisible();
  await expect(page.getByText("SHELL", { exact: true })).toBeVisible();
});

test("captured environment dialog can be dismissed", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    get_captured_environment: {
      entries: [{ key: "SHELL", value: "/bin/zsh" }],
      source: "login_shell",
      reason: null,
      ready: true,
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "debug-os-env" });
  await expect(page.getByText("Captured Environment")).toBeVisible();
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByText("Captured Environment")).toBeHidden();
});

test("captured environment dialog backdrop click hides the modal", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    get_captured_environment: {
      entries: [{ key: "SHELL", value: "/bin/zsh" }],
      source: "login_shell",
      reason: null,
      ready: true,
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "debug-os-env" });
  await expect(page.getByText("Captured Environment")).toBeVisible();
  await page.locator(".env-debug-dialog").evaluate((el) => {
    (el.parentElement as HTMLElement | null)?.click();
  });
  await expect(page.getByText("Captured Environment")).toBeHidden();
});

test("screen copy success shows copied toast", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async () => undefined,
      },
    });
  });

  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "screen-copy" });

  await expect(page.getByText("Copied to clipboard")).toBeVisible();
});

test("screen copy failure shows error toast", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async () => {
          throw new Error("clipboard unavailable");
        },
      },
    });
  });

  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "screen-copy" });

  await expect(page.getByText("Failed to copy screen text")).toBeVisible();
});

test("meta-click on an external link does not use the interceptor", async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as { __GWT_OPENED_URLS__?: string[] }).__GWT_OPENED_URLS__ = [];
    window.open = ((url?: string | URL | undefined) => {
      (
        window as unknown as { __GWT_OPENED_URLS__?: string[] }
      ).__GWT_OPENED_URLS__?.push(String(url ?? ""));
      return {} as Window;
    }) as typeof window.open;
  });

  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    const host = document.querySelector(".app-layout") ?? document.body;
    const link = document.createElement("a");
    link.href = "https://example.com/meta-ignore";
    link.textContent = "Meta External Link";
    link.style.position = "fixed";
    link.style.top = "52px";
    link.style.right = "24px";
    link.style.zIndex = "99999";
    host.appendChild(link);
  });

  await page.getByText("Meta External Link").click({ force: true, modifiers: ["Meta"] });
  await page.waitForTimeout(250);
  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          (
            window as unknown as { __GWT_OPENED_URLS__?: string[] }
          ).__GWT_OPENED_URLS__ ?? [],
      ),
    )
    .toEqual([]);
});

test("download link click is ignored by the external link interceptor", async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as { __GWT_OPENED_URLS__?: string[] }).__GWT_OPENED_URLS__ = [];
    window.open = ((url?: string | URL | undefined) => {
      (
        window as unknown as { __GWT_OPENED_URLS__?: string[] }
      ).__GWT_OPENED_URLS__?.push(String(url ?? ""));
      return {} as Window;
    }) as typeof window.open;
  });

  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    const host = document.querySelector(".app-layout") ?? document.body;
    const link = document.createElement("a");
    link.href = "https://example.com/download";
    link.textContent = "Download Link";
    link.setAttribute("download", "artifact.txt");
    link.style.position = "fixed";
    link.style.top = "80px";
    link.style.right = "24px";
    link.style.zIndex = "99999";
    host.appendChild(link);
  });

  await page.getByText("Download Link").click({ force: true });
  await page.waitForTimeout(250);
  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          (
            window as unknown as { __GWT_OPENED_URLS__?: string[] }
          ).__GWT_OPENED_URLS__ ?? [],
      ),
    )
    .toEqual([]);
});

test("dynamic open-recent-project action opens a project from the launch screen", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_path: {
      kind: "gwtProject",
      projectPath: defaultRecentProject.path,
      migrationSourceRoot: null,
      message: null,
    },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", {
    action: `open-recent-project::${defaultRecentProject.path}`,
  });

  await expectAgentCanvasVisible(page);
});

test("open-project menu action can open a selected gwt project", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    "plugin:dialog|open": "/tmp/menu-open-project",
    probe_path: {
      kind: "gwtProject",
      projectPath: "/tmp/menu-open-project",
      migrationSourceRoot: null,
      message: null,
    },
    open_project: {
      action: "opened",
      info: {
        path: "/tmp/menu-open-project",
        repo_name: "menu-open-project",
        current_branch: "main",
      },
      focusedWindowLabel: null,
    },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "open-project" });

  await expectAgentCanvasVisible(page);
  await waitForInvokeCommand(page, "probe_path");
  await waitForInvokeCommand(page, "open_project");
});

test("open-project menu action shows empty directory error", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    "plugin:dialog|open": "/tmp/empty-dir",
    probe_path: {
      kind: "emptyDir",
      projectPath: null,
      migrationSourceRoot: null,
      message: "Empty directory",
    },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "open-project" });

  await expect(
    page.getByText("Selected folder is empty. Use New Project on the start screen."),
  ).toBeVisible();
});

test("open-project keyboard shortcut can open Settings fallback error for invalid path", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    "plugin:dialog|open": "/tmp/invalid-dir",
    probe_path: {
      kind: "invalid",
      projectPath: null,
      migrationSourceRoot: null,
      message: null,
    },
  });

  await page.evaluate(() => {
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "o",
        metaKey: true,
        bubbles: true,
      }),
    );
  });

  await expect(page.getByText("Invalid path.")).toBeVisible();
});

test("open-project menu action can route to migration modal", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    "plugin:dialog|open": "/tmp/migrate-open-project",
    probe_path: {
      kind: "migrationRequired",
      projectPath: null,
      migrationSourceRoot: "/tmp/migrate-open-project",
      message: "Migration required",
    },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "open-project" });

  await expect(
    page.getByRole("dialog", { name: "Migration Required" }),
  ).toBeVisible();
});

test("open-project menu action shows not-a-project fallback message", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    "plugin:dialog|open": "/tmp/plain-folder",
    probe_path: {
      kind: "notGwtProject",
      projectPath: null,
      migrationSourceRoot: null,
      message: null,
    },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "open-project" });

  await expect(page.getByText("Not a gwt project.")).toBeVisible();
});

test("open-project menu action surfaces probe failure", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    "plugin:dialog|open": "/tmp/broken-open-project",
    probe_path: {
      __error: "dialog probe failed",
    },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "open-project" });

  await expect(
    page.getByText("Failed to open project: dialog probe failed"),
  ).toBeVisible();
});

test("open-project menu action surfaces dialog failure", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    "plugin:dialog|open": {
      __error: "dialog open failed",
    },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "open-project" });

  await expect(
    page.getByText("Failed to open project: dialog open failed"),
  ).toBeVisible();
});

test("app error close button hides the error dialog", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    "plugin:dialog|open": { __error: "dialog open failed" },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "open-project" });
  await expect(page.getByRole("heading", { name: "Error" })).toBeVisible();
  await page.locator(".error-dialog .about-close").click();
  await expect(page.getByRole("heading", { name: "Error" })).toBeHidden();
});

test("app error backdrop click hides the error dialog", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    "plugin:dialog|open": { __error: "dialog open failed" },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "open-project" });
  await expect(page.getByRole("heading", { name: "Error" })).toBeVisible();
  await page.locator(".error-dialog").evaluate((el) => {
    (el.parentElement as HTMLElement | null)?.click();
  });
  await expect(page.getByRole("heading", { name: "Error" })).toBeHidden();
});

test("dynamic open-recent-project action can route to migration modal", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_path: {
      kind: "migrationRequired",
      projectPath: null,
      migrationSourceRoot: "/tmp/migrate-me",
      message: "Migration required",
    },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", {
    action: "open-recent-project::/tmp/migrate-me",
  });

  await expect(
    page.getByRole("dialog", { name: "Migration Required" }),
  ).toBeVisible();
});

test("migration modal quit fallback dismisses the modal when quit_app fails", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_path: {
      kind: "migrationRequired",
      projectPath: null,
      migrationSourceRoot: "/tmp/migrate-me",
      message: "Migration required",
    },
    quit_app: {
      __error: "quit unavailable",
    },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", {
    action: "open-recent-project::/tmp/migrate-me",
  });

  const modal = page.getByRole("dialog", { name: "Migration Required" });
  await expect(modal).toBeVisible();
  await modal.getByRole("button", { name: "Quit" }).click();
  await expect(modal).toBeHidden();
});

test("previous-tab and next-tab cycle top-level tabs in the current shell", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "project-index" });
  await emitTauriEvent(page, "menu-action", { action: "open-settings" });
  await expect(page.locator('[data-tab-id="settings"]')).toHaveClass(/active/);

  await emitTauriEvent(page, "menu-action", { action: "previous-tab" });
  await expect(page.locator('[data-tab-id="projectIndex"]')).toHaveClass(/active/);

  await emitTauriEvent(page, "menu-action", { action: "next-tab" });
  await expect(page.locator('[data-tab-id="settings"]')).toHaveClass(/active/);
});

test("keyboard shortcuts cycle top-level tabs in the current shell", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "project-index" });
  await emitTauriEvent(page, "menu-action", { action: "open-settings" });
  await expect(page.locator('[data-tab-id="settings"]')).toHaveClass(/active/);

  await page.evaluate(() => {
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "{",
        metaKey: true,
        shiftKey: true,
        bubbles: true,
      }),
    );
  });
  await expect(page.locator('[data-tab-id="projectIndex"]')).toHaveClass(/active/);

  await page.evaluate(() => {
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "}",
        metaKey: true,
        shiftKey: true,
        bubbles: true,
      }),
    );
  });
  await expect(page.locator('[data-tab-id="settings"]')).toHaveClass(/active/);
});

test("focus-agent-tab dynamic action returns to the selected terminal session", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  const terminalTile = page.locator('[data-testid^="agent-canvas-session-terminal-"]').first();
  await expect(terminalTile).toBeVisible();
  const tabId = await terminalTile.getAttribute("data-testid");
  const terminalTabId = tabId?.replace("agent-canvas-session-", "");
  if (!terminalTabId) throw new Error("terminal tab id missing");

  await emitTauriEvent(page, "menu-action", { action: "toggle-sidebar" });
  await expect(page.locator('[data-tab-id="branchBrowser"]')).toHaveClass(/active/);

  await emitTauriEvent(page, "menu-action", {
    action: `focus-agent-tab::${terminalTabId}`,
  });
  await expect(page.locator('[data-tab-id="agentCanvas"]')).toHaveClass(/active/);
  await expect(terminalTile).toBeVisible();
});

test("edit copy and paste dispatch terminal edit actions for the active terminal", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    (window as unknown as { __GWT_TERMINAL_EDIT_EVENTS__?: Array<unknown> })
      .__GWT_TERMINAL_EDIT_EVENTS__ = [];
    window.addEventListener("gwt-terminal-edit-action", (event) => {
      const detail = (event as CustomEvent).detail;
      (
        window as unknown as { __GWT_TERMINAL_EDIT_EVENTS__?: Array<unknown> }
      ).__GWT_TERMINAL_EDIT_EVENTS__?.push(detail);
    });
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toBeVisible();

  await emitTauriEvent(page, "menu-action", { action: "edit-copy" });
  await emitTauriEvent(page, "menu-action", { action: "edit-paste" });

  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          (
            window as unknown as { __GWT_TERMINAL_EDIT_EVENTS__?: Array<{ action: string }> }
          ).__GWT_TERMINAL_EDIT_EVENTS__ ?? [],
      ),
    )
    .toEqual(
      expect.arrayContaining([
        expect.objectContaining({ action: "copy" }),
        expect.objectContaining({ action: "paste" }),
      ]),
    );
});

test("list terminals with no session keeps Agent Canvas focused", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "list-terminals" });

  await expect(page.locator('[data-tab-id="agentCanvas"]')).toHaveClass(/active/);
  await expectAgentCanvasVisible(page);
});

test("terminal diagnostics without an active terminal shows an error dialog", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "terminal-diagnostics" });

  await expect(page.getByRole("heading", { name: "Error" })).toBeVisible();
  await expect(page.getByText("No active terminal tab.")).toBeVisible();
});

test("terminal diagnostics with an active terminal shows the probe dialog", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_terminal_ansi: {
      pane_id: "mock-pane-1",
      bytes_scanned: 1024,
      esc_count: 12,
      sgr_count: 10,
      color_sgr_count: 0,
      has_256_color: false,
      has_true_color: false,
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toBeVisible();

  await emitTauriEvent(page, "menu-action", { action: "terminal-diagnostics" });
  await expect(page.getByText("Terminal Diagnostics")).toBeVisible();
  await expect(page.getByText("mock-pane-1")).toBeVisible();
});

test("terminal diagnostics close button hides the dialog", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_terminal_ansi: {
      pane_id: "mock-pane-1",
      bytes_scanned: 1024,
      esc_count: 12,
      sgr_count: 10,
      color_sgr_count: 0,
      has_256_color: false,
      has_true_color: false,
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await emitTauriEvent(page, "menu-action", { action: "terminal-diagnostics" });
  await expect(page.getByText("Terminal Diagnostics")).toBeVisible();
  await page.locator(".diag-dialog .about-close").click();
  await expect(page.getByText("Terminal Diagnostics")).toBeHidden();
});

test("terminal diagnostics backdrop click hides the dialog", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_terminal_ansi: {
      pane_id: "mock-pane-1",
      bytes_scanned: 1024,
      esc_count: 12,
      sgr_count: 10,
      color_sgr_count: 0,
      has_256_color: false,
      has_true_color: false,
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await emitTauriEvent(page, "menu-action", { action: "terminal-diagnostics" });
  await expect(page.getByText("Terminal Diagnostics")).toBeVisible();
  await page.locator(".diag-dialog").evaluate((el) => {
    (el.parentElement as HTMLElement | null)?.click();
  });
  await expect(page.getByText("Terminal Diagnostics")).toBeHidden();
});

test("dynamic open recent project notFound shows an error dialog", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_path: {
      kind: "notFound",
      projectPath: null,
      migrationSourceRoot: null,
      message: "Path does not exist.",
    },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", {
    action: "open-recent-project::/tmp/does-not-exist",
  });

  await expect(page.getByRole("heading", { name: "Error" })).toBeVisible();
  await expect(page.getByText("Path does not exist.")).toBeVisible();
});

test("error dialog can be dismissed after a recent-project failure", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_path: {
      kind: "notFound",
      projectPath: null,
      migrationSourceRoot: null,
      message: "Path does not exist.",
    },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", {
    action: "open-recent-project::/tmp/does-not-exist",
  });

  await expect(page.getByRole("heading", { name: "Error" })).toBeVisible();
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("heading", { name: "Error" })).toBeHidden();
});

test("dynamic open recent project exception shows a project-open error dialog", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_path: {
      __error: "mock probe failure",
    },
  });

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", {
    action: "open-recent-project::/tmp/broken-project",
  });

  await expect(page.getByRole("heading", { name: "Error" })).toBeVisible();
  await expect(
    page.getByText("Failed to open project: mock probe failure"),
  ).toBeVisible();
});

test("close project clears shell state and returns to the launch screen", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toBeVisible();

  await emitTauriEvent(page, "menu-action", { action: "close-project" });
  await waitForInvokeCommand(page, "close_project");
  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
});

test("close project still returns to launch screen when terminal cleanup fails", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    close_terminal: { __error: "close terminal failed" },
    close_project: { __error: "close project failed" },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toBeVisible();

  await emitTauriEvent(page, "menu-action", { action: "close-project" });
  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
});

test("edit copy and paste fall back to plain inputs outside terminals", async ({
  page,
}) => {
  await page.addInitScript(() => {
    let clipboardText = "";
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (value: string) => {
          clipboardText = value;
        },
        readText: async () => clipboardText,
      },
    });
    (window as unknown as { __GWT_CLIPBOARD_TEXT__?: () => string }).__GWT_CLIPBOARD_TEXT__ =
      () => clipboardText;
  });

  await page.goto("/");
  await page.getByRole("button", { name: "New Project" }).click();

  const repoInput = page.getByPlaceholder("https://github.com/owner/repo");
  await repoInput.fill("https://github.com/example/gwt.git");
  await repoInput.press("Meta+A");

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "edit-copy" });

  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          (
            window as unknown as { __GWT_CLIPBOARD_TEXT__?: () => string }
          ).__GWT_CLIPBOARD_TEXT__?.() ?? "",
      ),
    )
    .toBe("https://github.com/example/gwt.git");

  await repoInput.fill("");
  await emitTauriEvent(page, "menu-action", { action: "edit-paste" });
  await expect(repoInput).toHaveValue("https://github.com/example/gwt.git");
});

test("terminal diagnostics failure shows probe error inside the dialog", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_terminal_ansi: {
      __error: "probe failed",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toBeVisible();

  await emitTauriEvent(page, "menu-action", { action: "terminal-diagnostics" });
  await expect(page.getByText("Terminal Diagnostics")).toBeVisible();
  await expect(page.getByText(/Failed to probe terminal:/)).toBeVisible();
});

test("toast bus events surface a toast message", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(async () => {
    const mod = await import("/src/lib/toastBus.ts");
    mod.toastBus.emit({ message: "Toast bus message", durationMs: 0 });
  });

  await expect(page.getByText("Toast bus message")).toBeVisible();
});

test("error bus events surface a report toast action that opens the report dialog", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(async () => {
    const mod = await import("/src/lib/errorBus.ts");
    mod.errorBus.emit({
      severity: "error",
      code: "E7777",
      message: "Critical shell error",
      command: "probe_shell",
      category: "shell",
      suggestions: ["Retry"],
      timestamp: "2026-03-23T00:00:00.000Z",
    });
  });

  await expect(page.getByText("Error: Critical shell error")).toBeVisible();
  await page.getByRole("button", { name: "Report" }).click();
  await expect(page.getByRole("dialog", { name: "Report" })).toBeVisible();
  await page.getByRole("button", { name: "Error Details" }).click();
  await expect(page.getByText("Code: E7777")).toBeVisible();
});

test("gwt-settings-updated reapplies language and voice settings", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    window.dispatchEvent(
      new CustomEvent("gwt-settings-updated", {
        detail: {
          appLanguage: "ja",
          voiceInput: {
            enabled: true,
            engine: "qwen3-asr",
            language: "ja",
            quality: "balanced",
            model: "Qwen/Qwen3-ASR-1.7B",
          },
        },
      }),
    );
  });

  await waitForInvokeCommand(page, "rebuild_all_branch_session_summaries");
});

test("gwt-settings-updated normalizes invalid fonts and language", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    window.dispatchEvent(
      new CustomEvent("gwt-settings-updated", {
        detail: {
          uiFontSize: 30,
          terminalFontSize: 2,
          uiFontFamily: "",
          terminalFontFamily: "",
          appLanguage: "fr",
          voiceInput: {
            enabled: true,
            engine: "invalid-engine",
            language: "fr",
            quality: "invalid-quality",
            model: "",
          },
        },
      }),
    );
  });

  await expect
    .poll(async () =>
      page.evaluate(() => ({
        uiFontBase: getComputedStyle(document.documentElement).getPropertyValue("--ui-font-base").trim(),
        uiFontFamily: getComputedStyle(document.documentElement)
          .getPropertyValue("--ui-font-family")
          .trim(),
        terminalFontFamily: getComputedStyle(document.documentElement)
          .getPropertyValue("--terminal-font-family")
          .trim(),
        terminalFontSize: (window as unknown as { __gwtTerminalFontSize?: number }).__gwtTerminalFontSize ?? null,
        terminalFontFamilyWindow:
          (window as unknown as { __gwtTerminalFontFamily?: string }).__gwtTerminalFontFamily ?? "",
      })),
    )
    .toEqual(
      expect.objectContaining({
        uiFontBase: "24px",
        terminalFontSize: 8,
      }),
    );
});

test("gwt-settings-updated shows rebuild failure toast when summary refresh fails", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    rebuild_all_branch_session_summaries: {
      __error: "rebuild failed",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    window.dispatchEvent(
      new CustomEvent("gwt-settings-updated", {
        detail: {
          appLanguage: "ja",
        },
      }),
    );
  });

  await expect(
    page.getByText("Failed to rebuild summaries: rebuild failed"),
  ).toBeVisible();
});

test("startup diagnostics can disable automatic startup update checks", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    get_startup_diagnostics: {
      startupTrace: false,
      disableTray: false,
      disableLoginShellCapture: false,
      disableHeartbeatWatchdog: false,
      disableSessionWatcher: false,
      disableStartupUpdateCheck: true,
      disableProfiling: false,
      disableTabRestore: false,
      disableWindowSessionRestore: false,
    },
    check_app_update: {
      state: "available",
      current: "1.0.0",
      latest: "9.9.9",
      release_url: "https://github.com/example/gwt/releases/tag/v9.9.9",
      asset_url: "https://github.com/example/gwt/releases/download/v9.9.9/gwt.dmg",
      checked_at: "2026-03-23T00:00:00.000Z",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.waitForTimeout(3500);
  await expect(page.getByText("Update available: v9.9.9 (click update)")).toBeHidden();
});


test("worktrees-changed event for another project is ignored", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    list_worktrees: [],
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await setMockCommandResponses(page, {
    ...toolResponses(),
    list_worktrees: [
      {
        path: "/tmp/worktrees/foreign-branch",
        branch: "feature/foreign",
        commit: "fff0000",
        status: "active",
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
    ],
  });
  await emitTauriEvent(page, "worktrees-changed", {
    project_path: "/tmp/other-project",
  });
  await expect(
    page.locator('[data-testid="agent-canvas-worktree-tile-feature-foreign"]'),
  ).toBeHidden();
});

test("terminal-cwd-changed for another pane is ignored", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);
  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toBeVisible();

  await emitTauriEvent(page, "terminal-cwd-changed", {
    pane_id: "some-other-pane",
    cwd: "/tmp/ignored-name",
  });
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).not.toContainText("ignored-name");
});

test("project mode open-spec event opens an Issue Spec tab", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    window.dispatchEvent(
      new CustomEvent("gwt-project-mode-open-spec-issue", {
        detail: {
          issueNumber: 777,
          issueUrl: "https://github.com/example/gwt/issues/777",
        },
      }),
    );
  });

  await expect(page.locator('[data-tab-id="issueSpec"]')).toHaveClass(/active/);
  await expect(page.locator('[data-tab-id="issueSpec"]')).toContainText("Issue #777");
});

test("reopening top-level tools reuses existing tabs instead of duplicating them", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "open-settings" });
  await emitTauriEvent(page, "menu-action", { action: "open-settings" });
  await emitTauriEvent(page, "menu-action", { action: "version-history" });
  await emitTauriEvent(page, "menu-action", { action: "version-history" });
  await emitTauriEvent(page, "menu-action", { action: "project-index" });
  await emitTauriEvent(page, "menu-action", { action: "project-index" });
  await emitTauriEvent(page, "menu-action", { action: "git-pull-requests" });
  await emitTauriEvent(page, "menu-action", { action: "git-pull-requests" });

  await expect(page.locator('[data-tab-id="settings"]')).toHaveCount(1);
  await expect(page.locator('[data-tab-id="versionHistory"]')).toHaveCount(1);
  await expect(page.locator('[data-tab-id="projectIndex"]')).toHaveCount(1);
  await expect(page.locator('[data-tab-id="prs"]')).toHaveCount(1);
});

test("project mode open-spec event reuses the existing Issue Spec tab", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    window.dispatchEvent(
      new CustomEvent("gwt-project-mode-open-spec-issue", {
        detail: {
          issueNumber: 777,
          issueUrl: "https://github.com/example/gwt/issues/777",
        },
      }),
    );
  });
  await expect(page.locator('[data-tab-id="issueSpec"]')).toContainText("Issue #777");

  await page.evaluate(() => {
    window.dispatchEvent(
      new CustomEvent("gwt-project-mode-open-spec-issue", {
        detail: {
          issueNumber: 778,
          issueUrl: "https://github.com/example/gwt/issues/778",
        },
      }),
    );
  });

  await expect(page.locator('[data-tab-id="issueSpec"]')).toHaveCount(1);
  await expect(page.locator('[data-tab-id="issueSpec"]')).toContainText("Issue #778");
});

test("E2E hook can open Launch Agent modal from the current shell", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: { requestAgentLaunch: () => void };
      }
    ).__GWT_E2E_APP__?.requestAgentLaunch();
  });

  await expect(page.getByRole("dialog", { name: "Launch Agent" })).toBeVisible();
});

test("E2E hook can open CI logs into a terminal session", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    fetch_ci_log: "ci line 1\nci line 2\n",
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: { openCiLog: (runId: number) => Promise<void> };
      }
    ).__GWT_E2E_APP__;
    await app?.openCiLog(100);
  });

  await waitForInvokeCommand(page, "spawn_shell");
  await waitForInvokeCommand(page, "fetch_ci_log");
  await waitForInvokeCommand(page, "write_terminal");
  await expect(page.locator('[data-testid^="agent-canvas-session-terminal-"]')).toBeVisible();
});

test("E2E hook can trigger worktree label refresh after display-name change", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  const before = await page.evaluate(() => {
    const log = (
      window as unknown as {
        __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
      }
    ).__GWT_TAURI_INVOKE_LOG__ ?? [];
    return log.filter((entry) => entry.cmd === "list_worktree_branches").length;
  });

  await page.evaluate(() => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: { branchDisplayNameChanged: () => void };
      }
    ).__GWT_E2E_APP__?.branchDisplayNameChanged();
  });

  await expect
    .poll(async () =>
      page.evaluate(() => {
        const log = (
          window as unknown as {
            __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
          }
        ).__GWT_TAURI_INVOKE_LOG__ ?? [];
        return log.filter((entry) => entry.cmd === "list_worktree_branches").length;
      }),
    )
    .toBeGreaterThan(before);
});

test("E2E hook can prefill Launch Agent from an issue", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: {
          workOnIssue: (issue: {
            number: number;
            title: string;
            body: string;
            state: string;
            updatedAt: string;
            htmlUrl: string;
            labels: Array<{ name: string; color: string }>;
            assignees: unknown[];
            commentsCount: number;
          }) => void;
        };
      }
    ).__GWT_E2E_APP__?.workOnIssue({
      number: 101,
      title: "Prefilled issue",
      body: "Issue body",
      state: "open",
      updatedAt: "2026-03-23T00:00:00.000Z",
      htmlUrl: "https://github.com/example/gwt/issues/101",
      labels: [{ name: "bug", color: "d73a4a" }],
      assignees: [],
      commentsCount: 1,
    });
  });

  await expect(page.getByRole("dialog", { name: "Launch Agent" })).toBeVisible();
  await expect(page.getByText("Auto-generated from issue #101")).toBeVisible();
});

test("E2E hook can switch to Branch Browser for a worktree branch", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(() => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: {
          switchToWorktree: (branchName: string) => void;
        };
      }
    ).__GWT_E2E_APP__?.switchToWorktree("feature/workflow-demo");
  });

  await expect(page.locator('[data-tab-id="branchBrowser"]')).toHaveClass(/active/);
});

test("E2E hook can select an existing canvas terminal session", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  const sessionTile = page.locator('[data-testid^="agent-canvas-session-terminal-"]').first();
  await expect(sessionTile).toBeVisible();
  const tabId = await sessionTile.getAttribute("data-testid");
  const terminalTabId = tabId?.replace("agent-canvas-session-", "");
  if (!terminalTabId) throw new Error("terminal tab id missing");

  await page.evaluate((targetTabId) => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: {
          selectCanvasSession: (tabId: string) => void;
        };
      }
    ).__GWT_E2E_APP__?.selectCanvasSession(targetTabId);
  }, terminalTabId);

  await expect(page.locator('[data-tab-id="agentCanvas"]')).toHaveClass(/active/);
  await expect(sessionTile).toBeVisible();
});

test("E2E hook can open docs editor terminal", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      value: "Win32",
    });
  });

  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    get_settings: {
      ...settingsFixture,
      default_shell: "powershell",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: { openDocsEditor: (worktreePath: string) => Promise<void> };
      }
    ).__GWT_E2E_APP__;
    await app?.openDocsEditor("/tmp/gwt-playwright");
  });

  await waitForInvokeCommand(page, "spawn_shell");
  await waitForInvokeCommand(page, "write_terminal");
  await expect(page.locator('[data-testid^="agent-canvas-session-terminal-"]')).toBeVisible();
});

test("E2E hook falls back to cmd when Windows docs editor shell settings are unavailable", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      value: "Win32",
    });
  });

  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    get_settings: { __error: "settings unavailable" },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: { openDocsEditor: (worktreePath: string) => Promise<void> };
      }
    ).__GWT_E2E_APP__;
    await app?.openDocsEditor("/tmp/gwt-playwright");
  });

  await waitForInvokeCommand(page, "spawn_shell");
  const spawnArgs = await page.evaluate(() => {
    const log = (
      window as unknown as {
        __GWT_TAURI_INVOKE_LOG__?: Array<{
          cmd: string;
          args?: Record<string, unknown>;
        }>;
      }
    ).__GWT_TAURI_INVOKE_LOG__ ?? [];
    const entry = [...log].reverse().find((item) => item.cmd === "spawn_shell");
    return entry?.args ?? null;
  });
  expect(spawnArgs).toMatchObject({
    workingDir: "/tmp/gwt-playwright",
    shell: "cmd",
  });
});

test("E2E hook tracks docs editor auto-close panes for Windows WSL flows", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      value: "Win32",
    });
  });

  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    get_settings: {
      ...settingsFixture,
      default_shell: "wsl",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  const autoClosePaneIds = await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          openDocsEditor: (worktreePath: string) => Promise<void>;
          getDocsEditorAutoClosePaneIds: () => string[];
        };
      }
    ).__GWT_E2E_APP__;
    await app?.openDocsEditor("/tmp/gwt-playwright");
    return app?.getDocsEditorAutoClosePaneIds() ?? [];
  });

  expect(autoClosePaneIds.length).toBeGreaterThan(0);
});

test("terminal-closed event removes the terminal session from Agent Canvas", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toHaveCount(2);

  const paneId = await page.evaluate(
    () =>
      (
        window as unknown as { __GWT_MOCK_LAST_SPAWNED_PANE_ID__?: () => string | null }
      ).__GWT_MOCK_LAST_SPAWNED_PANE_ID__?.() ?? null,
  );
  if (!paneId) throw new Error("mock pane id missing");

  await emitTauriEvent(page, "terminal-closed", { pane_id: paneId });
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toHaveCount(1);
});

test("terminal-cwd-changed event updates the terminal session label", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  const paneId = await page.evaluate(
    () =>
      (
        window as unknown as { __GWT_MOCK_LAST_SPAWNED_PANE_ID__?: () => string | null }
      ).__GWT_MOCK_LAST_SPAWNED_PANE_ID__?.() ?? null,
  );
  if (!paneId) throw new Error("mock pane id missing");

  await emitTauriEvent(page, "terminal-cwd-changed", {
    pane_id: paneId,
    cwd: "/tmp/gwt-playwright/worktrees/renamed-terminal",
  });
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toContainText("renamed-terminal");
});

test("worktrees-changed event refreshes canvas worktrees for the active project", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    list_worktrees: [],
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await setMockCommandResponses(page, {
    ...toolResponses(),
    list_worktrees: [
      {
        path: "/tmp/worktrees/feature-added",
        branch: "feature/added",
        commit: "ddd4444",
        status: "active",
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
    ],
  });

  await emitTauriEvent(page, "worktrees-changed", {
    project_path: "/tmp/gwt-playwright",
  });
  await expect(
    page.locator('[data-testid="agent-canvas-worktree-tile-feature-added"]'),
  ).toBeVisible();
});

test("window-will-hide removes the current window session from storage", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  const before = await page.evaluate(() => window.localStorage.getItem("gwt.windowSessions.v1"));
  expect(before).toContain("/tmp/gwt-playwright");

  await emitTauriEvent(page, "window-will-hide", {});

  await expect
    .poll(async () =>
      page.evaluate(() => window.localStorage.getItem("gwt.windowSessions.v1")),
    )
    .not.toContain("/tmp/gwt-playwright");
});

test("closing a top-level tab removes it from the shell", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "project-index" });
  await expect(page.locator('[data-tab-id="projectIndex"]')).toBeVisible();

  await page
    .locator('[data-tab-id="projectIndex"] .tab-close')
    .click();

  await expect(page.locator('[data-tab-id="projectIndex"]')).toHaveCount(0);
});

test("E2E hook can restore stored top-level shell tabs for the current project", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate((projectPath) => {
    window.localStorage.setItem(
      "gwt.projectTabs.v2",
      JSON.stringify({
        version: 3,
        byProjectPath: {
          [`${projectPath}::window=main`]: {
            tabs: [
              { type: "settings", id: "settings", label: "Settings" },
              { type: "prs", id: "prs", label: "Pull Requests" },
              {
                type: "versionHistory",
                id: "versionHistory",
                label: "Version History",
              },
              { type: "projectIndex", id: "projectIndex", label: "Project Index" },
            ],
            activeTabId: "projectIndex",
          },
        },
      }),
    );
  }, defaultRecentProject.path);

  await page.evaluate((projectPath) => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: { restoreProjectTabs: (path: string) => void };
      }
    ).__GWT_E2E_APP__?.restoreProjectTabs(projectPath);
  }, defaultRecentProject.path);

  await expect(page.locator('[data-tab-id="settings"]')).toBeVisible();
  await expect(page.locator('[data-tab-id="prs"]')).toBeVisible();
  await expect(page.locator('[data-tab-id="versionHistory"]')).toBeVisible();
  await expect(page.locator('[data-tab-id="projectIndex"]')).toHaveClass(/active/);
  await captureUxSnapshot(page, testInfo, "restore-top-level-tabs");
});

test("E2E hook can open a stored window session into the current shell", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_path: {
      kind: "gwtProject",
      projectPath: defaultRecentProject.path,
    },
    open_project: {
      action: "opened",
      info: {
        path: defaultRecentProject.path,
      },
    },
  });

  await page.evaluate((projectPath) => {
    window.localStorage.setItem(
      "gwt.windowSessions.v1",
      JSON.stringify([{ label: "main", projectPath }]),
    );
  }, defaultRecentProject.path);

  await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: { applyWindowSession: (label: string) => Promise<void> };
      }
    ).__GWT_E2E_APP__;
    await app?.applyWindowSession("main");
  });

  await expectAgentCanvasVisible(page);
  await expect(page.locator(".recent-item")).toHaveCount(0);
});

test("E2E hook can respawn a stored terminal session during tab restore", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate((projectPath) => {
    window.localStorage.setItem(
      "gwt.projectTabs.v2",
      JSON.stringify({
        version: 3,
        byProjectPath: {
          [`${projectPath}::window=main`]: {
            tabs: [
              {
                type: "terminal",
                paneId: "stale-pane",
                label: "Saved Terminal",
                cwd: "/tmp/gwt-playwright/worktrees/restored",
              },
            ],
            activeTabId: "terminal-stale-pane",
            activeCanvasSessionTabId: "terminal-stale-pane",
          },
        },
      }),
    );
  }, defaultRecentProject.path);

  const spawnCountBefore = await page.evaluate(() => {
    const log = (
      window as unknown as {
        __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
      }
    ).__GWT_TAURI_INVOKE_LOG__ ?? [];
    return log.filter((entry) => entry.cmd === "spawn_shell").length;
  });

  await page.evaluate((projectPath) => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: { restoreProjectTabs: (path: string) => void };
      }
    ).__GWT_E2E_APP__?.restoreProjectTabs(projectPath);
  }, defaultRecentProject.path);

  await expect
    .poll(async () =>
      page.evaluate(() => {
        const log = (
          window as unknown as {
            __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
          }
        ).__GWT_TAURI_INVOKE_LOG__ ?? [];
        return log.filter((entry) => entry.cmd === "spawn_shell").length;
      }),
    )
    .toBeGreaterThan(spawnCountBefore);
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toBeVisible();
});

test("restore hook ignores stored tabs when startup diagnostics disable tab restore", async ({
  page,
}) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
      ...toolResponses(),
      get_startup_diagnostics: {
        startupTrace: false,
        disableTray: false,
        disableLoginShellCapture: false,
        disableHeartbeatWatchdog: false,
        disableSessionWatcher: false,
        disableStartupUpdateCheck: false,
        disableProfiling: false,
        disableTabRestore: true,
        disableWindowSessionRestore: false,
      },
    },
  });
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await page.evaluate((projectPath) => {
    window.localStorage.setItem(
      "gwt.projectTabs.v2",
      JSON.stringify({
        version: 3,
        byProjectPath: {
          [`${projectPath}::window=main`]: {
            tabs: [{ type: "settings", id: "settings", label: "Settings" }],
            activeTabId: "settings",
          },
        },
      }),
    );
  }, defaultRecentProject.path);

  await page.evaluate((projectPath) => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: { restoreProjectTabs: (path: string) => void };
      }
    ).__GWT_E2E_APP__?.restoreProjectTabs(projectPath);
  }, defaultRecentProject.path);

  await expect(page.locator('[data-tab-id="settings"]')).toHaveCount(0);
  await expect(page.locator('[data-tab-id="agentCanvas"]')).toBeVisible();
  await expect(page.locator('[data-tab-id="branchBrowser"]')).toBeVisible();
});

test("E2E hook can apply a stored window session that requires migration", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_path: {
      kind: "migrationRequired",
      migrationSourceRoot: "/tmp/gwt-playwright-migration",
      message: "Migration required",
    },
  });

  await page.evaluate(() => {
    window.localStorage.setItem(
      "gwt.windowSessions.v1",
      JSON.stringify([
        {
          label: "main",
          projectPath: "/tmp/gwt-playwright-migration",
        },
      ]),
    );
  });

  await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: { applyWindowSession: (label: string) => Promise<void> };
      }
    ).__GWT_E2E_APP__;
    await app?.applyWindowSession("main");
  });

  await expect(
    page.getByRole("dialog", { name: "Migration Required" }),
  ).toBeVisible();
});

test("E2E hook surfaces focused-existing restore errors from stored window sessions", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_path: {
      kind: "gwtProject",
      projectPath: defaultRecentProject.path,
    },
    open_project: {
      action: "focusedExisting",
      focusedWindowLabel: "review-2",
      info: {
        path: defaultRecentProject.path,
      },
    },
  });

  await page.evaluate((projectPath) => {
    window.localStorage.setItem(
      "gwt.windowSessions.v1",
      JSON.stringify([{ label: "main", projectPath }]),
    );
  }, defaultRecentProject.path);

  await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: { applyWindowSession: (label: string) => Promise<void> };
      }
    ).__GWT_E2E_APP__;
    await app?.applyWindowSession("main");
  });

  await expect(page.locator(".error-text")).toContainText(
    "Project is already open in window review-2.",
  );
});

test("E2E hook surfaces restore errors from stored window sessions", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_path: {
      __error: "probe exploded",
    },
  });

  await page.evaluate((projectPath) => {
    window.localStorage.setItem(
      "gwt.windowSessions.v1",
      JSON.stringify([{ label: "main", projectPath }]),
    );
  }, defaultRecentProject.path);

  await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: { applyWindowSession: (label: string) => Promise<void> };
      }
    ).__GWT_E2E_APP__;
    await app?.applyWindowSession("main");
  });

  await expect(page.locator(".error-text")).toContainText(
    "Failed to restore project: probe exploded",
  );
});

test("E2E hook ignores missing stored window sessions without changing the shell", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, toolResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  const activeBefore = await page.evaluate(() => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: { getActiveTabId: () => string };
      }
    ).__GWT_E2E_APP__;
    return app?.getActiveTabId() ?? null;
  });

  await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: { applyWindowSession: (label: string) => Promise<void> };
      }
    ).__GWT_E2E_APP__;
    await app?.applyWindowSession("missing-window");
  });

  const activeAfter = await page.evaluate(() => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: { getActiveTabId: () => string };
      }
    ).__GWT_E2E_APP__;
    return app?.getActiveTabId() ?? null;
  });
  expect(activeAfter).toBe(activeBefore);
});

test("E2E hook clears stale stored window sessions when probe marks them invalid", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...toolResponses(),
    probe_path: {
      kind: "notGwtProject",
      message: "Not a gwt project.",
    },
  });

  await page.evaluate((projectPath) => {
    window.localStorage.setItem(
      "gwt.windowSessions.v1",
      JSON.stringify([{ label: "main", projectPath }]),
    );
  }, defaultRecentProject.path);

  await page.evaluate(async () => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: { applyWindowSession: (label: string) => Promise<void> };
      }
    ).__GWT_E2E_APP__;
    await app?.applyWindowSession("main");
  });

  await expect
    .poll(async () =>
      page.evaluate(() => window.localStorage.getItem("gwt.windowSessions.v1")),
    )
    .not.toContain(defaultRecentProject.path);
});
