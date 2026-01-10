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

  it("updates safety icons as each branch check completes", async () => {
    let releaseSecond: (() => void) | null = null;
    const progressGate = new Promise<void>((resolve) => {
      releaseSecond = resolve;
    });
    const getCleanupStatusMock = mock(
      async ({
        onProgress,
      }: { onProgress?: (status: { branch: string }) => void } = {}) => {
        const firstStatus = {
          worktreePath: "/tmp/first",
          branch: "feature/first",
          hasUncommittedChanges: false,
          hasUnpushedCommits: false,
          cleanupType: "worktree-and-branch",
          hasRemoteBranch: true,
          hasUniqueCommits: false,
          hasUpstream: true,
          upstream: "origin/feature/first",
          isAccessible: true,
          reasons: ["no-diff-with-base"],
        };
        const secondStatus = {
          worktreePath: "/tmp/second",
          branch: "feature/second",
          hasUncommittedChanges: false,
          hasUnpushedCommits: false,
          cleanupType: "worktree-and-branch",
          hasRemoteBranch: true,
          hasUniqueCommits: true,
          hasUpstream: true,
          upstream: "origin/feature/second",
          isAccessible: true,
          reasons: ["remote-synced"],
        };

        onProgress?.(firstStatus);
        await progressGate;
        onProgress?.(secondStatus);
        return [firstStatus, secondStatus];
      },
    );

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

    const firstBranch = createBranch({
      name: "feature/first",
      label: "feature/first",
      value: "feature/first",
      worktree: { path: "/tmp/first", locked: false, prunable: false },
    });
    const secondBranch = createBranch({
      name: "feature/second",
      label: "feature/second",
      value: "feature/second",
      worktree: { path: "/tmp/second", locked: false, prunable: false },
    });

    const testSetup = await testRender(
      () => (
        <AppSolid
          branches={[firstBranch, secondBranch]}
          stats={makeStats({ localCount: 2, worktreeCount: 2 })}
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
      let frame = testSetup.captureCharFrame();
      expect(frame).toMatch(/\[ \] w o feature\/first/);
      expect(frame).toMatch(/\[ \] w [-\\|/] feature\/second/);

      releaseSecond?.();
      await new Promise((resolve) => setTimeout(resolve, 0));
      await testSetup.renderOnce();

      frame = testSetup.captureCharFrame();
      expect(frame).toMatch(/\[ \] w \* feature\/second/);
    } finally {
      testSetup.renderer.destroy();
    }
  });
});

