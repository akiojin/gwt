import { beforeEach, describe, expect, it, mock, spyOn } from "bun:test";
import type { SelectionResult } from "../../src/cli/ui/App.solid.js";
import type { ExecutionMode } from "../../src/cli/ui/App.solid.js";

const {
  execaMock,
  ensureWorktreeMock,
  fetchAllRemotesMock,
  pullFastForwardMock,
  getBranchDivergenceStatusesMock,
  launchClaudeCodeMock,
  saveSessionMock,
  worktreeExistsMock,
  switchToProtectedBranchMock,
  branchExistsMock,
  getRepositoryRootMock,
  getCurrentBranchMock,
  installDependenciesMock,
  hasUncommittedChangesMock,
  hasUnpushedCommitsMock,
  getUncommittedChangesCountMock,
  getUnpushedCommitsCountMock,
  pushBranchToRemoteMock,
} = (({
  execaMock: mock(async () => ({ stdout: "" })),
  ensureWorktreeMock: mock(async () => "/repo"),
  fetchAllRemotesMock: mock(async () => undefined),
  pullFastForwardMock: mock(async () => undefined),
  getBranchDivergenceStatusesMock: mock(async () => []),
  launchClaudeCodeMock: mock(async () => undefined),
  saveSessionMock: mock(async () => undefined),
  worktreeExistsMock: mock(async () => null),
  switchToProtectedBranchMock: mock(async () => "local" as const),
  branchExistsMock: mock(async () => true),
  getRepositoryRootMock: mock(async () => "/repo"),
  getCurrentBranchMock: mock(async () => "develop"),
  installDependenciesMock: mock(async () => ({
    skipped: false as const,
    manager: "bun" as const,
    lockfile: "/repo/bun.lock",
  })),
  hasUncommittedChangesMock: mock(async () => false),
  hasUnpushedCommitsMock: mock(async () => false),
  getUncommittedChangesCountMock: mock(async () => 0),
  getUnpushedCommitsCountMock: mock(async () => 0),
  pushBranchToRemoteMock: mock(async () => undefined),
}));

const DependencyInstallErrorMock = (
  () =>
    class extends Error {
      constructor(message?: string) {
        super(message);
        this.name = "DependencyInstallError";
      }
    },
);

const waitForUserAcknowledgementMock = (
  (mock<() => Promise<void>>()),
);

const waitForEnterMock = ((mock<() => Promise<void>>()));

const confirmYesNoMock = ((mock<() => Promise<boolean>>()));
mock.module("execa", () => ({
  execa: execaMock,
}));

mock.module("../../src/git.js", async () => {
  const actual =
    await import("../../src/git.js
      "../../src/git.js",
    );
  return {
    isGitRepository: mock(),
    getRepositoryRoot: getRepositoryRootMock,
    branchExists: branchExistsMock,
    fetchAllRemotes: fetchAllRemotesMock,
    pullFastForward: pullFastForwardMock,
    getBranchDivergenceStatuses: getBranchDivergenceStatusesMock,
    getCurrentBranch: getCurrentBranchMock,
    hasUncommittedChanges: hasUncommittedChangesMock,
    hasUnpushedCommits: hasUnpushedCommitsMock,
    getUncommittedChangesCount: getUncommittedChangesCountMock,
    getUnpushedCommitsCount: getUnpushedCommitsCountMock,
    pushBranchToRemote: pushBranchToRemoteMock,
    GitError: actual.GitError,
  };
});

mock.module("../../src/worktree.js", async () => {
  const actual = await import("../../src/worktree.js
    "../../src/worktree.js",
  );
  return {
    worktreeExists: worktreeExistsMock,
    resolveWorktreePathForBranch: mock(async (branch: string) => ({
      path: await worktreeExistsMock(branch),
    })),
    isProtectedBranchName: (name: string) =>
      name === "main" || name === "origin/main",
    switchToProtectedBranch: switchToProtectedBranchMock,
    WorktreeError: actual.WorktreeError,
  };
});

mock.module("../../src/services/WorktreeOrchestrator.js", () => ({
  WorktreeOrchestrator: class {
    ensureWorktree = ensureWorktreeMock;
  },
}));

