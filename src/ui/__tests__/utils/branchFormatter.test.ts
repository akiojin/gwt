import { describe, it, expect } from "vitest";
import {
  formatBranchItem,
  formatBranchItems,
} from "../../utils/branchFormatter.js";
import type { BranchInfo, BranchItem } from "../../types.js";

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
    it("should format multiple branches", () => {
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
      expect(results[0].name).toBe("main");
      expect(results[0].isCurrent).toBe(true);
      expect(results[1].name).toBe("feature/test");
      expect(results[2].name).toBe("origin/main");
      expect(results[2].type).toBe("remote");
    });

    it("should handle empty array", () => {
      const results = formatBranchItems([]);

      expect(results).toHaveLength(0);
    });

    it("should preserve branch order", () => {
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

      expect(results[0].name).toBe("z-branch");
      expect(results[1].name).toBe("a-branch");
    });
  });
});
