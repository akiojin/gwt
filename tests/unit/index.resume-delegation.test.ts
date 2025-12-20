import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SelectionResult } from "../../src/cli/ui/components/App.js";
import type { ExecutionMode } from "../../src/cli/ui/components/screens/ExecutionModeSelectorScreen.js";

// Vitest shim for environments lacking vi.hoisted (e.g., bun)
if (typeof (vi as Record<string, unknown>).hoisted !== "function") {
  // @ts-expect-error injected shim
  vi.hoisted = (factory: () => unknown) => factory();
}

const {
  ensureWorktreeMock,
  fetchAllRemotesMock,
  pullFastForwardMock,
  getBranchDivergenceStatusesMock,
  worktreeExistsMock,
  getRepositoryRootMock,
  getToolByIdMock,
  getSharedEnvironmentMock,
  installDependenciesMock,
  launchCodexCLIMock,
  saveSessionMock,
  loadSessionMock,
  findLatestCodexSessionMock,
  hasUncommittedChangesMock,
  hasUnpushedCommitsMock,
  getUncommittedChangesCountMock,
  getUnpushedCommitsCountMock,
  pushBranchToRemoteMock,
} = vi.hoisted(() => ({
  ensureWorktreeMock: vi.fn(async () => "/repo/worktrees/feature/resume"),
  fetchAllRemotesMock: vi.fn(async () => undefined),
  pullFastForwardMock: vi.fn(async () => undefined),
  getBranchDivergenceStatusesMock: vi.fn(async () => []),
  worktreeExistsMock: vi.fn(async () => null),
  getRepositoryRootMock: vi.fn(async () => "/repo"),
  getToolByIdMock: vi.fn(() => ({ id: "codex-cli", displayName: "Codex" })),
  getSharedEnvironmentMock: vi.fn(async () => ({})),
  installDependenciesMock: vi.fn(async () => ({
    skipped: false as const,
    manager: "bun" as const,
    lockfile: "/repo/bun.lock",
  })),
  launchCodexCLIMock: vi.fn(async () => ({ sessionId: null })),
  saveSessionMock: vi.fn(async () => undefined),
  loadSessionMock: vi.fn(async () => ({
    lastWorktreePath: "/repo/worktrees/feature/resume",
    lastBranch: "feature/resume",
    lastUsedTool: "codex-cli",
    toolLabel: "Codex",
    mode: "continue",
    model: null,
    reasoningLevel: null,
    skipPermissions: null,
    timestamp: Date.now(),
    repositoryRoot: "/repo",
    lastSessionId: "saved-session-id",
    history: [
      {
        branch: "feature/resume",
        worktreePath: "/repo/worktrees/feature/resume",
        toolId: "codex-cli",
        toolLabel: "Codex",
        sessionId: "saved-session-id",
        mode: "continue",
        model: null,
        reasoningLevel: null,
        skipPermissions: null,
        timestamp: Date.now(),
      },
    ],
  })),
  findLatestCodexSessionMock: vi.fn(async () => null),
  hasUncommittedChangesMock: vi.fn(async () => false),
  hasUnpushedCommitsMock: vi.fn(async () => false),
  getUncommittedChangesCountMock: vi.fn(async () => 0),
  getUnpushedCommitsCountMock: vi.fn(async () => 0),
  pushBranchToRemoteMock: vi.fn(async () => undefined),
}));

const waitForUserAcknowledgementMock = vi.hoisted(() =>
  vi.fn<() => Promise<void>>(),
);

const confirmYesNoMock = vi.hoisted(() => vi.fn<() => Promise<boolean>>());
vi.mock("../../src/git.js", async () => {
  const actual =
    await vi.importActual<typeof import("../../src/git.js")>(
      "../../src/git.js",
    );
  return {
    ...actual,
    getRepositoryRoot: getRepositoryRootMock,
    fetchAllRemotes: fetchAllRemotesMock,
    pullFastForward: pullFastForwardMock,
    getBranchDivergenceStatuses: getBranchDivergenceStatusesMock,
    branchExists: vi.fn(async () => true),
    hasUncommittedChanges: hasUncommittedChangesMock,
    hasUnpushedCommits: hasUnpushedCommitsMock,
    getUncommittedChangesCount: getUncommittedChangesCountMock,
    getUnpushedCommitsCount: getUnpushedCommitsCountMock,
    pushBranchToRemote: pushBranchToRemoteMock,
  };
});

vi.mock("../../src/worktree.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/worktree.js")>(
    "../../src/worktree.js",
  );
  return {
    ...actual,
    worktreeExists: worktreeExistsMock,
    resolveWorktreePathForBranch: vi.fn(async (branch: string) => ({
      path: await worktreeExistsMock(branch),
    })),
    isProtectedBranchName: vi.fn(() => false),
    switchToProtectedBranch: vi.fn(async () => "none" as const),
  };
});

vi.mock("../../src/services/WorktreeOrchestrator.js", () => ({
  WorktreeOrchestrator: class {
    ensureWorktree = ensureWorktreeMock;
  },
}));

