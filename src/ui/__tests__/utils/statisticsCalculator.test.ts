import { describe, it, expect } from "vitest";
import { calculateStatistics } from "../../utils/statisticsCalculator.js";
import type { BranchInfo } from "../../types.js";

describe("statisticsCalculator", () => {
  describe("calculateStatistics", () => {
    it("should calculate basic counts", () => {
      const branches: BranchInfo[] = [
        {
          name: "main",
          type: "local",
          branchType: "main",
          isCurrent: true,
        },
        {
          name: "feature/test",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "origin/main",
          type: "remote",
          branchType: "main",
          isCurrent: false,
        },
      ];

      const stats = calculateStatistics(branches);

      expect(stats.localCount).toBe(2);
      expect(stats.remoteCount).toBe(1);
      expect(stats.worktreeCount).toBe(0);
      expect(stats.changesCount).toBe(0);
      expect(stats.lastUpdated).toBeInstanceOf(Date);
    });

    it("should count worktrees", () => {
      const branches: BranchInfo[] = [
        {
          name: "main",
          type: "local",
          branchType: "main",
          isCurrent: true,
          worktree: {
            path: "/path/to/main",
            locked: false,
            prunable: false,
          },
        },
        {
          name: "feature/test",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          worktree: {
            path: "/path/to/feature",
            locked: false,
            prunable: false,
          },
        },
        {
          name: "feature/no-worktree",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
      ];

      const stats = calculateStatistics(branches);

      expect(stats.localCount).toBe(3);
      expect(stats.worktreeCount).toBe(2);
    });

    it("should count branches with changes", () => {
      const branches: BranchInfo[] = [
        {
          name: "main",
          type: "local",
          branchType: "main",
          isCurrent: true,
          worktree: {
            path: "/path/to/main",
            locked: false,
            prunable: false,
          },
        },
        {
          name: "feature/test",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          worktree: {
            path: "/path/to/feature",
            locked: false,
            prunable: false,
          },
        },
      ];

      const changedBranches = new Set(["main", "feature/test"]);
      const stats = calculateStatistics(branches, changedBranches);

      expect(stats.changesCount).toBe(2);
    });

    it("should handle empty branch array", () => {
      const stats = calculateStatistics([]);

      expect(stats.localCount).toBe(0);
      expect(stats.remoteCount).toBe(0);
      expect(stats.worktreeCount).toBe(0);
      expect(stats.changesCount).toBe(0);
      expect(stats.lastUpdated).toBeInstanceOf(Date);
    });

    it("should handle only remote branches", () => {
      const branches: BranchInfo[] = [
        {
          name: "origin/main",
          type: "remote",
          branchType: "main",
          isCurrent: false,
        },
        {
          name: "origin/develop",
          type: "remote",
          branchType: "develop",
          isCurrent: false,
        },
      ];

      const stats = calculateStatistics(branches);

      expect(stats.localCount).toBe(0);
      expect(stats.remoteCount).toBe(2);
      expect(stats.worktreeCount).toBe(0);
    });

    it("should handle mixed branches with worktrees and changes", () => {
      const branches: BranchInfo[] = [
        {
          name: "main",
          type: "local",
          branchType: "main",
          isCurrent: true,
          worktree: {
            path: "/path/to/main",
            locked: false,
            prunable: false,
          },
        },
        {
          name: "feature/test",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          worktree: {
            path: "/path/to/feature",
            locked: false,
            prunable: false,
          },
        },
        {
          name: "feature/no-worktree",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "origin/main",
          type: "remote",
          branchType: "main",
          isCurrent: false,
        },
        {
          name: "origin/develop",
          type: "remote",
          branchType: "develop",
          isCurrent: false,
        },
      ];

      const changedBranches = new Set(["main", "feature/test"]);
      const stats = calculateStatistics(branches, changedBranches);

      expect(stats.localCount).toBe(3);
      expect(stats.remoteCount).toBe(2);
      expect(stats.worktreeCount).toBe(2);
      expect(stats.changesCount).toBe(2);
    });

    it("should only count changes for local branches with worktrees", () => {
      const branches: BranchInfo[] = [
        {
          name: "main",
          type: "local",
          branchType: "main",
          isCurrent: true,
          worktree: {
            path: "/path/to/main",
            locked: false,
            prunable: false,
          },
        },
        {
          name: "feature/no-worktree",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "origin/main",
          type: "remote",
          branchType: "main",
          isCurrent: false,
        },
      ];

      // All branches in changed set, but only worktree branches should count
      const changedBranches = new Set([
        "main",
        "feature/no-worktree",
        "origin/main",
      ]);
      const stats = calculateStatistics(branches, changedBranches);

      expect(stats.changesCount).toBe(1); // Only main has worktree
    });

    it("should generate recent timestamp", () => {
      const before = new Date();
      const stats = calculateStatistics([]);
      const after = new Date();

      expect(stats.lastUpdated.getTime()).toBeGreaterThanOrEqual(
        before.getTime(),
      );
      expect(stats.lastUpdated.getTime()).toBeLessThanOrEqual(after.getTime());
    });
  });
});