mock.module("../../src/services/dependency-installer.js", () => ({
  installDependenciesForWorktree: installDependenciesMock,
  DependencyInstallError: DependencyInstallErrorMock,
}));

mock.module("../../src/claude.js", () => ({
  launchClaudeCode: launchClaudeCodeMock,
}));

mock.module("../../src/codex.js", () => ({
  launchCodexCLI: mock(async () => undefined),
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

mock.module("../../src/launcher.js", () => ({
  launchCodingAgent: mock(async () => undefined),
}));

mock.module("../../src/config/tools.js", () => ({
  getCodingAgentById: mock(async () => ({
    id: "claude-code",
    displayName: "Claude Code",
    type: "bunx",
    command: "@anthropic-ai/claude-code@latest",
    modeArgs: { normal: [], continue: ["-c"], resume: ["-r"] },
  })),
  getSharedEnvironment: mock(async () => ({})),
}));

mock.module("../../src/config/index.js", () => ({
  saveSession: saveSessionMock,
}));

mock.module("../../src/utils/terminal.js", async () => {
  const actual = await import(
    typeof import("../../src/utils/terminal.js")
  >("../../src/utils/terminal.js");
  return {
    ...actual,
    waitForUserAcknowledgement: waitForUserAcknowledgementMock,
  };
});

mock.module("../../src/utils/prompt.js", async () => {
  const actual = await import(
    typeof import("../../src/utils/prompt.js")
  >("../../src/utils/prompt.js");
  return {
    ...actual,
    waitForEnter: waitForEnterMock,
    confirmYesNo: confirmYesNoMock,
  };
});

// Import after mocks are set up
import { handleAIToolWorkflow } from "../../src/index.js";

beforeEach(() => {
  execaMock.mockClear();
  ensureWorktreeMock.mockClear();
  fetchAllRemotesMock.mockClear();
  pullFastForwardMock.mockClear();
  getBranchDivergenceStatusesMock.mockClear();
  getBranchDivergenceStatusesMock.mockResolvedValue([]);
  launchClaudeCodeMock.mockClear();
  saveSessionMock.mockClear();
  worktreeExistsMock.mockClear();
  branchExistsMock.mockClear();
  getRepositoryRootMock.mockClear();
  getCurrentBranchMock.mockClear();
  switchToProtectedBranchMock.mockClear();
  installDependenciesMock.mockClear();
  hasUncommittedChangesMock.mockClear();
  hasUnpushedCommitsMock.mockClear();
  getUncommittedChangesCountMock.mockClear();
  getUnpushedCommitsCountMock.mockClear();
  pushBranchToRemoteMock.mockClear();
  installDependenciesMock.mockResolvedValue({
    skipped: false,
    manager: "bun",
    lockfile: "/repo/bun.lock",
  });
  hasUncommittedChangesMock.mockResolvedValue(false);
  hasUnpushedCommitsMock.mockResolvedValue(false);
  getUncommittedChangesCountMock.mockResolvedValue(0);
  getUnpushedCommitsCountMock.mockResolvedValue(0);
  pushBranchToRemoteMock.mockResolvedValue(undefined);
  waitForUserAcknowledgementMock.mockClear();
  waitForUserAcknowledgementMock.mockResolvedValue(undefined);
  waitForEnterMock.mockClear();
  waitForEnterMock.mockResolvedValue(undefined);
  confirmYesNoMock.mockClear();
  confirmYesNoMock.mockResolvedValue(false);
  switchToProtectedBranchMock.mockResolvedValue("local");
  branchExistsMock.mockResolvedValue(true);
  getCurrentBranchMock.mockResolvedValue("develop");
});

describe("handleAIToolWorkflow - protected branches", () => {
  it("checks out protected branch in repository root instead of creating worktree", async () => {
    const selection: SelectionResult = {
      branch: "main",
      displayName: "main",
      branchType: "local",
      tool: "claude-code",
      mode: "normal" as ExecutionMode,
      skipPermissions: true,
      model: "sonnet",
    };
    await handleAIToolWorkflow(selection);

    expect(switchToProtectedBranchMock).toHaveBeenCalledWith({
      branchName: "main",
      repoRoot: "/repo",
      remoteRef: null,
    });
    expect(ensureWorktreeMock).toHaveBeenCalledWith(
      "main",
      "/repo",
      expect.objectContaining({
        isNewBranch: false,
      }),
    );
    expect(fetchAllRemotesMock).toHaveBeenCalled();
    expect(pullFastForwardMock).toHaveBeenCalledWith("/repo");
    expect(launchClaudeCodeMock).toHaveBeenCalled();
    expect(saveSessionMock).toHaveBeenCalled();
  });

  it("creates local tracking branch when only remote protected branch exists", async () => {
    branchExistsMock.mockResolvedValue(false);
    getCurrentBranchMock.mockResolvedValue("develop");
    switchToProtectedBranchMock.mockResolvedValueOnce("remote");

    const selection: SelectionResult = {
      branch: "main",
      displayName: "origin/main",
      branchType: "remote",
      remoteBranch: "origin/main",
      tool: "claude-code",
      mode: "normal" as ExecutionMode,
      skipPermissions: false,
      model: "sonnet",
    };
    await handleAIToolWorkflow(selection);

    expect(switchToProtectedBranchMock).toHaveBeenCalledWith({
      branchName: "main",
      repoRoot: "/repo",
      remoteRef: "origin/main",
    });
    expect(ensureWorktreeMock).toHaveBeenCalledWith(
      "main",
      "/repo",
      expect.objectContaining({
        baseBranch: "origin/main",
        isNewBranch: false,
      }),
    );
  });
});

describe("handleAIToolWorkflow - divergence handling", () => {
  it("continues AI tool launch when divergence is detected", async () => {
    getBranchDivergenceStatusesMock.mockResolvedValue([
      { branch: "feature/diverged", remoteAhead: 7, localAhead: 2 },
    ]);

    const selection: SelectionResult = {
      branch: "feature/diverged",
      displayName: "feature/diverged",
      branchType: "local",
      tool: "claude-code",
      mode: "normal" as ExecutionMode,
      skipPermissions: false,
      model: "sonnet",
    };

    await handleAIToolWorkflow(selection);

    expect(fetchAllRemotesMock).toHaveBeenCalled();
    expect(pullFastForwardMock).toHaveBeenCalledWith("/repo");
    expect(launchClaudeCodeMock).toHaveBeenCalled();
    expect(saveSessionMock).toHaveBeenCalled();
  });

  it("continues AI tool launch when divergence check fails", async () => {
    getBranchDivergenceStatusesMock.mockRejectedValueOnce(
      new Error("divergence check failed"),
    );

    const selection: SelectionResult = {
      branch: "feature/divergence-error",
      displayName: "feature/divergence-error",
      branchType: "local",
      tool: "claude-code",
      mode: "normal" as ExecutionMode,
      skipPermissions: false,
      model: "sonnet",
    };

    await handleAIToolWorkflow(selection);

    expect(fetchAllRemotesMock).toHaveBeenCalled();
    expect(pullFastForwardMock).toHaveBeenCalledWith("/repo");
    expect(launchClaudeCodeMock).toHaveBeenCalled();
    expect(saveSessionMock).toHaveBeenCalled();
    expect(waitForUserAcknowledgementMock).not.toHaveBeenCalled();
  });
});

describe("handleAIToolWorkflow - git failure tolerance", () => {
  it("aborts workflow gracefully when fetchAllRemotes fails", async () => {
    const gitError = Object.assign(new Error("fetch failed"), {
      name: "GitError",
    });
    fetchAllRemotesMock.mockRejectedValueOnce(gitError);

    const selection: SelectionResult = {
      branch: "feature/network-issue",
      displayName: "feature/network-issue",
      branchType: "local",
      tool: "claude-code",
      mode: "normal" as ExecutionMode,
      skipPermissions: false,
      model: "sonnet",
    };

    await handleAIToolWorkflow(selection);

    expect(fetchAllRemotesMock).toHaveBeenCalled();
    expect(launchClaudeCodeMock).not.toHaveBeenCalled();
    expect(saveSessionMock).not.toHaveBeenCalled();
    expect(waitForUserAcknowledgementMock).toHaveBeenCalledTimes(1);
  });
});

describe("handleAIToolWorkflow - dependency installation", () => {
  it("installs dependencies before launching the AI tool", async () => {
    const selection: SelectionResult = {
      branch: "feature/test",
      displayName: "feature/test",
      branchType: "local",
      tool: "claude-code",
      mode: "normal" as ExecutionMode,
      skipPermissions: true,
      model: "sonnet",
    };

    await handleAIToolWorkflow(selection);
    expect(installDependenciesMock).toHaveBeenCalledWith("/repo");
  });

  it("prompts the user but continues when install fails", async () => {
    installDependenciesMock.mockRejectedValueOnce(
      new DependencyInstallErrorMock("install failed"),
    );

    const selection: SelectionResult = {
      branch: "feature/test",
      displayName: "feature/test",
      branchType: "local",
      tool: "claude-code",
      mode: "normal" as ExecutionMode,
      skipPermissions: false,
      model: "sonnet",
    };

    await handleAIToolWorkflow(selection);

    expect(waitForUserAcknowledgementMock).toHaveBeenCalled();
    expect(launchClaudeCodeMock).toHaveBeenCalled();
    expect(saveSessionMock).toHaveBeenCalled();
  });

  it("continues when dependency install is skipped due to missing binary", async () => {
    installDependenciesMock.mockResolvedValueOnce({
      manager: "bun",
      lockfile: "/repo/bun.lock",
      skipped: true,
      reason: "missing-binary",
    });

    const selection: SelectionResult = {
      branch: "feature/test",
      displayName: "feature/test",
      branchType: "local",
      tool: "claude-code",
      mode: "normal" as ExecutionMode,
      skipPermissions: false,
      model: "sonnet",
    };

    await handleAIToolWorkflow(selection);

    expect(installDependenciesMock).toHaveBeenCalled();
    expect(waitForUserAcknowledgementMock).not.toHaveBeenCalled();
  });

  it("warns and continues when dependency metadata is missing", async () => {
    installDependenciesMock.mockResolvedValueOnce({
      manager: null,
      lockfile: null,
      skipped: true,
      reason: "missing-lockfile",
    });

    const warnSpy = spyOn(console, "warn").mockImplementation(() => {});

    const selection: SelectionResult = {
      branch: "feature/test",
      displayName: "feature/test",
      branchType: "local",
      tool: "claude-code",
      mode: "normal" as ExecutionMode,
      skipPermissions: false,
      model: "sonnet",
    };

    await handleAIToolWorkflow(selection);

    expect(installDependenciesMock).toHaveBeenCalled();
    expect(waitForUserAcknowledgementMock).not.toHaveBeenCalled();
    expect(launchClaudeCodeMock).toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining(
        "Skipping automatic install because no lockfiles",
      ),
    );

    warnSpy.mockRestore();
  });

  it("warns with details when dependency installation fails", async () => {
    installDependenciesMock.mockResolvedValueOnce({
      manager: "bun",
      lockfile: "/repo/bun.lock",
      skipped: true,
      reason: "install-failed",
      message: "Dependency installation failed (bun). Command: bun install",
    });

    const warnSpy = spyOn(console, "warn").mockImplementation(() => {});

    const selection: SelectionResult = {
      branch: "feature/test",
      displayName: "feature/test",
      branchType: "local",
      tool: "claude-code",
      mode: "normal" as ExecutionMode,
      skipPermissions: false,
      model: "sonnet",
    };

    await handleAIToolWorkflow(selection);

    expect(installDependenciesMock).toHaveBeenCalled();
    expect(waitForUserAcknowledgementMock).not.toHaveBeenCalled();
    expect(launchClaudeCodeMock).toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining("Dependency installation failed via bun"),
    );
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining("Details: Dependency installation failed"),
    );

    warnSpy.mockRestore();
  });
});
