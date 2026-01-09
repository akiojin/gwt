import { describe, it, expect } from "bun:test";
import {
  formatBranchItem,
  formatBranchItems,
} from "../../utils/branchFormatter.js";
import type { BranchInfo } from "../../types.js";

describe("branchFormatter", () => {
  describe("formatBranchItem", () => {
    it("should format a branch without icons", () => {
      const branchInfo: BranchInfo = {
        name: "main",
        type: "local",
        branchType: "main",
        isCurrent: true,
      };

      const result = formatBranchItem(branchInfo);

      expect(result.name).toBe("main");
      expect(result.type).toBe("local");
      expect(result.branchType).toBe("main");
      expect(result.isCurrent).toBe(true);
      expect(result.icons).toHaveLength(0);
      expect(result.label).toBe("main");
      expect(result.value).toBe("main");
      expect(result.hasChanges).toBe(false);
    });

    it("should include last tool usage label when present", () => {
      const branchInfo: BranchInfo = {
        name: "feature/tool",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        lastToolUsage: {
          branch: "feature/tool",
          worktreePath: "/tmp/wt",
          toolId: "codex-cli",
          toolLabel: "Codex",
          mode: "normal",
          timestamp: Date.UTC(2025, 10, 26, 14, 3), // 2025-11-26 14:03 UTC
          model: "gpt-5.2-codex",
        },
      };

      const result = formatBranchItem(branchInfo);

      expect(result.lastToolUsageLabel).toContain("Codex");
      expect(result.lastToolUsageLabel).toContain("2025-11-26");
    });

    it("should set lastToolUsageLabel to null when no usage exists", () => {
      const branchInfo: BranchInfo = {
        name: "feature/no-usage",
        type: "local",
        branchType: "feature",
        isCurrent: false,
      };

      const result = formatBranchItem(branchInfo);

      expect(result.lastToolUsageLabel).toBeNull();
    });

    it("should format a remote branch", () => {
      const branchInfo: BranchInfo = {
        name: "origin/main",
        type: "remote",
        branchType: "main",
        isCurrent: false,
      };

      const result = formatBranchItem(branchInfo);

      expect(result.type).toBe("remote");
      expect(result.label).toBe("origin/main");
      expect(result.remoteName).toBe("origin/main");
    });

    it("should set worktree status when provided", () => {
      const branchInfo: BranchInfo = {
        name: "feature/test",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        worktree: {
          path: "/path/to/worktree",
          locked: false,
          prunable: false,
        },
      };

      const result = formatBranchItem(branchInfo);

      expect(result.worktreeStatus).toBe("active");
    });

    it("should mark branch with changes", () => {
      const branchInfo: BranchInfo = {
        name: "feature/wip",
        type: "local",
        branchType: "feature",
        isCurrent: false,
      };

      const result = formatBranchItem(branchInfo, { hasChanges: true });

      expect(result.hasChanges).toBe(true);
    });

    it("should show warning icon for inaccessible worktree", () => {
      const branchInfo: BranchInfo = {
        name: "feature/broken-worktree",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        worktree: {
          path: "/path/to/worktree",
          locked: false,
          prunable: false,
          isAccessible: false,
        },
      };

      const result = formatBranchItem(branchInfo);

      expect(result.worktreeStatus).toBe("inaccessible");
    });
  });

  describe("formatBranchItems", () => {
    it("should format multiple branches with sorting", () => {
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

      const results = formatBranchItems(branches);

      expect(results).toHaveLength(3);
      // Current branch (main) should be first
      expect(results[0]!.name).toBe("main");
      expect(results[0]!.isCurrent).toBe(true);
      // origin/main is also main branch, so it comes second
      expect(results[1]!.name).toBe("origin/main");
      expect(results[1]!.type).toBe("remote");
      // feature/test comes last
      expect(results[2]!.name).toBe("feature/test");
    });

    it("should handle empty array", () => {
      const results = formatBranchItems([]);

      expect(results).toHaveLength(0);
    });

    it("should sort branches alphabetically when no other priority applies", () => {
      const branches: BranchInfo[] = [
        {
          name: "z-branch",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "a-branch",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
      ];

      const results = formatBranchItems(branches);

      expect(results[0]!.name).toBe("a-branch");
      expect(results[1]!.name).toBe("z-branch");
    });
  });

  describe("formatBranchItems - sorting", () => {
    it("should prioritize current branch at the top", () => {
      const branches: BranchInfo[] = [
        {
          name: "feature/a",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "feature/current",
          type: "local",
          branchType: "feature",
          isCurrent: true,
        },
        {
          name: "main",
          type: "local",
          branchType: "main",
          isCurrent: false,
        },
      ];

      const results = formatBranchItems(branches);

      expect(results[0]!.name).toBe("feature/current");
      expect(results[0]!.isCurrent).toBe(true);
    });

    it("should prioritize main branch as second (after current)", () => {
      const branches: BranchInfo[] = [
        {
          name: "feature/test",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
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
      ];

      const results = formatBranchItems(branches);

      expect(results[0]!.name).toBe("main");
      expect(results[1]!.name).toBe("develop");
    });

    it("should prioritize develop branch after main (when main exists)", () => {
      const branches: BranchInfo[] = [
        {
          name: "feature/test",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "develop",
          type: "local",
          branchType: "develop",
          isCurrent: false,
        },
        {
          name: "main",
          type: "local",
          branchType: "main",
          isCurrent: false,
        },
      ];

      const results = formatBranchItems(branches);

      expect(results[0]!.name).toBe("main");
      expect(results[1]!.name).toBe("develop");
      expect(results[2]!.name).toBe("feature/test");
    });

    it("should NOT prioritize develop branch when main does not exist", () => {
      const branches: BranchInfo[] = [
        {
          name: "feature/a",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "develop",
          type: "local",
          branchType: "develop",
          isCurrent: false,
        },
        {
          name: "feature/z",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
      ];

      const results = formatBranchItems(branches);

      // develop should be sorted alphabetically, not prioritized
      expect(results[0]!.name).toBe("develop");
      expect(results[1]!.name).toBe("feature/a");
      expect(results[2]!.name).toBe("feature/z");
    });

    it("should prioritize branches with worktree", () => {
      const worktreeMap = new Map([
        [
          "feature/with-worktree",
          {
            path: "/path/to/worktree",
            locked: false,
            prunable: false,
            isAccessible: true,
          },
        ],
      ]);

      const branches: BranchInfo[] = [
        {
          name: "feature/no-worktree",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "feature/with-worktree",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          worktree: {
            path: "/path/to/worktree",
            locked: false,
            prunable: false,
            isAccessible: true,
          },
        },
      ];

      const results = formatBranchItems(branches, worktreeMap);

      expect(results[0]!.name).toBe("feature/with-worktree");
      expect(results[1]!.name).toBe("feature/no-worktree");
    });

    it("should sort branches with worktree by latest commit timestamp", () => {
      const worktreeMap = new Map([
        [
          "feature/recent",
          {
            path: "/path/to/recent",
            locked: false,
            prunable: false,
            isAccessible: true,
          },
        ],
        [
          "feature/older",
          {
            path: "/path/to/older",
            locked: false,
            prunable: false,
            isAccessible: true,
          },
        ],
      ]);

      const branches: BranchInfo[] = [
        {
          name: "feature/older",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          worktree: {
            path: "/path/to/older",
            locked: false,
            prunable: false,
            isAccessible: true,
          },
          latestCommitTimestamp: 1_700_000_000,
        },
        {
          name: "feature/recent",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          worktree: {
            path: "/path/to/recent",
            locked: false,
            prunable: false,
            isAccessible: true,
          },
          latestCommitTimestamp: 1_800_000_000,
        },
      ];

      const results = formatBranchItems(branches, worktreeMap);

      expect(results[0]!.name).toBe("feature/recent");
      expect(results[1]!.name).toBe("feature/older");
    });

    it("should prioritize local branches over remote branches", () => {
      const branches: BranchInfo[] = [
        {
          name: "origin/feature/remote",
          type: "remote",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "feature/local",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
      ];

      const results = formatBranchItems(branches);

      expect(results[0]!.name).toBe("feature/local");
      expect(results[0]!.type).toBe("local");
      expect(results[1]!.name).toBe("origin/feature/remote");
      expect(results[1]!.type).toBe("remote");
    });

    it("should sort by latest commit timestamp when worktree status matches", () => {
      const branches: BranchInfo[] = [
        {
          name: "origin/feature/newer",
          type: "remote",
          branchType: "feature",
          isCurrent: false,
          latestCommitTimestamp: 1_900_000_000,
        },
        {
          name: "feature/local-older",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          latestCommitTimestamp: 1_800_000_000,
        },
      ];

      const results = formatBranchItems(branches);

      expect(results[0]!.name).toBe("origin/feature/newer");
      expect(results[1]!.name).toBe("feature/local-older");
    });

    it("should apply all sorting rules in correct priority order", () => {
      const worktreeMap = new Map([
        [
          "feature/with-worktree",
          {
            path: "/path/to/worktree",
            locked: false,
            prunable: false,
            isAccessible: true,
          },
        ],
      ]);

      const branches: BranchInfo[] = [
        {
          name: "origin/feature/z-remote",
          type: "remote",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "feature/with-worktree",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          worktree: {
            path: "/path/to/worktree",
            locked: false,
            prunable: false,
            isAccessible: true,
          },
        },
        {
          name: "feature/z-local-no-worktree",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "feature/a-local-no-worktree",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "develop",
          type: "local",
          branchType: "develop",
          isCurrent: false,
        },
        {
          name: "main",
          type: "local",
          branchType: "main",
          isCurrent: false,
        },
        {
          name: "feature/current",
          type: "local",
          branchType: "feature",
          isCurrent: true,
        },
      ];

      const results = formatBranchItems(branches, worktreeMap);

      // Expected order:
      // 1. Current branch
      // 2. main
      // 3. develop (because main exists)
      // 4. Branches with worktree
      // 5. Local branches (alphabetically)
      // 6. Remote branches
      expect(results[0]!.name).toBe("feature/current");
      expect(results[1]!.name).toBe("main");
      expect(results[2]!.name).toBe("develop");
      expect(results[3]!.name).toBe("feature/with-worktree");
      expect(results[4]!.name).toBe("feature/a-local-no-worktree");
      expect(results[5]!.name).toBe("feature/z-local-no-worktree");
      expect(results[6]!.name).toBe("origin/feature/z-remote");
    });

    it("should handle release and hotfix branches without special priority", () => {
      const branches: BranchInfo[] = [
        {
          name: "feature/test",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "hotfix/urgent",
          type: "local",
          branchType: "hotfix",
          isCurrent: false,
        },
        {
          name: "release/v1.0",
          type: "local",
          branchType: "release",
          isCurrent: false,
        },
      ];

      const results = formatBranchItems(branches);

      // Should be sorted alphabetically (no special priority)
      expect(results[0]!.name).toBe("feature/test");
      expect(results[1]!.name).toBe("hotfix/urgent");
      expect(results[2]!.name).toBe("release/v1.0");
    });

    it("should sort by latest activity time (max of git commit and tool usage)", () => {
      const branches: BranchInfo[] = [
        {
          name: "feature/git-newer",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          latestCommitTimestamp: 1_800_000_000, // git commit is newer
          lastToolUsage: {
            branch: "feature/git-newer",
            worktreePath: "/tmp/wt1",
            toolId: "claude-code",
            toolLabel: "Claude",
            timestamp: 1_700_000_000_000, // 1_700_000_000 seconds (older)
          },
        },
        {
          name: "feature/tool-newer",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          latestCommitTimestamp: 1_700_000_000, // git commit is older
          lastToolUsage: {
            branch: "feature/tool-newer",
            worktreePath: "/tmp/wt2",
            toolId: "claude-code",
            toolLabel: "Claude",
            timestamp: 1_800_000_000_000, // 1_800_000_000 seconds (newer)
          },
        },
      ];

      const results = formatBranchItems(branches);

      // Both have same latest activity time (1_800_000_000), so alphabetical
      // feature/git-newer: max(1_800_000_000, 1_700_000_000) = 1_800_000_000
      // feature/tool-newer: max(1_700_000_000, 1_800_000_000) = 1_800_000_000
      expect(results[0]!.name).toBe("feature/git-newer");
      expect(results[1]!.name).toBe("feature/tool-newer");
    });

    it("should prioritize branch with tool usage over branch with only git commit when tool is newer", () => {
      const branches: BranchInfo[] = [
        {
          name: "feature/git-only",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          latestCommitTimestamp: 1_700_000_000,
        },
        {
          name: "feature/with-tool",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          latestCommitTimestamp: 1_600_000_000,
          lastToolUsage: {
            branch: "feature/with-tool",
            worktreePath: "/tmp/wt",
            toolId: "claude-code",
            toolLabel: "Claude",
            timestamp: 1_800_000_000_000, // 1_800_000_000 seconds (newest)
          },
        },
      ];

      const results = formatBranchItems(branches);

      // feature/with-tool has newer activity (tool usage at 1_800_000_000)
      expect(results[0]!.name).toBe("feature/with-tool");
      expect(results[1]!.name).toBe("feature/git-only");
    });

    it("should prioritize branch with newer git commit over branch with older tool usage", () => {
      const branches: BranchInfo[] = [
        {
          name: "feature/old-tool",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          latestCommitTimestamp: 1_600_000_000,
          lastToolUsage: {
            branch: "feature/old-tool",
            worktreePath: "/tmp/wt",
            toolId: "claude-code",
            toolLabel: "Claude",
            timestamp: 1_650_000_000_000, // 1_650_000_000 seconds
          },
        },
        {
          name: "feature/new-git",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          latestCommitTimestamp: 1_800_000_000, // newest
        },
      ];

      const results = formatBranchItems(branches);

      // feature/new-git has newer activity (git commit at 1_800_000_000)
      expect(results[0]!.name).toBe("feature/new-git");
      expect(results[1]!.name).toBe("feature/old-tool");
    });
  });
});
