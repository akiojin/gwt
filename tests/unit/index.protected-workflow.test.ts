import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SelectionResult } from "../../src/ui/components/App.js";
import type { ExecutionMode } from "../../src/ui/components/screens/ExecutionModeSelectorScreen.js";

const execaMock = vi.fn(async () => ({ stdout: "" }));
const ensureWorktreeMock = vi.fn(async () => "/repo");
const fetchAllRemotesMock = vi.fn(async () => undefined);
const pullFastForwardMock = vi.fn(async () => undefined);
const getBranchDivergenceStatusesMock = vi.fn(async () => []);
const launchClaudeCodeMock = vi.fn(async () => undefined);
const saveSessionMock = vi.fn(async () => undefined);
const worktreeExistsMock = vi.fn(async () => null);
const branchExistsMock = vi.fn(async () => true);
const getRepositoryRootMock = vi.fn(async () => "/repo");
const getCurrentBranchMock = vi.fn(async () => "develop");

vi.mock("execa", () => ({
  execa: execaMock,
}));

vi.mock("../../src/git.js", async () => {
  return {
    isGitRepository: vi.fn(),
    getRepositoryRoot: getRepositoryRootMock,
    branchExists: branchExistsMock,
    fetchAllRemotes: fetchAllRemotesMock,
    pullFastForward: pullFastForwardMock,
    getBranchDivergenceStatuses: getBranchDivergenceStatusesMock,
    getCurrentBranch: getCurrentBranchMock,
  };
});

vi.mock("../../src/worktree.js", () => ({
  worktreeExists: worktreeExistsMock,
  isProtectedBranchName: (name: string) =>
    name === "main" || name === "origin/main",
}));

vi.mock("../../src/services/WorktreeOrchestrator.js", () => ({
  WorktreeOrchestrator: vi.fn().mockImplementation(() => ({
    ensureWorktree: ensureWorktreeMock,
  })),
}));

vi.mock("../../src/claude.js", () => ({
  launchClaudeCode: launchClaudeCodeMock,
}));

vi.mock("../../src/codex.js", () => ({
  launchCodexCLI: vi.fn(async () => undefined),
}));

vi.mock("../../src/launcher.js", () => ({
  launchCustomAITool: vi.fn(async () => undefined),
}));

vi.mock("../../src/config/tools.js", () => ({
  getToolById: vi.fn(() => ({
    id: "claude-code",
    displayName: "Claude Code",
  })),
}));

vi.mock("../../src/config/index.js", () => ({
  saveSession: saveSessionMock,
}));

// Import after mocks are set up
import { handleAIToolWorkflow } from "../../src/index.js";

describe("handleAIToolWorkflow - protected branches", () => {
  beforeEach(() => {
    execaMock.mockClear();
    ensureWorktreeMock.mockClear();
    fetchAllRemotesMock.mockClear();
    pullFastForwardMock.mockClear();
    getBranchDivergenceStatusesMock.mockClear();
    launchClaudeCodeMock.mockClear();
    saveSessionMock.mockClear();
    worktreeExistsMock.mockClear();
    branchExistsMock.mockClear();
    getRepositoryRootMock.mockClear();
    getCurrentBranchMock.mockClear();
    branchExistsMock.mockResolvedValue(true);
    getCurrentBranchMock.mockResolvedValue("develop");
  });

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

    expect(execaMock).toHaveBeenCalledWith("git", ["checkout", "main"], {
      cwd: "/repo",
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

    expect(execaMock).toHaveBeenNthCalledWith(
      1,
      "git",
      ["fetch", "origin", "main"],
      {
        cwd: "/repo",
      },
    );
    expect(execaMock).toHaveBeenNthCalledWith(
      2,
      "git",
      ["checkout", "-b", "main", "origin/main"],
      {
        cwd: "/repo",
      },
    );
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
