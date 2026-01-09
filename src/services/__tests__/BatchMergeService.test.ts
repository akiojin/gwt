import { describe, it, expect, mock, beforeEach, afterEach } from "bun:test";
import { BatchMergeService } from "../BatchMergeService";
import type { BatchMergeConfig, BatchMergeProgress } from "../../cli/ui/types";

// Mock git module
mock.module("../../git", () => ({
  fetchAllRemotes: mock(),
  getLocalBranches: mock(),
  mergeFromBranch: mock(),
  hasMergeConflict: mock(),
  abortMerge: mock(),
  getMergeStatus: mock(),
  getRepositoryRoot: mock(),
  resetToHead: mock(),
  getCurrentBranchName: mock(),
  pushBranchToRemote: mock(),
}));

// Mock worktree module
mock.module("../../worktree", () => ({
  listAdditionalWorktrees: mock(),
  generateWorktreePath: mock(),
  createWorktree: mock(),
}));

import * as git from "../../git";
import * as worktree from "../../worktree";

// ========================================
// BatchMergeService Tests (SPEC-ee33ca26)
// ========================================

describe("BatchMergeService", () => {
  let service: BatchMergeService;

  beforeEach(() => {
    mock.restore();
    service = new BatchMergeService();
  });

  afterEach(() => {
    mock.restore();
  });

  describe("Initialization (T201)", () => {
    it("should create BatchMergeService instance", () => {
      expect(service).toBeInstanceOf(BatchMergeService);
    });

    it("should have required methods", () => {
      expect(typeof service.determineSourceBranch).toBe("function");
      expect(typeof service.getTargetBranches).toBe("function");
      expect(typeof service.ensureWorktree).toBe("function");
      expect(typeof service.mergeBranch).toBe("function");
      expect(typeof service.executeBatchMerge).toBe("function");
    });
  });

  describe("determineSourceBranch (T203)", () => {
    it("should return 'main' when main branch exists", async () => {
      (git.getLocalBranches as ReturnType<typeof mock>).mockResolvedValue([
        {
          name: "main",
          type: "local",
          branchType: "main",
          isCurrent: false,
        },
        {
          name: "develop",
          type: "local",
          branchType: "develop",
          isCurrent: false,
        },
        {
          name: "feature/test",
          type: "local",
          branchType: "feature",
          isCurrent: true,
        },
      ]);

      const sourceBranch = await service.determineSourceBranch();

      expect(sourceBranch).toBe("main");
    });

    it("should return 'develop' when main does not exist but develop exists", async () => {
      (git.getLocalBranches as ReturnType<typeof mock>).mockResolvedValue([
        {
          name: "develop",
          type: "local",
          branchType: "develop",
          isCurrent: false,
        },
        {
          name: "feature/test",
          type: "local",
          branchType: "feature",
          isCurrent: true,
        },
      ]);

      const sourceBranch = await service.determineSourceBranch();

      expect(sourceBranch).toBe("develop");
    });

    it("should return 'master' when main and develop do not exist", async () => {
      (git.getLocalBranches as ReturnType<typeof mock>).mockResolvedValue([
        {
          name: "master",
          type: "local",
          branchType: "main",
          isCurrent: false,
        },
        {
          name: "feature/test",
          type: "local",
          branchType: "feature",
          isCurrent: true,
        },
      ]);

      const sourceBranch = await service.determineSourceBranch();

      expect(sourceBranch).toBe("master");
    });

    it("should throw error when no source branch found", async () => {
      (git.getLocalBranches as ReturnType<typeof mock>).mockResolvedValue([
        {
          name: "feature/test",
          type: "local",
          branchType: "feature",
          isCurrent: true,
        },
      ]);

      await expect(service.determineSourceBranch()).rejects.toThrow(
        "Unable to determine source branch",
      );
    });
  });

  describe("getTargetBranches (T205)", () => {
    it("should return all local branches excluding main, develop, master", async () => {
      (git.getLocalBranches as ReturnType<typeof mock>).mockResolvedValue([
        {
          name: "main",
          type: "local",
          branchType: "main",
          isCurrent: false,
        },
        {
          name: "develop",
          type: "local",
          branchType: "develop",
          isCurrent: false,
        },
        {
          name: "feature/a",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "feature/b",
          type: "local",
          branchType: "feature",
          isCurrent: true,
        },
        {
          name: "hotfix/c",
          type: "local",
          branchType: "hotfix",
          isCurrent: false,
        },
      ]);

      const targetBranches = await service.getTargetBranches();

      expect(targetBranches).toEqual(["feature/a", "feature/b", "hotfix/c"]);
      expect(targetBranches).not.toContain("main");
      expect(targetBranches).not.toContain("develop");
    });

    it("should return empty array when only main/develop/master exist", async () => {
      (git.getLocalBranches as ReturnType<typeof mock>).mockResolvedValue([
        {
          name: "main",
          type: "local",
          branchType: "main",
          isCurrent: true,
        },
        {
          name: "develop",
          type: "local",
          branchType: "develop",
          isCurrent: false,
        },
      ]);

      const targetBranches = await service.getTargetBranches();

      expect(targetBranches).toEqual([]);
    });
  });

  describe("ensureWorktree (T207)", () => {
    it("should return existing worktree path if worktree exists", async () => {
      (
        worktree.listAdditionalWorktrees as ReturnType<typeof mock>
      ).mockResolvedValue([
        {
          path: "/repo/.worktrees/feature-a",
          locked: false,
          prunable: false,
          isAccessible: true,
        },
      ]);

      const worktreePath = await service.ensureWorktree("feature/a");

      expect(worktreePath).toBe("/repo/.worktrees/feature-a");
      expect(worktree.createWorktree).not.toHaveBeenCalled();
    });

    it("should create worktree if it does not exist", async () => {
      (
        worktree.listAdditionalWorktrees as ReturnType<typeof mock>
      ).mockResolvedValue([]);
      (git.getRepositoryRoot as ReturnType<typeof mock>).mockResolvedValue(
        "/repo",
      );
      (
        worktree.generateWorktreePath as ReturnType<typeof mock>
      ).mockResolvedValue("/repo/.worktrees/feature-b");
      (worktree.createWorktree as ReturnType<typeof mock>).mockResolvedValue(
        undefined,
      );

      const worktreePath = await service.ensureWorktree("feature/b");

      expect(worktreePath).toBe("/repo/.worktrees/feature-b");
      expect(worktree.createWorktree).toHaveBeenCalledWith({
        branchName: "feature/b",
        worktreePath: "/repo/.worktrees/feature-b",
        repoRoot: "/repo",
        isNewBranch: false,
        baseBranch: "",
      });
    });
  });

  describe("mergeBranch (T209-T212)", () => {
    const config: BatchMergeConfig = {
      sourceBranch: "main",
      targetBranches: ["feature/a"],
      dryRun: false,
      autoPush: false,
    };

    beforeEach(() => {
      (
        worktree.listAdditionalWorktrees as ReturnType<typeof mock>
      ).mockResolvedValue([
        {
          path: "/repo/.worktrees/feature-a",
          locked: false,
          prunable: false,
        },
      ]);
    });

    it("should successfully merge without conflicts", async () => {
      (git.mergeFromBranch as ReturnType<typeof mock>).mockResolvedValue(
        undefined,
      );
      (git.hasMergeConflict as ReturnType<typeof mock>).mockResolvedValue(
        false,
      );

      const status = await service.mergeBranch("feature/a", "main", config);

      expect(status.branchName).toBe("feature/a");
      expect(status.status).toBe("success");
      expect(status.worktreeCreated).toBe(false);
      expect(git.mergeFromBranch).toHaveBeenCalledWith(
        "/repo/.worktrees/feature-a",
        "main",
        false,
      );
    });

    it("should skip branch on merge conflict", async () => {
      (git.mergeFromBranch as ReturnType<typeof mock>).mockRejectedValue(
        new Error("Merge conflict"),
      );
      (git.hasMergeConflict as ReturnType<typeof mock>).mockResolvedValue(true);
      (git.abortMerge as ReturnType<typeof mock>).mockResolvedValue(undefined);

      const status = await service.mergeBranch("feature/a", "main", config);

      expect(status.branchName).toBe("feature/a");
      expect(status.status).toBe("skipped");
      expect(git.abortMerge).toHaveBeenCalled();
    });

    it("should handle other errors as failed", async () => {
      (git.mergeFromBranch as ReturnType<typeof mock>).mockRejectedValue(
        new Error("Network error"),
      );
      (git.hasMergeConflict as ReturnType<typeof mock>).mockResolvedValue(
        false,
      );

      const status = await service.mergeBranch("feature/a", "main", config);

      expect(status.branchName).toBe("feature/a");
      expect(status.status).toBe("failed");
      expect(status.error).toContain("Network error");
    });
  });

  describe("mergeBranch - Dry-run mode (T303-T304)", () => {
    const dryRunConfig: BatchMergeConfig = {
      sourceBranch: "main",
      targetBranches: ["feature/a"],
      dryRun: true,
      autoPush: false,
    };

    beforeEach(() => {
      (
        worktree.listAdditionalWorktrees as ReturnType<typeof mock>
      ).mockResolvedValue([
        {
          path: "/repo/.worktrees/feature-a",
          locked: false,
          prunable: false,
        },
      ]);
    });

    it("should rollback with resetToHead after successful dry-run merge", async () => {
      (git.mergeFromBranch as ReturnType<typeof mock>).mockResolvedValue(
        undefined,
      );
      (git.hasMergeConflict as ReturnType<typeof mock>).mockResolvedValue(
        false,
      );
      (git.resetToHead as ReturnType<typeof mock>).mockResolvedValue(undefined);

      const status = await service.mergeBranch(
        "feature/a",
        "main",
        dryRunConfig,
      );

      expect(status.branchName).toBe("feature/a");
      expect(status.status).toBe("success");
      expect(git.mergeFromBranch).toHaveBeenCalledWith(
        "/repo/.worktrees/feature-a",
        "main",
        true,
      );
      expect(git.resetToHead).toHaveBeenCalledWith(
        "/repo/.worktrees/feature-a",
      );
    });

    it("should rollback with abortMerge after dry-run merge conflict", async () => {
      (git.mergeFromBranch as ReturnType<typeof mock>).mockRejectedValue(
        new Error("CONFLICT (content)"),
      );
      (git.hasMergeConflict as ReturnType<typeof mock>).mockResolvedValue(true);
      (git.abortMerge as ReturnType<typeof mock>).mockResolvedValue(undefined);

      const status = await service.mergeBranch(
        "feature/a",
        "main",
        dryRunConfig,
      );

      expect(status.branchName).toBe("feature/a");
      expect(status.status).toBe("skipped");
      expect(git.abortMerge).toHaveBeenCalledWith("/repo/.worktrees/feature-a");
      expect(git.resetToHead).not.toHaveBeenCalled();
    });
  });

  describe("mergeBranch - Auto-push mode (T401-T404)", () => {
    const autoPushConfig: BatchMergeConfig = {
      sourceBranch: "main",
      targetBranches: ["feature/a"],
      dryRun: false,
      autoPush: true,
      remote: "origin",
    };

    beforeEach(() => {
      (
        worktree.listAdditionalWorktrees as ReturnType<typeof mock>
      ).mockResolvedValue([
        {
          path: "/repo/.worktrees/feature-a",
          locked: false,
          prunable: false,
        },
      ]);
    });

    it("should push successfully after merge when autoPush is enabled", async () => {
      (git.mergeFromBranch as ReturnType<typeof mock>).mockResolvedValue(
        undefined,
      );
      (git.hasMergeConflict as ReturnType<typeof mock>).mockResolvedValue(
        false,
      );
      (git.getCurrentBranchName as ReturnType<typeof mock>).mockResolvedValue(
        "feature/a",
      );
      (git.pushBranchToRemote as ReturnType<typeof mock>).mockResolvedValue(
        undefined,
      );

      const status = await service.mergeBranch(
        "feature/a",
        "main",
        autoPushConfig,
      );

      expect(status.branchName).toBe("feature/a");
      expect(status.status).toBe("success");
      expect(status.pushStatus).toBe("success");
      expect(git.pushBranchToRemote).toHaveBeenCalledWith(
        "/repo/.worktrees/feature-a",
        "feature/a",
        "origin",
      );
    });

    it("should handle push failure without failing merge", async () => {
      (git.mergeFromBranch as ReturnType<typeof mock>).mockResolvedValue(
        undefined,
      );
      (git.hasMergeConflict as ReturnType<typeof mock>).mockResolvedValue(
        false,
      );
      (git.getCurrentBranchName as ReturnType<typeof mock>).mockResolvedValue(
        "feature/a",
      );
      (git.pushBranchToRemote as ReturnType<typeof mock>).mockRejectedValue(
        new Error("Push failed: permission denied"),
      );

      const status = await service.mergeBranch(
        "feature/a",
        "main",
        autoPushConfig,
      );

      expect(status.branchName).toBe("feature/a");
      expect(status.status).toBe("success");
      expect(status.pushStatus).toBe("failed");
    });

    it("should not push when autoPush is false", async () => {
      const noPushConfig = { ...autoPushConfig, autoPush: false };
      (git.mergeFromBranch as ReturnType<typeof mock>).mockResolvedValue(
        undefined,
      );
      (git.hasMergeConflict as ReturnType<typeof mock>).mockResolvedValue(
        false,
      );

      const status = await service.mergeBranch(
        "feature/a",
        "main",
        noPushConfig,
      );

      expect(status.branchName).toBe("feature/a");
      expect(status.status).toBe("success");
      expect(status.pushStatus).toBe("not_executed");
      expect(git.pushBranchToRemote).not.toHaveBeenCalled();
    });
  });

  describe("executeBatchMerge (T213)", () => {
    it("should execute batch merge for all target branches", async () => {
      const config: BatchMergeConfig = {
        sourceBranch: "main",
        targetBranches: ["feature/a", "feature/b"],
        dryRun: false,
        autoPush: false,
      };

      (git.fetchAllRemotes as ReturnType<typeof mock>).mockResolvedValue(
        undefined,
      );
      (git.getRepositoryRoot as ReturnType<typeof mock>).mockResolvedValue(
        "/repo",
      );
      (
        worktree.listAdditionalWorktrees as ReturnType<typeof mock>
      ).mockResolvedValue([]);
      (worktree.generateWorktreePath as ReturnType<typeof mock>)
        .mockResolvedValueOnce("/repo/.worktrees/feature-a")
        .mockResolvedValueOnce("/repo/.worktrees/feature-b");
      (worktree.createWorktree as ReturnType<typeof mock>).mockResolvedValue(
        undefined,
      );
      (git.mergeFromBranch as ReturnType<typeof mock>).mockResolvedValue(
        undefined,
      );
      (git.hasMergeConflict as ReturnType<typeof mock>).mockResolvedValue(
        false,
      );

      const progressUpdates: BatchMergeProgress[] = [];
      const result = await service.executeBatchMerge(config, (progress) => {
        progressUpdates.push(progress);
      });

      expect(result.statuses).toHaveLength(2);
      expect(result.summary.totalCount).toBe(2);
      expect(result.summary.successCount).toBeGreaterThanOrEqual(0);
      expect(result.cancelled).toBe(false);
      expect(progressUpdates.length).toBeGreaterThan(0);
      expect(git.fetchAllRemotes).toHaveBeenCalled();
    });
  });
});
