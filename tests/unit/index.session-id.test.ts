import { beforeEach, describe, expect, it,  mock } from "bun:test";
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
} = (({
  ensureWorktreeMock: mock(async () => "/repo/.worktrees/feature"),
  fetchAllRemotesMock: mock(async () => undefined),
  pullFastForwardMock: mock(async () => undefined),
  getBranchDivergenceStatusesMock: mock(async () => []),
  launchCodexCLIMock: mock(async () => ({ sessionId: null as string | null })),
  saveSessionMock: mock(async () => undefined),
  loadSessionMock: mock(async () => null),
  worktreeExistsMock: mock(async () => null),
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
}));

const confirmYesNoMock = ((mock<() => Promise<boolean>>()));
mock.module("../../src/git.js", async () => {
  const actual =
    await import("../../src/git.js
      "../../src/git.js",
    );
  return {
    ...actual,
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
  };
});

mock.module("../../src/worktree.js", async () => {
  const actual = await import("../../src/worktree.js
    "../../src/worktree.js",
  );
  return {
    ...actual,
    worktreeExists: worktreeExistsMock,
    resolveWorktreePathForBranch: mock(async (branch: string) => ({
      path: await worktreeExistsMock(branch),
    })),
    isProtectedBranchName: mock(() => false),
    switchToProtectedBranch: mock(),
  };
});

mock.module("../../src/services/WorktreeOrchestrator.js", () => ({
  WorktreeOrchestrator: class {
    ensureWorktree = ensureWorktreeMock;
  },
}));

mock.module("../../src/services/dependency-installer.js", async () => {
  const actual = await import(
    typeof import("../../src/services/dependency-installer.js")
  >("../../src/services/dependency-installer.js");
  return {
    ...actual,
    installDependenciesForWorktree: installDependenciesMock,
  };
});

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
  findLatestClaudeSessionId: mock(async () => null),
}));

mock.module("../../src/utils/prompt.js", async () => {
  const actual = await import(
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

  getBranchDivergenceStatusesMock.mockResolvedValue([]);
  worktreeExistsMock.mockResolvedValue(null);
  getRepositoryRootMock.mockResolvedValue("/repo");
  hasUncommittedChangesMock.mockResolvedValue(false);
  hasUnpushedCommitsMock.mockResolvedValue(false);
  getUncommittedChangesCountMock.mockResolvedValue(0);
  getUnpushedCommitsCountMock.mockResolvedValue(0);
  pushBranchToRemoteMock.mockResolvedValue(undefined);
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

    const lastSaved = saveSessionMock.mock.calls.at(-1)?.[0] as
      | { lastSessionId?: string | null }
      | undefined;

    expect(lastSaved?.lastSessionId).toBe(explicit);
    expect(findLatestCodexSessionMock).not.toHaveBeenCalled();
  });
});