describe("AppSolid unsafe selection confirm", () => {
  it("shows confirm and cancels selection on Cancel", async () => {
    const getCleanupStatusMock = mock(async () => [
      {
        worktreePath: "/tmp/worktree",
        branch: "feature/unsafe",
        hasUncommittedChanges: false,
        hasUnpushedCommits: true,
        cleanupType: "worktree-and-branch",
        hasRemoteBranch: true,
        hasUniqueCommits: false,
        hasUpstream: true,
        upstream: "origin/feature/unsafe",
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

    const branch = createBranch({
      name: "feature/unsafe",
      label: "feature/unsafe",
      value: "feature/unsafe",
    });
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
    await new Promise((resolve) => setTimeout(resolve, 0));
    await testSetup.renderOnce();

    try {
      await testSetup.mockInput.typeText(" ");
      await testSetup.renderOnce();

      let frame = testSetup.captureCharFrame();
      expect(frame).toContain("Unsafe branch selected. Select anyway?");
      expect(frame).toContain("OK");
      expect(frame).toContain("Cancel");

      await testSetup.mockInput.typeText("n");
      await testSetup.renderOnce();

      frame = testSetup.captureCharFrame();
      expect(frame).toContain("[ ] w");
      expect(frame).toContain("feature/unsafe");
      expect(frame).not.toContain("Unsafe branch selected. Select anyway?");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("shows confirm when safety check is pending", async () => {
    let resolveStatus: ((value: unknown[]) => void) | null = null;
    const pendingPromise = new Promise<unknown[]>((resolve) => {
      resolveStatus = resolve;
    });
    const getCleanupStatusMock = mock(async () => pendingPromise);

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

    const branch = createBranch({
      name: "feature/pending",
      label: "feature/pending",
      value: "feature/pending",
    });
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
    await new Promise((resolve) => setTimeout(resolve, 0));
    await testSetup.renderOnce();

    try {
      await testSetup.mockInput.typeText(" ");
      await testSetup.renderOnce();

      let frame = testSetup.captureCharFrame();
      expect(frame).toContain("Safety check in progress. Select anyway?");

      await testSetup.mockInput.typeText("n");
      await testSetup.renderOnce();

      frame = testSetup.captureCharFrame();
      expect(frame).toContain("[ ] w");
      expect(frame).toContain("feature/pending");
      expect(frame).not.toContain("Safety check in progress. Select anyway?");
    } finally {
      resolveStatus?.([]);
      testSetup.renderer.destroy();
    }
  });

  it("does not propagate Enter from confirm to branch selection", async () => {
    const getCleanupStatusMock = mock(async () => [
      {
        worktreePath: "/tmp/worktree",
        branch: "feature/unsafe-enter",
        hasUncommittedChanges: false,
        hasUnpushedCommits: true,
        cleanupType: "worktree-and-branch",
        hasRemoteBranch: true,
        hasUniqueCommits: false,
        hasUpstream: true,
        upstream: "origin/feature/unsafe-enter",
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

    const branch = createBranch({
      name: "feature/unsafe-enter",
      label: "feature/unsafe-enter",
      value: "feature/unsafe-enter",
    });
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
    await new Promise((resolve) => setTimeout(resolve, 0));
    await testSetup.renderOnce();

    try {
      await testSetup.mockInput.typeText(" ");
      await testSetup.renderOnce();

      testSetup.mockInput.pressEnter();
      await testSetup.renderOnce();

      const frame = testSetup.captureCharFrame();
      expect(frame).not.toContain("Unsafe branch selected. Select anyway?");
      expect(frame).toContain("[ ] w");
      expect(frame).not.toContain("Open existing worktree");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("selects unsafe branch on OK", async () => {
    const getCleanupStatusMock = mock(async () => [
      {
        worktreePath: "/tmp/worktree",
        branch: "feature/unsafe-ok",
        hasUncommittedChanges: false,
        hasUnpushedCommits: true,
        cleanupType: "worktree-and-branch",
        hasRemoteBranch: true,
        hasUniqueCommits: false,
        hasUpstream: true,
        upstream: "origin/feature/unsafe-ok",
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

    const branch = createBranch({
      name: "feature/unsafe-ok",
      label: "feature/unsafe-ok",
      value: "feature/unsafe-ok",
    });
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
    await new Promise((resolve) => setTimeout(resolve, 0));
    await testSetup.renderOnce();

    try {
      await testSetup.mockInput.typeText(" ");
      await testSetup.renderOnce();

      await testSetup.mockInput.typeText("y");
      await testSetup.renderOnce();

      const frame = testSetup.captureCharFrame();
      expect(frame).toContain("[*]");
      expect(frame).toContain("feature/unsafe-ok");
      expect(frame).not.toContain("Unsafe branch selected. Select anyway?");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});

describe("AppSolid selected cleanup targets", () => {
  it("cleans unsafe branch when confirmed and selected", async () => {
    const deleteBranchMock = mock(async () => {});
    const removeWorktreeMock = mock(async () => {});
    const getCleanupStatusMock = mock(async () => [
      {
        worktreePath: "/tmp/worktree",
        branch: "feature/unsafe-clean",
        hasUncommittedChanges: false,
        hasUnpushedCommits: true,
        cleanupType: "worktree-and-branch",
        hasRemoteBranch: true,
        hasUniqueCommits: false,
        hasUpstream: true,
        upstream: "origin/feature/unsafe-clean",
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

    const branch = createBranch({
      name: "feature/unsafe-clean",
      label: "feature/unsafe-clean",
      value: "feature/unsafe-clean",
    });
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
    await new Promise((resolve) => setTimeout(resolve, 0));
    await testSetup.renderOnce();

    try {
      await testSetup.mockInput.typeText(" ");
      await testSetup.renderOnce();

      await testSetup.mockInput.typeText("y");
      await testSetup.renderOnce();

      await testSetup.mockInput.typeText("c");
      await testSetup.renderOnce();
      await new Promise((resolve) => setTimeout(resolve, 0));

      expect(removeWorktreeMock).toHaveBeenCalledWith("/tmp/worktree", false);
      expect(deleteBranchMock).toHaveBeenCalledWith(
        "feature/unsafe-clean",
        true,
      );
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("includes protected branch when selected", async () => {
    const deleteBranchMock = mock(async () => {});
    const removeWorktreeMock = mock(async () => {});
    const getCleanupStatusMock = mock(async () => [
      {
        worktreePath: "/tmp/worktree",
        branch: "develop",
        hasUncommittedChanges: false,
        hasUnpushedCommits: false,
        cleanupType: "worktree-and-branch",
        hasRemoteBranch: true,
        hasUniqueCommits: false,
        hasUpstream: true,
        upstream: "origin/develop",
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
      isProtectedBranchName: mock(() => true),
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

    const branch = createBranch({
      name: "develop",
      label: "develop",
      value: "develop",
    });
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
    await new Promise((resolve) => setTimeout(resolve, 0));
    await testSetup.renderOnce();

    try {
      await testSetup.mockInput.typeText(" ");
      await testSetup.renderOnce();

      await testSetup.mockInput.typeText("c");
      await testSetup.renderOnce();
      await new Promise((resolve) => setTimeout(resolve, 0));

      expect(deleteBranchMock).toHaveBeenCalledWith("develop", true);
      expect(removeWorktreeMock).toHaveBeenCalledWith("/tmp/worktree", false);
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("excludes current branch even when selected", async () => {
    const deleteBranchMock = mock(async () => {});
    const removeWorktreeMock = mock(async () => {});
    const getCleanupStatusMock = mock(async () => [
      {
        worktreePath: "/tmp/worktree",
        branch: "main",
        hasUncommittedChanges: false,
        hasUnpushedCommits: false,
        cleanupType: "worktree-and-branch",
        hasRemoteBranch: true,
        hasUniqueCommits: false,
        hasUpstream: true,
        upstream: "origin/main",
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
      isProtectedBranchName: mock(() => true),
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

    const branch = createBranch({
      name: "main",
      label: "main",
      value: "main",
      isCurrent: true,
    });
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
    await new Promise((resolve) => setTimeout(resolve, 0));
    await testSetup.renderOnce();

    try {
      await testSetup.mockInput.typeText(" ");
      await testSetup.renderOnce();

      await testSetup.mockInput.typeText("c");
      await testSetup.renderOnce();
      await new Promise((resolve) => setTimeout(resolve, 0));

      expect(deleteBranchMock).not.toHaveBeenCalled();
      expect(removeWorktreeMock).not.toHaveBeenCalled();
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("repairs selected branch even when worktree is accessible", async () => {
    const repairWorktreesMock = mock(async () => ({
      repairedCount: 1,
      failedCount: 0,
      failures: [],
    }));
    const getCleanupStatusMock = mock(async () => [
      {
        worktreePath: "/tmp/worktree",
        branch: "feature/repair-target",
        hasUncommittedChanges: false,
        hasUnpushedCommits: false,
        cleanupType: "worktree-and-branch",
        hasRemoteBranch: true,
        hasUniqueCommits: false,
        hasUpstream: true,
        upstream: "origin/feature/repair-target",
        isAccessible: true,
        reasons: ["no-diff-with-base"],
      },
    ]);

    mock.module?.("../../../../worktree.js", () => ({
      listAdditionalWorktrees: mock(async () => []),
      repairWorktrees: repairWorktreesMock,
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

    const branch = createBranch({
      name: "feature/repair-target",
      label: "feature/repair-target",
      value: "feature/repair-target",
      worktreeStatus: "active",
    });
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
    await new Promise((resolve) => setTimeout(resolve, 0));
    await testSetup.renderOnce();

    try {
      await testSetup.mockInput.typeText(" ");
      await testSetup.renderOnce();

      await testSetup.mockInput.typeText("x");
      await testSetup.renderOnce();
      await new Promise((resolve) => setTimeout(resolve, 0));

      expect(repairWorktreesMock).toHaveBeenCalledWith([
        "feature/repair-target",
      ]);
    } finally {
      testSetup.renderer.destroy();
    }
  });
});
