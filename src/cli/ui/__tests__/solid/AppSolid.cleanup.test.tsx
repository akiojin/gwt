/** @jsxImportSource @opentui/solid */
import { describe, expect, it, mock, afterEach } from "bun:test";
import { testRender } from "@opentui/solid";
import type { BranchItem, Statistics } from "../../types.js";

if (!mock.module) {
  mock.module = mock.module.bind(vi);
}

const makeStats = (overrides: Partial<Statistics> = {}): Statistics => ({
  localCount: 0,
  remoteCount: 0,
  worktreeCount: 0,
  changesCount: 0,
  lastUpdated: new Date("2025-01-01T00:00:00Z"),
  ...overrides,
});

const createBranch = (overrides: Partial<BranchItem> = {}): BranchItem => {
  const base: BranchItem = {
    name: "feature/cleanup-target",
    type: "local",
    branchType: "feature",
    isCurrent: false,
    icons: [],
    hasChanges: false,
    label: "feature/cleanup-target",
    value: "feature/cleanup-target",
    worktreeStatus: "active",
    worktree: {
      path: "/tmp/worktree",
      locked: false,
      prunable: false,
      isAccessible: true,
    },
    mergedPR: { number: 1, mergedAt: "2025-01-01T00:00:00Z" },
    hasUnpushedCommits: false,
    syncStatus: "no-upstream",
  };
  return {
    ...base,
    ...overrides,
    worktreeStatus: overrides.worktreeStatus ?? base.worktreeStatus,
  };
};

afterEach(() => {
  mock.restore();
  mock.restore();
});

