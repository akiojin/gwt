import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createBranchTable } from "../../../src/ui/table";
import { localBranches, remoteBranches } from "../../fixtures/branches";
import { worktrees } from "../../fixtures/worktrees";

const stripAnsi = (value: string) => value.replace(/\u001B\[[0-9;]*m/g, "");
const normalizeName = (name: string) => {
  const slashIndex = name.indexOf("/");
  return slashIndex === -1 ? name : name.slice(slashIndex + 1);
};

// Mock dependencies
vi.mock("../../../src/git.js", () => ({
  getChangedFilesCount: vi.fn(),
}));

import { getChangedFilesCount } from "../../../src/git.js";

describe("table.ts - Branch Table Operations", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("createBranchTable (icon list)", () => {
    beforeEach(() => {
      (getChangedFilesCount as any).mockResolvedValue(0);
    });

    it("creates one entry per branch without headers", async () => {
      const allBranches = [...localBranches, ...remoteBranches];
      const choices = await createBranchTable(allBranches, worktrees);
      const localSet = new Set(localBranches.map((b) => normalizeName(b.name)));
      const expectedSize = new Set([
        ...localSet,
        ...remoteBranches
          .map((b) => normalizeName(b.name))
          .filter((name) => !localSet.has(name)),
      ]).size;
      expect(choices).toHaveLength(expectedSize);
      expect(choices.every((c) => !c.value.startsWith("__"))).toBe(true);
    });

    it("prefixes icons for branch type, worktree, and changes", async () => {
      const allBranches = [...localBranches, ...remoteBranches];
      const choices = await createBranchTable(allBranches, worktrees);
      const main = choices.find((c) => c.value === "main");
      expect(main).toBeDefined();
      const plain = stripAnsi(main!.name);
      expect(plain.startsWith("⚡🟢⭐  main")).toBe(true);
    });

    it("omits worktree icon when none exists", async () => {
      const allBranches = [...localBranches, ...remoteBranches];
      const choices = await createBranchTable(allBranches, []);
      const remoteOnly = choices.find(
        (c) => c.value === "origin/feature/api-integration",
      );
      expect(remoteOnly).toBeDefined();
      const plain = stripAnsi(remoteOnly!.name);
      expect(plain.includes("☁ ")).toBe(true);
      expect(plain.trimStart().endsWith("feature/api-integration")).toBe(true);
    });

    it("shows change icon when worktree has modifications", async () => {
      (getChangedFilesCount as any).mockImplementation(async (path: string) => {
        if (path.includes("user-auth")) return 5;
        return 0;
      });
      const allBranches = [...localBranches, ...remoteBranches];
      const choices = await createBranchTable(allBranches, worktrees);
      const target = choices.find((c) => c.value === "hotfix/security-patch");
      expect(target).toBeDefined();
      const plain = stripAnsi(target!.name);
      expect(plain.includes("⚠️")).toBe(true);
      expect(plain.includes("hotfix/security-patch")).toBe(true);
      expect(plain.includes("☁")).toBe(false);
    });

    it("masks inaccessible worktrees with warning icon", async () => {
      const inaccessible = [
        {
          branch: "feature/test",
          path: "/inaccessible/path",
          isAccessible: false,
        },
      ];
      const branches = [
        {
          name: "feature/test",
          type: "local" as const,
          branchType: "feature" as const,
          isCurrent: false,
        },
      ];

      const allBranches = [...branches, ...remoteBranches];
      const choices = await createBranchTable(allBranches, inaccessible as any);
      const warning = choices.find((c) => c.value === "feature/test");
      expect(warning).toBeDefined();
      const plain = stripAnsi(warning!.name);
      expect(plain.includes("⚠️")).toBe(true);
      expect(plain.includes("feature/test")).toBe(true);
      expect(plain.includes("☁")).toBe(false);
    });

    it("still emits entries when change detection fails", async () => {
      (getChangedFilesCount as any).mockRejectedValue(new Error("boom"));
      const allBranches = [...localBranches, ...remoteBranches];
      const choices = await createBranchTable(allBranches, worktrees);
      expect(choices.length).toBeGreaterThan(0);
    });
  });
});
