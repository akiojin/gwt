import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { BatchMergeService } from "../BatchMergeService";
import type { BatchMergeConfig } from "../../ui/types";

// Mock git module
vi.mock("../../git", () => ({
  fetchAllRemotes: vi.fn(),
  getLocalBranches: vi.fn(),
  mergeFromBranch: vi.fn(),
  hasMergeConflict: vi.fn(),
  abortMerge: vi.fn(),
  getMergeStatus: vi.fn(),
}));

// Mock worktree module
vi.mock("../../worktree", () => ({
  getWorktrees: vi.fn(),
  createWorktree: vi.fn(),
}));

import * as git from "../../git";
import * as worktree from "../../worktree";

// ========================================
// BatchMergeService Tests (SPEC-ee33ca26)
// ========================================

describe("BatchMergeService", () => {
  let service: BatchMergeService;

  beforeEach(() => {
    vi.clearAllMocks();
    service = new BatchMergeService();
  });

  afterEach(() => {
    vi.restoreAllMocks();
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
      vi.mocked(git.getLocalBranches).mockResolvedValue([
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
      vi.mocked(git.getLocalBranches).mockResolvedValue([
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
      vi.mocked(git.getLocalBranches).mockResolvedValue([
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
      vi.mocked(git.getLocalBranches).mockResolvedValue([
        {
          name: "feature/test",
          type: "local",
          branchType: "feature",
          isCurrent: true,
        },
      ]);

      await expect(service.determineSourceBranch()).rejects.toThrow(
        "マージ元ブランチを特定できません",
      );
    });
  });

  describe("getTargetBranches (T205)", () => {
    it("should return all local branches excluding main, develop, master", async () => {
      vi.mocked(git.getLocalBranches).mockResolvedValue([
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
      vi.mocked(git.getLocalBranches).mockResolvedValue([
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
      vi.mocked(worktree.getWorktrees).mockResolvedValue([
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
      vi.mocked(worktree.getWorktrees).mockResolvedValue([]);
      vi.mocked(worktree.createWorktree).mockResolvedValue(
        "/repo/.worktrees/feature-b",
      );

      const worktreePath = await service.ensureWorktree("feature/b");

      expect(worktreePath).toBe("/repo/.worktrees/feature-b");
      expect(worktree.createWorktree).toHaveBeenCalledWith("feature/b");
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
      vi.mocked(worktree.getWorktrees).mockResolvedValue([
        {
          path: "/repo/.worktrees/feature-a",
          locked: false,
          prunable: false,
        },
      ]);
    });

    it("should successfully merge without conflicts", async () => {
      vi.mocked(git.mergeFromBranch).mockResolvedValue();
      vi.mocked(git.hasMergeConflict).mockResolvedValue(false);

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
      vi.mocked(git.mergeFromBranch).mockRejectedValue(
        new Error("Merge conflict"),
      );
      vi.mocked(git.hasMergeConflict).mockResolvedValue(true);
      vi.mocked(git.abortMerge).mockResolvedValue();

      const status = await service.mergeBranch("feature/a", "main", config);

      expect(status.branchName).toBe("feature/a");
      expect(status.status).toBe("skipped");
      expect(git.abortMerge).toHaveBeenCalled();
    });

    it("should handle other errors as failed", async () => {
      vi.mocked(git.mergeFromBranch).mockRejectedValue(
        new Error("Network error"),
      );
      vi.mocked(git.hasMergeConflict).mockResolvedValue(false);

      const status = await service.mergeBranch("feature/a", "main", config);

      expect(status.branchName).toBe("feature/a");
      expect(status.status).toBe("failed");
      expect(status.error).toContain("Network error");
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

      vi.mocked(git.fetchAllRemotes).mockResolvedValue();
      vi.mocked(worktree.getWorktrees).mockResolvedValue([]);
      vi.mocked(worktree.createWorktree)
        .mockResolvedValueOnce("/repo/.worktrees/feature-a")
        .mockResolvedValueOnce("/repo/.worktrees/feature-b");
      vi.mocked(git.mergeFromBranch).mockResolvedValue();
      vi.mocked(git.hasMergeConflict).mockResolvedValue(false);

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
