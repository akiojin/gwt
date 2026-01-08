import { beforeEach, describe, expect, it, mock } from "bun:test";
import type { SelectionResult } from "../../src/cli/ui/App.solid.js";
import type { ExecutionMode } from "../../src/cli/ui/App.solid.js";

const {
  ensureWorktreeMock,
  fetchAllRemotesMock,
  pullFastForwardMock,
  getBranchDivergenceStatusesMock,
  worktreeExistsMock,
  getRepositoryRootMock,
  getCodingAgentByIdMock,
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
} = {
  ensureWorktreeMock: mock(async () => "/repo/worktrees/feature/resume"),
  fetchAllRemotesMock: mock(async () => undefined),
  pullFastForwardMock: mock(async () => undefined),
  getBranchDivergenceStatusesMock: mock(async () => []),
  worktreeExistsMock: mock(async () => null),
  getRepositoryRootMock: mock(async () => "/repo"),
  getCodingAgentByIdMock: mock(async () => ({
    id: "codex-cli",
    displayName: "Codex",
    type: "command",
    command: "codex",
    modeArgs: { normal: [] },
  })),
  getSharedEnvironmentMock: mock(async () => ({})),
  installDependenciesMock: mock(async () => ({
    skipped: false as const,
    manager: "bun" as const,
    lockfile: "/repo/bun.lock",
  })),
  launchCodexCLIMock: mock(async () => ({ sessionId: null })),
  saveSessionMock: mock(async () => undefined),
  loadSessionMock: mock(async () => ({
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
  findLatestCodexSessionMock: mock(async () => null),
  hasUncommittedChangesMock: mock(async () => false),
  hasUnpushedCommitsMock: mock(async () => false),
  getUncommittedChangesCountMock: mock(async () => 0),
  getUnpushedCommitsCountMock: mock(async () => 0),
  pushBranchToRemoteMock: mock(async () => undefined),
};

const waitForUserAcknowledgementMock = mock<() => Promise<void>>();
const confirmYesNoMock = mock<() => Promise<boolean>>();

mock.module("../../src/git.js", async () => {
  const actual = await import("../../src/git.js");
  return {
    ...actual,
    getRepositoryRoot: getRepositoryRootMock,
    fetchAllRemotes: fetchAllRemotesMock,
    pullFastForward: pullFastForwardMock,
    getBranchDivergenceStatuses: getBranchDivergenceStatusesMock,
    branchExists: mock(async () => true),
    hasUncommittedChanges: hasUncommittedChangesMock,
    hasUnpushedCommits: hasUnpushedCommitsMock,
    getUncommittedChangesCount: getUncommittedChangesCountMock,
    getUnpushedCommitsCount: getUnpushedCommitsCountMock,
    pushBranchToRemote: pushBranchToRemoteMock,
  };
});

mock.module("../../src/worktree.js", async () => {
  const actual = await import("../../src/worktree.js");
  return {
    ...actual,
    worktreeExists: worktreeExistsMock,
    resolveWorktreePathForBranch: mock(async (branch: string) => ({
      path: await worktreeExistsMock(branch),
    })),
    isProtectedBranchName: mock(() => false),
    switchToProtectedBranch: mock(async () => "none" as const),
  };
});

mock.module("../../src/services/WorktreeOrchestrator.js", () => ({
  WorktreeOrchestrator: class {
    ensureWorktree = ensureWorktreeMock;
  },
}));

const DependencyInstallErrorMock = () =>
  class DependencyInstallError extends Error {
    constructor(message?: string) {
      super(message);
      this.name = "DependencyInstallError";
    }
  };

mock.module("../../src/services/dependency-installer.js", () => ({
  installDependenciesForWorktree: installDependenciesMock,
  DependencyInstallError: DependencyInstallErrorMock,
}));

mock.module("../../src/config/tools.js", () => ({
  getCodingAgentById: getCodingAgentByIdMock,
  getSharedEnvironment: getSharedEnvironmentMock,
}));

mock.module("../../src/config/index.js", () => ({
  saveSession: saveSessionMock,
  loadSession: loadSessionMock,
}));

mock.module("../../src/codex.js", () => ({
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

mock.module("../../src/utils/session.js", () => ({
  findLatestCodexSession: findLatestCodexSessionMock,
  findLatestClaudeSession: mock(async () => null),
  findLatestGeminiSession: mock(async () => null),
  findLatestClaudeSessionId: mock(async () => null),
}));

mock.module("../../src/utils/terminal.js", async () => {
  const actual = await import("../../src/utils/terminal.js");
  return {
    ...actual,
    waitForUserAcknowledgement: waitForUserAcknowledgementMock,
  };
});

mock.module("../../src/utils/prompt.js", async () => {
  const actual = await import("../../src/utils/prompt.js");
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
  getCodingAgentByIdMock.mockClear();
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

    const run = handleAIToolWorkflow(selection);
    await new Promise((r) => setTimeout(r, 3000));
    await run;

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

    const run = handleAIToolWorkflow(selection);
    await new Promise((r) => setTimeout(r, 3000));
    await run;

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
