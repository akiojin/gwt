import { beforeEach, describe, expect, it, mock, spyOn } from "bun:test";
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
  findLatestCodexSessionMock: mock(async () => null),
  hasUncommittedChangesMock: mock(async () => false),
  hasUnpushedCommitsMock: mock(async () => false),
  getUncommittedChangesCountMock: mock(async () => 0),
  getUnpushedCommitsCountMock: mock(async () => 0),
  pushBranchToRemoteMock: mock(async () => undefined),
  confirmYesNoMock: mock(async () => false),
  waitForEnterMock: mock(async () => undefined),
  resolveWorktreePathForBranchMock: mock(async () => ({ path: null })),
};

mock.module("../../src/git.js", async () => {
  const actual = await import("../../src/git.js");
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
  const actual = await import("../../src/worktree.js");
  return {
    ...actual,
    worktreeExists: worktreeExistsMock,
    resolveWorktreePathForBranch: resolveWorktreePathForBranchMock,
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
  const actual = await import("../../src/services/dependency-installer.js");
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
  const actual = await import("../../src/utils/prompt.js");
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
    // TODO: use setSystemTime for fake timers in bun;
    const warnSpy = spyOn(console, "warn").mockImplementation(() => {});

    hasUncommittedChangesMock.mockResolvedValue(true);
    getUncommittedChangesCountMock.mockResolvedValue(2);

    const run = handleAIToolWorkflow(selection);
    await new Promise((r) => setTimeout(r, 3000));
    await run;

    const messages = warnSpy.mock.calls.flat().join(" ");
    expect(messages).toContain("Uncommitted changes detected");
    expect(waitForEnterMock).not.toHaveBeenCalled();
    expect(confirmYesNoMock).not.toHaveBeenCalled();
    expect(pushBranchToRemoteMock).not.toHaveBeenCalled();

    warnSpy.mockRestore();
    // TODO: restore real timers;
  });

  it("warns when unpushed commits exist and waits 3 seconds", async () => {
    // TODO: use setSystemTime for fake timers in bun;
    const warnSpy = spyOn(console, "warn").mockImplementation(() => {});

    hasUnpushedCommitsMock.mockResolvedValue(true);
    getUnpushedCommitsCountMock.mockResolvedValue(3);

    const run = handleAIToolWorkflow(selection);
    await new Promise((r) => setTimeout(r, 3000));
    await run;

    const messages = warnSpy.mock.calls.flat().join(" ");
    expect(messages).toContain("Unpushed commits detected");
    expect(waitForEnterMock).not.toHaveBeenCalled();
    expect(confirmYesNoMock).not.toHaveBeenCalled();
    expect(pushBranchToRemoteMock).not.toHaveBeenCalled();

    warnSpy.mockRestore();
    // TODO: restore real timers;
  });

  it("warns for both uncommitted and unpushed changes before waiting", async () => {
    // TODO: use setSystemTime for fake timers in bun;
    const warnSpy = spyOn(console, "warn").mockImplementation(() => {});

    hasUncommittedChangesMock.mockResolvedValue(true);
    hasUnpushedCommitsMock.mockResolvedValue(true);
    getUncommittedChangesCountMock.mockResolvedValue(1);
    getUnpushedCommitsCountMock.mockResolvedValue(1);

    const run = handleAIToolWorkflow(selection);
    await new Promise((r) => setTimeout(r, 3000));
    await run;

    const messages = warnSpy.mock.calls.flat().join(" ");
    expect(messages).toContain("Uncommitted changes detected");
    expect(messages).toContain("Unpushed commits detected");
    expect(waitForEnterMock).not.toHaveBeenCalled();
    expect(confirmYesNoMock).not.toHaveBeenCalled();
    expect(pushBranchToRemoteMock).not.toHaveBeenCalled();

    warnSpy.mockRestore();
    // TODO: restore real timers;
  });

  it("uses 3-second delay when no uncommitted or unpushed changes exist", async () => {
    // TODO: use setSystemTime for fake timers in bun;

    hasUncommittedChangesMock.mockResolvedValue(false);
    hasUnpushedCommitsMock.mockResolvedValue(false);

    const run = handleAIToolWorkflow(selection);
    await new Promise((r) => setTimeout(r, 3000));
    await run;

    expect(waitForEnterMock).not.toHaveBeenCalled();
    expect(confirmYesNoMock).not.toHaveBeenCalled();
    expect(pushBranchToRemoteMock).not.toHaveBeenCalled();

    // TODO: restore real timers;
  });
});