const DependencyInstallErrorMock = vi.hoisted(
  () =>
    class DependencyInstallError extends Error {
      constructor(message?: string) {
        super(message);
        this.name = "DependencyInstallError";
      }
    },
);

vi.mock("../../src/services/dependency-installer.js", () => ({
  installDependenciesForWorktree: installDependenciesMock,
  DependencyInstallError: DependencyInstallErrorMock,
}));

vi.mock("../../src/config/tools.js", () => ({
  getToolById: getToolByIdMock,
  getSharedEnvironment: getSharedEnvironmentMock,
}));

vi.mock("../../src/config/index.js", () => ({
  saveSession: saveSessionMock,
  loadSession: loadSessionMock,
}));

vi.mock("../../src/codex.js", () => ({
  launchCodexCLI: launchCodexCLIMock,
  CodexError: class CodexError extends Error {
    constructor(
      message: string,
      public cause?: unknown,
    ) {
      super(message);
      this.name = "CodexError";
    }
  },
}));

vi.mock("../../src/utils/session.js", () => ({
  findLatestCodexSession: findLatestCodexSessionMock,
  findLatestClaudeSession: vi.fn(async () => null),
  findLatestGeminiSession: vi.fn(async () => null),
  findLatestClaudeSessionId: vi.fn(async () => null),
}));

vi.mock("../../src/utils/terminal.js", async () => {
  const actual = await vi.importActual<
    typeof import("../../src/utils/terminal.js")
  >("../../src/utils/terminal.js");
  return {
    ...actual,
    waitForUserAcknowledgement: waitForUserAcknowledgementMock,
  };
});

vi.mock("../../src/utils/prompt.js", async () => {
  const actual = await vi.importActual<
    typeof import("../../src/utils/prompt.js")
  >("../../src/utils/prompt.js");
  return {
    ...actual,
    confirmYesNo: confirmYesNoMock,
  };
});
// Import after mocks are set up
import { handleAIToolWorkflow } from "../../src/index.js";

beforeEach(() => {
  ensureWorktreeMock.mockClear();
  fetchAllRemotesMock.mockClear();
  pullFastForwardMock.mockClear();
  getBranchDivergenceStatusesMock.mockClear();
  worktreeExistsMock.mockClear();
  getRepositoryRootMock.mockClear();
  getToolByIdMock.mockClear();
  getSharedEnvironmentMock.mockClear();
  installDependenciesMock.mockClear();
  launchCodexCLIMock.mockClear();
  saveSessionMock.mockClear();
  loadSessionMock.mockClear();
  findLatestCodexSessionMock.mockClear();
  hasUncommittedChangesMock.mockClear();
  hasUnpushedCommitsMock.mockClear();
  getUncommittedChangesCountMock.mockClear();
  getUnpushedCommitsCountMock.mockClear();
  pushBranchToRemoteMock.mockClear();
  waitForUserAcknowledgementMock.mockClear();
  waitForUserAcknowledgementMock.mockResolvedValue(undefined);
  confirmYesNoMock.mockClear();
  confirmYesNoMock.mockResolvedValue(false);
  hasUncommittedChangesMock.mockResolvedValue(false);
  hasUnpushedCommitsMock.mockResolvedValue(false);
  getUncommittedChangesCountMock.mockResolvedValue(0);
  getUnpushedCommitsCountMock.mockResolvedValue(0);
  pushBranchToRemoteMock.mockResolvedValue(undefined);
});

describe("handleAIToolWorkflow - Resume delegation", () => {
  const baseSelection: Omit<SelectionResult, "mode"> = {
    branch: "feature/resume",
    displayName: "feature/resume",
    branchType: "local",
    tool: "codex-cli",
    skipPermissions: false,
    model: "gpt-5.2-codex",
  };

  it("does not auto-resolve sessionId when mode=resume (delegates to tool resume)", async () => {
    const selection: SelectionResult = {
      ...baseSelection,
      mode: "resume" as ExecutionMode,
      sessionId: null,
    };

    vi.useFakeTimers();
    const run = handleAIToolWorkflow(selection);
    await vi.advanceTimersByTimeAsync(3000);
    await run;
    vi.useRealTimers();

    expect(loadSessionMock).not.toHaveBeenCalled();
    expect(launchCodexCLIMock).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({
        mode: "resume",
        sessionId: null,
      }),
    );
  });

  it("auto-resolves sessionId when mode=continue and none is provided", async () => {
    const selection: SelectionResult = {
      ...baseSelection,
      mode: "continue" as ExecutionMode,
      sessionId: null,
    };

    vi.useFakeTimers();
    const run = handleAIToolWorkflow(selection);
    await vi.advanceTimersByTimeAsync(3000);
    await run;
    vi.useRealTimers();

    expect(loadSessionMock).toHaveBeenCalledTimes(1);
    expect(launchCodexCLIMock).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({
        mode: "continue",
        sessionId: "saved-session-id",
      }),
    );
  });
});
