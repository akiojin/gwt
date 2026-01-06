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
  confirmYesNoMock,
  waitForEnterMock,
  resolveWorktreePathForBranchMock,
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
  findLatestCodexSessionMock: vi.fn(async () => null),
  hasUncommittedChangesMock: vi.fn(async () => false),
  hasUnpushedCommitsMock: vi.fn(async () => false),
  getUncommittedChangesCountMock: vi.fn(async () => 0),
  getUnpushedCommitsCountMock: vi.fn(async () => 0),
  pushBranchToRemoteMock: vi.fn(async () => undefined),
  confirmYesNoMock: vi.fn(async () => false),
  waitForEnterMock: vi.fn(async () => undefined),
  resolveWorktreePathForBranchMock: vi.fn(async () => ({ path: null })),
}));

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
    resolveWorktreePathForBranch: resolveWorktreePathForBranchMock,
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
  getCodingAgentById: vi.fn(async () => ({
    id: "codex-cli",
    displayName: "Codex",
    type: "command",
    command: "codex",
    modeArgs: { normal: [] },
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
    waitForEnter: waitForEnterMock,
  };
});

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
  waitForEnterMock.mockClear();
  resolveWorktreePathForBranchMock.mockClear();

  getBranchDivergenceStatusesMock.mockResolvedValue([]);
  worktreeExistsMock.mockResolvedValue(null);
  getRepositoryRootMock.mockResolvedValue("/repo");
  hasUncommittedChangesMock.mockResolvedValue(false);
  hasUnpushedCommitsMock.mockResolvedValue(false);
  getUncommittedChangesCountMock.mockResolvedValue(0);
  getUnpushedCommitsCountMock.mockResolvedValue(0);
  confirmYesNoMock.mockResolvedValue(false);
  resolveWorktreePathForBranchMock.mockResolvedValue({ path: null });
});

const selection: SelectionResult = {
  branch: "feature/test",
  displayName: "feature/test",
  branchType: "local",
  tool: "codex-cli",
  mode: "normal",
  skipPermissions: false,
};

describe("handleAIToolWorkflow - post session checks", () => {
  it("warns when uncommitted changes exist and waits 3 seconds", async () => {
    vi.useFakeTimers();
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    hasUncommittedChangesMock.mockResolvedValue(true);
    getUncommittedChangesCountMock.mockResolvedValue(2);

    const run = handleAIToolWorkflow(selection);
    await vi.advanceTimersByTimeAsync(3000);
    await run;

    const messages = warnSpy.mock.calls.flat().join(" ");
    expect(messages).toContain("Uncommitted changes detected");
    expect(waitForEnterMock).not.toHaveBeenCalled();
    expect(confirmYesNoMock).not.toHaveBeenCalled();
    expect(pushBranchToRemoteMock).not.toHaveBeenCalled();

    warnSpy.mockRestore();
    vi.useRealTimers();
  });

  it("warns when unpushed commits exist and waits 3 seconds", async () => {
    vi.useFakeTimers();
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    hasUnpushedCommitsMock.mockResolvedValue(true);
    getUnpushedCommitsCountMock.mockResolvedValue(3);

    const run = handleAIToolWorkflow(selection);
    await vi.advanceTimersByTimeAsync(3000);
    await run;

    const messages = warnSpy.mock.calls.flat().join(" ");
    expect(messages).toContain("Unpushed commits detected");
    expect(waitForEnterMock).not.toHaveBeenCalled();
    expect(confirmYesNoMock).not.toHaveBeenCalled();
    expect(pushBranchToRemoteMock).not.toHaveBeenCalled();

    warnSpy.mockRestore();
    vi.useRealTimers();
  });

  it("warns for both uncommitted and unpushed changes before waiting", async () => {
    vi.useFakeTimers();
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    hasUncommittedChangesMock.mockResolvedValue(true);
    hasUnpushedCommitsMock.mockResolvedValue(true);
    getUncommittedChangesCountMock.mockResolvedValue(1);
    getUnpushedCommitsCountMock.mockResolvedValue(1);

    const run = handleAIToolWorkflow(selection);
    await vi.advanceTimersByTimeAsync(3000);
    await run;

    const messages = warnSpy.mock.calls.flat().join(" ");
    expect(messages).toContain("Uncommitted changes detected");
    expect(messages).toContain("Unpushed commits detected");
    expect(waitForEnterMock).not.toHaveBeenCalled();
    expect(confirmYesNoMock).not.toHaveBeenCalled();
    expect(pushBranchToRemoteMock).not.toHaveBeenCalled();

    warnSpy.mockRestore();
    vi.useRealTimers();
  });

  it("uses 3-second delay when no uncommitted or unpushed changes exist", async () => {
    vi.useFakeTimers();

    hasUncommittedChangesMock.mockResolvedValue(false);
    hasUnpushedCommitsMock.mockResolvedValue(false);

    const run = handleAIToolWorkflow(selection);
    await vi.advanceTimersByTimeAsync(3000);
    await run;

    expect(waitForEnterMock).not.toHaveBeenCalled();
    expect(confirmYesNoMock).not.toHaveBeenCalled();
    expect(pushBranchToRemoteMock).not.toHaveBeenCalled();

    vi.useRealTimers();
  });
});
