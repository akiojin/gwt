import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SelectionResult } from "../../src/ui/components/App.js";
import type { ExecutionMode } from "../../src/ui/components/screens/ExecutionModeSelectorScreen.js";

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
} = vi.hoisted(() => ({
  execaMock: vi.fn(async () => ({ stdout: "" })),
  ensureWorktreeMock: vi.fn(async () => "/repo"),
  fetchAllRemotesMock: vi.fn(async () => undefined),
  pullFastForwardMock: vi.fn(async () => undefined),
  getBranchDivergenceStatusesMock: vi.fn(async () => []),
  launchClaudeCodeMock: vi.fn(async () => undefined),
  saveSessionMock: vi.fn(async () => undefined),
  worktreeExistsMock: vi.fn(async () => null),
  switchToProtectedBranchMock: vi.fn(async () => "local" as const),
  branchExistsMock: vi.fn(async () => true),
  getRepositoryRootMock: vi.fn(async () => "/repo"),
  getCurrentBranchMock: vi.fn(async () => "develop"),
  installDependenciesMock: vi.fn(async () => ({
    skipped: false as const,
    manager: "bun" as const,
    lockfile: "/repo/bun.lock",
  })),
}));

const DependencyInstallErrorMock = vi.hoisted(
  () =>
    class extends Error {
      constructor(message?: string) {
        super(message);
        this.name = "DependencyInstallError";
      }
    },
);

const waitForUserAcknowledgementMock = vi.hoisted(() =>
  vi.fn<() => Promise<void>>(),
);

vi.mock("execa", () => ({
  execa: execaMock,
}));

vi.mock("../../src/git.js", async () => {
  const actual =
    await vi.importActual<typeof import("../../src/git.js")>(
      "../../src/git.js",
    );
  return {
    isGitRepository: vi.fn(),
    getRepositoryRoot: getRepositoryRootMock,
    branchExists: branchExistsMock,
    fetchAllRemotes: fetchAllRemotesMock,
    pullFastForward: pullFastForwardMock,
    getBranchDivergenceStatuses: getBranchDivergenceStatusesMock,
    getCurrentBranch: getCurrentBranchMock,
    GitError: actual.GitError,
  };
});

vi.mock("../../src/worktree.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/worktree.js")>(
    "../../src/worktree.js",
  );
  return {
    worktreeExists: worktreeExistsMock,
    isProtectedBranchName: (name: string) =>
      name === "main" || name === "origin/main",
    switchToProtectedBranch: switchToProtectedBranchMock,
    WorktreeError: actual.WorktreeError,
  };
});

vi.mock("../../src/services/WorktreeOrchestrator.js", () => ({
  WorktreeOrchestrator: class {
    ensureWorktree = ensureWorktreeMock;
  },
}));

vi.mock("../../src/services/dependency-installer.js", () => ({
  installDependenciesForWorktree: installDependenciesMock,
  DependencyInstallError: DependencyInstallErrorMock,
}));

vi.mock("../../src/claude.js", () => ({
  launchClaudeCode: launchClaudeCodeMock,
}));

vi.mock("../../src/codex.js", () => ({
  launchCodexCLI: vi.fn(async () => undefined),
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

vi.mock("../../src/launcher.js", () => ({
  launchCustomAITool: vi.fn(async () => undefined),
}));

vi.mock("../../src/config/tools.js", () => ({
  getToolById: vi.fn(() => ({
    id: "claude-code",
    displayName: "Claude Code",
  })),
  getSharedEnvironment: vi.fn(async () => ({})),
}));

vi.mock("../../src/config/index.js", () => ({
  saveSession: saveSessionMock,
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
  installDependenciesMock.mockResolvedValue({
    skipped: false,
    manager: "bun",
    lockfile: "/repo/bun.lock",
  });
  waitForUserAcknowledgementMock.mockClear();
  waitForUserAcknowledgementMock.mockResolvedValue(undefined);
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
  it("skips AI tool launch when divergence is detected", async () => {
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
    };

    await handleAIToolWorkflow(selection);

    expect(fetchAllRemotesMock).toHaveBeenCalled();
    expect(pullFastForwardMock).toHaveBeenCalledWith("/repo");
    expect(launchClaudeCodeMock).not.toHaveBeenCalled();
    expect(saveSessionMock).not.toHaveBeenCalled();
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

    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    const selection: SelectionResult = {
      branch: "feature/test",
      displayName: "feature/test",
      branchType: "local",
      tool: "claude-code",
      mode: "normal" as ExecutionMode,
      skipPermissions: false,
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

    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    const selection: SelectionResult = {
      branch: "feature/test",
      displayName: "feature/test",
      branchType: "local",
      tool: "claude-code",
      mode: "normal" as ExecutionMode,
      skipPermissions: false,
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
