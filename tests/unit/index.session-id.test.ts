import { beforeEach, describe, expect, it, mock } from "bun:test";
import type { SelectionResult } from "../../src/cli/ui/App.solid.js";

const {
  ensureWorktreeMock,
  fetchAllRemotesMock,
  pullFastForwardMock,
  getBranchDivergenceStatusesMock,
  launchCodexCLIMock,
  saveSessionMock,
  loadSessionMock,
  worktreeExistsMock,
  getRepositoryRootMock,
  installDependenciesMock,
  findLatestCodexSessionMock,
  hasUncommittedChangesMock,
  hasUnpushedCommitsMock,
  getUncommittedChangesCountMock,
  getUnpushedCommitsCountMock,
  pushBranchToRemoteMock,
} = {
  ensureWorktreeMock: mock(async () => "/repo/.worktrees/feature"),
  fetchAllRemotesMock: mock(async () => undefined),
  pullFastForwardMock: mock(async () => undefined),
  getBranchDivergenceStatusesMock: mock(async () => []),
  launchCodexCLIMock: mock(async () => ({ sessionId: null as string | null })),
  saveSessionMock: mock<(...args: unknown[]) => Promise<void>>(
    async () => undefined,
  ),
  loadSessionMock: mock(async () => null),
  worktreeExistsMock: mock(async (_branch: string) => null),
  getRepositoryRootMock: mock(async () => "/repo"),
  installDependenciesMock: mock(async () => ({
    skipped: true as const,
    manager: null,
    lockfile: null,
    reason: "missing-lockfile" as const,
  })),
  findLatestCodexSessionMock: mock(async () => ({
    id: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
    mtime: Date.now(),
  })),
  hasUncommittedChangesMock: mock(async () => false),
  hasUnpushedCommitsMock: mock(async () => false),
  getUncommittedChangesCountMock: mock(async () => 0),
  getUnpushedCommitsCountMock: mock(async () => 0),
  pushBranchToRemoteMock: mock(async () => undefined),
};

const confirmYesNoMock = mock<() => Promise<boolean>>();
const waitForUserAcknowledgementMock = mock<() => Promise<void>>(
  async () => undefined,
);
const writeTerminalMock = mock();
const writeTerminalLineMock = mock();
const terminalStreamsMock = {
  stdin: process.stdin,
  stdout: { write: writeTerminalMock } as NodeJS.WriteStream,
  stderr: { write: writeTerminalMock } as NodeJS.WriteStream,
  stdinFd: undefined as number | undefined,
  stdoutFd: undefined as number | undefined,
  stderrFd: undefined as number | undefined,
  usingFallback: false,
  exitRawMode: mock(),
};
const mockChildStdio = {
  stdin: "inherit" as const,
  stdout: "inherit" as const,
  stderr: "inherit" as const,
  cleanup: mock(),
};
mock.module("../../src/git.js", () => ({
  isGitRepository: mock(async () => true),
  getRepositoryRoot: getRepositoryRootMock,
  fetchAllRemotes: fetchAllRemotesMock,
  pullFastForward: pullFastForwardMock,
  getBranchDivergenceStatuses: getBranchDivergenceStatusesMock,
  branchExists: mock(async () => true),
  getCurrentBranch: mock(async () => "develop"),
  hasUncommittedChanges: hasUncommittedChangesMock,
  hasUnpushedCommits: hasUnpushedCommitsMock,
  getUncommittedChangesCount: getUncommittedChangesCountMock,
  getUnpushedCommitsCount: getUnpushedCommitsCountMock,
  pushBranchToRemote: pushBranchToRemoteMock,
  GitError: class GitError extends Error {
    constructor(
      message: string,
      public cause?: unknown,
    ) {
      super(message);
      this.name = "GitError";
    }
  },
}));

mock.module("../../src/worktree.js", () => ({
  worktreeExists: worktreeExistsMock,
  resolveWorktreePathForBranch: mock(async (branch: string) => ({
    path: await worktreeExistsMock(branch),
  })),
  isProtectedBranchName: mock(() => false),
  switchToProtectedBranch: mock(),
  listAllWorktrees: mock(async () => []),
  listAdditionalWorktrees: mock(async () => []),
  generateWorktreePath: mock(async () => "/repo/.worktrees/feature"),
  createWorktree: mock(async () => undefined),
  WorktreeError: class WorktreeError extends Error {
    constructor(
      message: string,
      public cause?: unknown,
    ) {
      super(message);
      this.name = "WorktreeError";
    }
  },
}));