describe("AppSolid cleanup command", () => {
  it("runs cleanup for selected branches when pressing c", async () => {
    const deleteBranchMock = mock(async () => {});
    const removeWorktreeMock = mock(async () => {});
    const getCleanupStatusMock = mock(async () => [
      {
        worktreePath: "/tmp/worktree",
        branch: "feature/cleanup-target",
        hasUncommittedChanges: false,
        hasUnpushedCommits: false,
        cleanupType: "worktree-and-branch",
        hasRemoteBranch: true,
        hasUniqueCommits: false,
        hasUpstream: true,
        upstream: "origin/feature/cleanup-target",
        isAccessible: true,
        reasons: ["no-diff-with-base"],
      },
    ]);

    mock.module?.("../../../../worktree.js", () => ({
      listAdditionalWorktrees: mock(async () => []),
      repairWorktrees: mock(async () => ({
        repairedCount: 0,
        failedCount: 0,
        failures: [],
      })),
      removeWorktree: removeWorktreeMock,
      getCleanupStatus: getCleanupStatusMock,
      isProtectedBranchName: mock(() => false),
    }));

    mock.module?.("../../../../git.js", () => ({
      getRepositoryRoot: mock(async () => "/repo"),
      getAllBranches: mock(async () => []),
      getLocalBranches: mock(async () => []),
      getCurrentBranch: mock(async () => "main"),
      deleteBranch: deleteBranchMock,
    }));

    mock.module?.("../../../../config/index.js", () => ({
      getConfig: mock(async () => ({ defaultBaseBranch: "main" })),
      getLastToolUsageMap: mock(async () => new Map()),
      loadSession: mock(async () => null),
    }));

    mock.module?.("../../../../config/tools.js", () => ({
      getAllCodingAgents: mock(async () => [
        { id: "codex-cli", displayName: "Codex CLI" },
      ]),
    }));

    mock.module?.("../../../../config/profiles.js", () => ({
      loadProfiles: mock(async () => ({ profiles: {}, activeProfile: null })),
      createProfile: mock(async () => {}),
      updateProfile: mock(async () => {}),
      deleteProfile: mock(async () => {}),
      setActiveProfile: mock(async () => {}),
    }));

    const { AppSolid } = await import("../../App.solid.js");

    const branch = createBranch();
    const stats = makeStats({ localCount: 1, worktreeCount: 1 });

    const testSetup = await testRender(
      () => (
        <AppSolid
          branches={[branch]}
          stats={stats}
          version={null}
          toolStatuses={[]}
        />
      ),
      { width: 80, height: 24 },
    );
    await testSetup.renderOnce();

    try {
      await testSetup.mockInput.typeText(" ");
      await testSetup.renderOnce();

      await testSetup.mockInput.typeText("c");
      await testSetup.renderOnce();
      await new Promise((resolve) => setTimeout(resolve, 0));

      expect(removeWorktreeMock).toHaveBeenCalledWith("/tmp/worktree", false);
      expect(deleteBranchMock).toHaveBeenCalledWith(
        "feature/cleanup-target",
        true,
      );
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("maps cleanup safety indicators from cleanup candidates", async () => {
    const getCleanupStatusMock = mock(async () => [
      {
        worktreePath: "/tmp/safe",
        branch: "feature/safe",
        hasUncommittedChanges: false,
        hasUnpushedCommits: false,
        cleanupType: "worktree-and-branch",
        hasRemoteBranch: true,
        hasUniqueCommits: false,
        hasUpstream: true,
        upstream: "origin/feature/safe",
        isAccessible: true,
        reasons: ["no-diff-with-base"],
      },
      {
        worktreePath: null,
        branch: "feature/unsafe",
        hasUncommittedChanges: false,
        hasUnpushedCommits: false,
        cleanupType: "branch-only",
        hasRemoteBranch: false,
        hasUniqueCommits: false,
        hasUpstream: false,
        upstream: null,
        reasons: [],
      },
    ]);

    mock.module?.("../../../../worktree.js", () => ({
      listAdditionalWorktrees: mock(async () => []),
      repairWorktrees: mock(async () => ({
        repairedCount: 0,
        failedCount: 0,
        failures: [],
      })),
      removeWorktree: mock(async () => {}),
      getCleanupStatus: getCleanupStatusMock,
      isProtectedBranchName: mock(() => false),
    }));

    mock.module?.("../../../../git.js", () => ({
      getRepositoryRoot: mock(async () => "/repo"),
      getAllBranches: mock(async () => []),
      getLocalBranches: mock(async () => []),
      getCurrentBranch: mock(async () => "main"),
      deleteBranch: mock(async () => {}),
    }));

    mock.module?.("../../../../config/index.js", () => ({
      getConfig: mock(async () => ({ defaultBaseBranch: "main" })),
      getLastToolUsageMap: mock(async () => new Map()),
      loadSession: mock(async () => null),
    }));

    mock.module?.("../../../../config/tools.js", () => ({
      getAllCodingAgents: mock(async () => [
        { id: "codex-cli", displayName: "Codex CLI" },
      ]),
    }));

    mock.module?.("../../../../config/profiles.js", () => ({
      loadProfiles: mock(async () => ({ profiles: {}, activeProfile: null })),
      createProfile: mock(async () => {}),
      updateProfile: mock(async () => {}),
      deleteProfile: mock(async () => {}),
      setActiveProfile: mock(async () => {}),
    }));

    const { AppSolid } = await import("../../App.solid.js");

    const safeBranch = createBranch({
      name: "feature/safe",
      label: "feature/safe",
      value: "feature/safe",
    });
    const unsafeBranch = createBranch({
      name: "feature/unsafe",
      label: "feature/unsafe",
      value: "feature/unsafe",
      worktree: undefined,
      worktreeStatus: undefined,
    });

    const testSetup = await testRender(
      () => (
        <AppSolid
          branches={[safeBranch, unsafeBranch]}
          stats={makeStats({ localCount: 2, worktreeCount: 1 })}
          version={null}
          toolStatuses={[]}
        />
      ),
      { width: 80, height: 24 },
    );
    await testSetup.renderOnce();
    await new Promise((resolve) => setTimeout(resolve, 0));
    await testSetup.renderOnce();

    try {
      const frame = testSetup.captureCharFrame();
      expect(frame).toMatch(/\[ \] w o feature\/safe/);
      expect(frame).toContain("[ ] w ! feature/unsafe");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});
