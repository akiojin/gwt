import { describe, it, expect } from "vitest";
import {
  formatBranchItem,
  formatBranchItems,
} from "../../utils/branchFormatter.js";
import type { BranchInfo } from "../../types.js";
import stringWidth from "string-width";

describe("branchFormatter", () => {
  describe("formatBranchItem", () => {
    it("should format a main branch", () => {
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
      expect(result.icons).toContain("⚡"); // main icon
      expect(result.icons).toContain("⭐"); // current icon
      expect(result.label).toContain("main");
      expect(result.value).toBe("main");
      expect(result.hasChanges).toBe(false);
    });

    it("should format a feature branch", () => {
      const branchInfo: BranchInfo = {
        name: "feature/new-ui",
        type: "local",
        branchType: "feature",
        isCurrent: false,
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("✨"); // feature icon
      expect(result.icons).not.toContain("⭐"); // not current
      expect(result.label).toContain("feature/new-ui");
      expect(result.value).toBe("feature/new-ui");
    });

    it("should format a bugfix branch", () => {
      const branchInfo: BranchInfo = {
        name: "bugfix/security-issue",
        type: "local",
        branchType: "bugfix",
        isCurrent: false,
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("🐛"); // bugfix icon
      expect(result.label).toContain("bugfix/security-issue");
    });

    it("should format a hotfix branch", () => {
      const branchInfo: BranchInfo = {
        name: "hotfix/critical-bug",
        type: "local",
        branchType: "hotfix",
        isCurrent: false,
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("🔥"); // hotfix icon
      expect(result.label).toContain("hotfix/critical-bug");
    });

    it("should format a release branch", () => {
      const branchInfo: BranchInfo = {
        name: "release/v1.0.0",
        type: "local",
        branchType: "release",
        isCurrent: false,
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("🚀"); // release icon
      expect(result.label).toContain("release/v1.0.0");
    });

    it("should format a remote branch", () => {
      const branchInfo: BranchInfo = {
        name: "origin/main",
        type: "remote",
        branchType: "main",
        isCurrent: false,
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("⚡"); // main icon
      expect(result.icons).toContain("☁"); // remote icon
      expect(result.type).toBe("remote");
      expect(result.label).toContain("origin/main");
    });

    it("should align branch names regardless of remote icon presence", () => {
      const localBranch: BranchInfo = {
        name: "feature/foo",
        type: "local",
        branchType: "feature",
        isCurrent: false,
      };
      const remoteBranch: BranchInfo = {
        name: "origin/feature/foo",
        type: "remote",
        branchType: "feature",
        isCurrent: false,
      };

      const localResult = formatBranchItem(localBranch);
      const remoteResult = formatBranchItem(remoteBranch);

      const localNameIndex = localResult.label.indexOf(localResult.name);
      const remoteNameIndex = remoteResult.label.indexOf(remoteResult.name);

      expect(localNameIndex).toBeGreaterThan(0);
      expect(localNameIndex).toBe(remoteNameIndex);
      expect(remoteResult.label).toMatch(/☁(?:️|︎)?\s+origin/);
    });

    it("should keep icon columns fixed-width when using wide emoji icons", () => {
      const branchInfo: BranchInfo = {
        name: "feature/wide-icons",
        type: "remote",
        branchType: "feature",
        isCurrent: false,
        hasUnpushedCommits: true,
        worktree: {
          path: "/path/to/worktree",
          locked: false,
          prunable: false,
          isAccessible: true,
        },
      };

      const result = formatBranchItem(branchInfo, { hasChanges: true });

      // Four icon columns should occupy exactly 8 columns (4 * COLUMN_WIDTH)
      const iconBlockWidth =
        stringWidth(result.label) - stringWidth(branchInfo.name);

      expect(iconBlockWidth).toBe(8);
    });

    it("should include worktree status icon when provided", () => {
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

      expect(result.icons).toContain("🟢"); // active worktree icon
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

      expect(result.icons).toContain("✏️"); // changes icon
      expect(result.hasChanges).toBe(true);
    });

    it("should show unpushed commits icon", () => {
      const branchInfo: BranchInfo = {
        name: "feature/unpushed",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        hasUnpushedCommits: true,
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("⬆️"); // unpushed icon
      expect(result.label).toContain("⬆️");
    });

    it("should show open PR icon", () => {
      const branchInfo: BranchInfo = {
        name: "feature/pr-open",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        openPR: { number: 123, title: "Test PR" },
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("🔀"); // open PR icon
      expect(result.label).toContain("🔀");
    });

    it("should show merged PR icon", () => {
      const branchInfo: BranchInfo = {
        name: "feature/pr-merged",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        mergedPR: { number: 456, mergedAt: "2025-10-31T00:00:00Z" },
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("✅"); // merged PR icon
      expect(result.label).toContain("✅");
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

      expect(result.icons).toContain("🟠"); // inaccessible worktree icon
      expect(result.icons).toContain("⚠️"); // warning icon
      expect(result.worktreeStatus).toBe("inaccessible");
    });

    it("should prioritize hasChanges over unpushed commits", () => {
      const branchInfo: BranchInfo = {
        name: "feature/both",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        hasUnpushedCommits: true,
      };

      const resultWithChanges = formatBranchItem(branchInfo, {
        hasChanges: true,
      });
      expect(resultWithChanges.icons).toContain("✏️");
      expect(resultWithChanges.icons).not.toContain("⬆️");

      const resultWithoutChanges = formatBranchItem(branchInfo);
      expect(resultWithoutChanges.icons).toContain("⬆️");
      expect(resultWithoutChanges.icons).not.toContain("✏️");
    });

    it("should prioritize unpushed over open PR", () => {
      const branchInfo: BranchInfo = {
        name: "feature/unpushed-pr",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        hasUnpushedCommits: true,
        openPR: { number: 123, title: "Test PR" },
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("⬆️");
      expect(result.icons).not.toContain("🔀");
    });

    it("should prioritize open PR over merged PR", () => {
      const branchInfo: BranchInfo = {
        name: "feature/both-pr",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        openPR: { number: 123, title: "Test PR" },
        mergedPR: { number: 456, mergedAt: "2025-10-31T00:00:00Z" },
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("🔀");
      expect(result.icons).not.toContain("✅");
    });

    it("should prioritize merged PR over warning", () => {
      const branchInfo: BranchInfo = {
        name: "feature/merged-broken",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        mergedPR: { number: 456, mergedAt: "2025-10-31T00:00:00Z" },
        worktree: {
          path: "/path/to/worktree",
          locked: false,
          prunable: false,
          isAccessible: false,
        },
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("✅");
      expect(result.icons).not.toContain("⚠️");
    });

    it("should prioritize warning over current branch icon", () => {
      const branchInfo: BranchInfo = {
        name: "feature/current-broken",
        type: "local",
        branchType: "feature",
        isCurrent: true,
        worktree: {
          path: "/path/to/worktree",
          locked: false,
          prunable: false,
          isAccessible: false,
        },
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("⚠️");
      expect(result.icons).not.toContain("⭐");
    });

    it("should handle develop branch", () => {
      const branchInfo: BranchInfo = {
        name: "develop",
        type: "local",
        branchType: "develop",
        isCurrent: false,
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("⚡"); // develop icon (same as main)
      expect(result.label).toContain("develop");
    });

    it("should handle other branch type", () => {
      const branchInfo: BranchInfo = {
        name: "custom-branch",
        type: "local",
        branchType: "other",
        isCurrent: false,
      };

      const result = formatBranchItem(branchInfo);

      expect(result.icons).toContain("📌"); // other icon
      expect(result.label).toContain("custom-branch");
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
      expect(results[0].name).toBe("main");
      expect(results[0].isCurrent).toBe(true);
      // origin/main is also main branch, so it comes second
      expect(results[1].name).toBe("origin/main");
      expect(results[1].type).toBe("remote");
      // feature/test comes last
      expect(results[2].name).toBe("feature/test");
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

      expect(results[0].name).toBe("a-branch");
      expect(results[1].name).toBe("z-branch");
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

      expect(results[0].name).toBe("feature/current");
      expect(results[0].isCurrent).toBe(true);
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

      expect(results[0].name).toBe("main");
      expect(results[1].name).toBe("develop");
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

      expect(results[0].name).toBe("main");
      expect(results[1].name).toBe("develop");
      expect(results[2].name).toBe("feature/test");
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
      expect(results[0].name).toBe("develop");
      expect(results[1].name).toBe("feature/a");
      expect(results[2].name).toBe("feature/z");
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

      expect(results[0].name).toBe("feature/with-worktree");
      expect(results[1].name).toBe("feature/no-worktree");
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

      expect(results[0].name).toBe("feature/recent");
      expect(results[1].name).toBe("feature/older");
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

      expect(results[0].name).toBe("feature/local");
      expect(results[0].type).toBe("local");
      expect(results[1].name).toBe("origin/feature/remote");
      expect(results[1].type).toBe("remote");
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

      expect(results[0].name).toBe("origin/feature/newer");
      expect(results[1].name).toBe("feature/local-older");
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
      expect(results[0].name).toBe("feature/current");
      expect(results[1].name).toBe("main");
      expect(results[2].name).toBe("develop");
      expect(results[3].name).toBe("feature/with-worktree");
      expect(results[4].name).toBe("feature/a-local-no-worktree");
      expect(results[5].name).toBe("feature/z-local-no-worktree");
      expect(results[6].name).toBe("origin/feature/z-remote");
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
      expect(results[0].name).toBe("feature/test");
      expect(results[1].name).toBe("hotfix/urgent");
      expect(results[2].name).toBe("release/v1.0");
    });
  });
});