mock.module("../../src/services/WorktreeOrchestrator.js", () => ({
  WorktreeOrchestrator: class {
    ensureWorktree = ensureWorktreeMock;
  },
}));

mock.module("../../src/services/dependency-installer.js", () => ({
  installDependenciesForWorktree: installDependenciesMock,
  DependencyInstallError: class DependencyInstallError extends Error {
    constructor(
      message?: string,
      public cause?: unknown,
    ) {
      super(message);
      this.name = "DependencyInstallError";
    }
  },
}));

mock.module("../../src/config/tools.js", () => ({
  getCodingAgentById: mock(async () => ({
    id: "codex-cli",
    displayName: "Codex",
    type: "command",
    command: "codex",
    modeArgs: { normal: [] },
  })),
  getSharedEnvironment: mock(async () => ({})),
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
  findLatestGeminiSessionId: mock(async () => null),
  findLatestClaudeSessionId: mock(async () => null),
}));

mock.module("../../src/utils/prompt.js", () => ({
  confirmYesNo: confirmYesNoMock,
}));

mock.module("../../src/utils/terminal.js", () => ({
  getTerminalStreams: mock(() => terminalStreamsMock),
  resetTerminalModes: mock(),
  waitForUserAcknowledgement: waitForUserAcknowledgementMock,
  writeTerminalLine: writeTerminalLineMock,
  createChildStdio: mock(() => mockChildStdio),
}));
let handleAIToolWorkflow: typeof import("../../src/index.js").handleAIToolWorkflow;

beforeEach(async () => {
  ensureWorktreeMock.mockClear();
  fetchAllRemotesMock.mockClear();
  pullFastForwardMock.mockClear();
  getBranchDivergenceStatusesMock.mockClear();
  launchCodexCLIMock.mockClear();
  saveSessionMock.mockClear();
  loadSessionMock.mockClear();
  worktreeExistsMock.mockClear();
  getRepositoryRootMock.mockClear();
  installDependenciesMock.mockClear();
  findLatestCodexSessionMock.mockClear();
  hasUncommittedChangesMock.mockClear();
  hasUnpushedCommitsMock.mockClear();
  getUncommittedChangesCountMock.mockClear();
  getUnpushedCommitsCountMock.mockClear();
  pushBranchToRemoteMock.mockClear();

  confirmYesNoMock.mockClear();
  confirmYesNoMock.mockResolvedValue(false);

  waitForUserAcknowledgementMock.mockClear();
  waitForUserAcknowledgementMock.mockResolvedValue(undefined);
  writeTerminalMock.mockClear();
  writeTerminalLineMock.mockClear();

  getBranchDivergenceStatusesMock.mockResolvedValue([]);
  worktreeExistsMock.mockResolvedValue(null);
  getRepositoryRootMock.mockResolvedValue("/repo");
  hasUncommittedChangesMock.mockResolvedValue(false);
  hasUnpushedCommitsMock.mockResolvedValue(false);
  getUncommittedChangesCountMock.mockResolvedValue(0);
  getUnpushedCommitsCountMock.mockResolvedValue(0);
  pushBranchToRemoteMock.mockResolvedValue(undefined);

  ({ handleAIToolWorkflow } = await import("../../src/index.js"));
});

describe("handleAIToolWorkflow - session ID persistence", () => {
  it("does not overwrite explicit sessionId with on-disk detection", async () => {
    const explicit = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    launchCodexCLIMock.mockResolvedValueOnce({ sessionId: explicit });

    const selection: SelectionResult = {
      branch: "feature/test",
      displayName: "feature/test",
      branchType: "local",
      tool: "codex-cli",
      mode: "resume",
      skipPermissions: false,
      sessionId: explicit,
    };

    await handleAIToolWorkflow(selection);

    const calls = saveSessionMock.mock.calls;
    const lastCall = calls[calls.length - 1];
    if (!lastCall) {
      throw new Error("Expected session save call");
    }
    const lastSaved = lastCall[0] as { lastSessionId?: string | null };

    expect(lastSaved.lastSessionId).toBe(explicit);
    expect(findLatestCodexSessionMock).not.toHaveBeenCalled();
  });
});
