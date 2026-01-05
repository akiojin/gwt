import { beforeEach, describe, expect, it, vi } from "vitest";
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
} = vi.hoisted(() => ({
  ensureWorktreeMock: vi.fn(async () => "/repo/.worktrees/feature"),
  fetchAllRemotesMock: vi.fn(async () => undefined),
  pullFastForwardMock: vi.fn(async () => undefined),
  getBranchDivergenceStatusesMock: vi.fn(async () => []),
  launchCodexCLIMock: vi.fn(async () => ({ sessionId: null as string | null })),
  saveSessionMock: vi.fn(async () => undefined),
  loadSessionMock: vi.fn(async () => null),
  worktreeExistsMock: vi.fn(async () => null),
  getRepositoryRootMock: vi.fn(async () => "/repo"),
  installDependenciesMock: vi.fn(async () => ({
    skipped: true as const,
    manager: null,
    lockfile: null,
    reason: "missing-lockfile" as const,
  })),
  findLatestCodexSessionMock: vi.fn(async () => ({
    id: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
    mtime: Date.now(),
  })),
  hasUncommittedChangesMock: vi.fn(async () => false),
  hasUnpushedCommitsMock: vi.fn(async () => false),
  getUncommittedChangesCountMock: vi.fn(async () => 0),
  getUnpushedCommitsCountMock: vi.fn(async () => 0),
  pushBranchToRemoteMock: vi.fn(async () => undefined),
}));

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
    getCurrentBranch: vi.fn(async () => "develop"),
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
    switchToProtectedBranch: vi.fn(),
  };
});

vi.mock("../../src/services/WorktreeOrchestrator.js", () => ({
  WorktreeOrchestrator: class {
    ensureWorktree = ensureWorktreeMock;
  },
}));

vi.mock("../../src/services/dependency-installer.js", async () => {
  const actual = await vi.importActual<
    typeof import("../../src/services/dependency-installer.js")
  >("../../src/services/dependency-installer.js");
  return {
    ...actual,
    installDependenciesForWorktree: installDependenciesMock,
  };
});

vi.mock("../../src/config/tools.js", () => ({
  getToolById: vi.fn(() => ({
    id: "codex-cli",
    displayName: "Codex",
  })),
  getSharedEnvironment: vi.fn(async () => ({})),
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
