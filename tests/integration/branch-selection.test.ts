import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as git from "../../src/git";
import * as worktree from "../../src/worktree";

// Mock execa
vi.mock("execa", () => ({
  execa: vi.fn(),
}));

vi.mock("node:fs", () => ({
  existsSync: vi.fn(() => true),
}));

const mkdirMock = vi.hoisted(() => vi.fn(async () => undefined));

vi.mock("node:fs/promises", async () => {
  const actual =
    await vi.importActual<typeof import("node:fs/promises")>(
      "node:fs/promises",
    );

  const mocked = {
    ...actual,
    mkdir: mkdirMock,
  };

  return {
    ...mocked,
    default: {
      ...actual.default,
      mkdir: mkdirMock,
    },
  };
});

import { execa } from "execa";

describe("Integration: Branch Selection to Worktree Creation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mkdirMock.mockClear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("Branch Selection Flow (T108)", () => {
    it("should complete full flow: get branches -> check worktree -> create worktree", async () => {
      // Setup: Mock git branch list
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          // getLocalBranches
          if (
            args?.[0] === "branch" &&
            args.includes("--format=%(refname:short)")
          ) {
            return {
              stdout: "main\nfeature/test",
              stderr: "",
              exitCode: 0,
            };
          }

          // getRemoteBranches
          if (args?.[0] === "branch" && args.includes("-r")) {
            return {
              stdout: "origin/main",
              stderr: "",
              exitCode: 0,
            };
          }

          // getCurrentBranch
          if (args?.[0] === "branch" && args.includes("--show-current")) {
            return {
              stdout: "main",
              stderr: "",
              exitCode: 0,
            };
          }

          // worktree list (no existing worktree for feature/test)
          if (args?.[0] === "worktree" && args[1] === "list") {
            return {
              stdout: `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main
`,
              stderr: "",
              exitCode: 0,
            };
          }

          // worktree add (create new worktree)
          if (args?.[0] === "worktree" && args[1] === "add") {
            return {
              stdout: "",
              stderr: "",
              exitCode: 0,
            };
          }

          return {
            stdout: "",
            stderr: "",
            exitCode: 0,
          };
        },
      );

      // Step 1: Get all branches
      const branches = await git.getAllBranches();
      // main, feature/test (local) + origin/main (remote) = 3, but getAllBranches marks current
      expect(branches.length).toBeGreaterThanOrEqual(2);

      // Step 2: Select a branch (feature/test)
      const selectedBranch = branches.find((b) => b.name === "feature/test");
      expect(selectedBranch).toBeDefined();

      // Step 3: Check if worktree exists
      const existingWorktree = await worktree.worktreeExists("feature/test");
      expect(existingWorktree).toBeNull();

      // Step 4: Generate worktree path
      const worktreePath = await worktree.generateWorktreePath(
        "/path/to/repo",
        "feature/test",
      );
      expect(worktreePath).toContain("feature-test");

      // Step 5: Create worktree
      await worktree.createWorktree({
        branchName: "feature/test",
        worktreePath,
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      });

      // Verify worktree creation was called
      expect(execa).toHaveBeenCalledWith(
        "git",
        expect.arrayContaining(["worktree", "add"]),
      );
    });

    it("should handle existing worktree gracefully", async () => {
      // Mock worktree that already exists
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          if (args?.[0] === "worktree" && args[1] === "list") {
            return {
              stdout: `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main

worktree /path/to/existing-worktree
HEAD def5678
branch refs/heads/feature/test
`,
              stderr: "",
              exitCode: 0,
            };
          }

          return {
            stdout: "",
            stderr: "",
            exitCode: 0,
          };
        },
      );

      // Check if worktree exists
      const existingWorktree = await worktree.worktreeExists("feature/test");
      expect(existingWorktree).toBe("/path/to/existing-worktree");

      // Should not create new worktree if one already exists
      // This is typically handled in the UI layer
    });

    it("should create worktree for local branch without existing worktree", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          // No existing worktrees
          if (args?.[0] === "worktree" && args[1] === "list") {
            return {
              stdout: `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main
`,
              stderr: "",
              exitCode: 0,
            };
          }

          // Successful worktree creation
          if (args?.[0] === "worktree" && args[1] === "add") {
            return {
              stdout: "",
              stderr: "",
              exitCode: 0,
            };
          }

          return {
            stdout: "",
            stderr: "",
            exitCode: 0,
          };
        },
      );

      // Verify no worktree exists
      const existingPath = await worktree.worktreeExists("feature/new-feature");
      expect(existingPath).toBeNull();

      // Create worktree
      const config = {
        branchName: "feature/new-feature",
        worktreePath: "/path/to/new-worktree",
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      };

      await worktree.createWorktree(config);

      // Verify creation was called with correct arguments
      expect(execa).toHaveBeenCalledWith("git", [
        "worktree",
        "add",
        "/path/to/new-worktree",
        "feature/new-feature",
      ]);
    });

    it("should handle branch selection errors gracefully", async () => {
      // Mock git command failure
      (execa as any).mockRejectedValue(new Error("Git command failed"));

      // Should throw appropriate error
      await expect(git.getAllBranches()).rejects.toThrow(
        "Failed to get local branches",
      );
    });

    it("should validate worktree path generation", async () => {
      const testCases = [
        { branch: "feature/user-auth", expected: "feature-user-auth" },
        { branch: "hotfix/bug-123", expected: "hotfix-bug-123" },
        { branch: "release/1.0.0", expected: "release-1.0.0" },
      ];

      for (const { branch, expected } of testCases) {
        const path = await worktree.generateWorktreePath("/repo", branch);
        expect(path).toContain(expected);
      }
    });
  });

  describe("Error Handling", () => {
    it("should handle worktree creation failure", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          if (args?.[0] === "worktree" && args[1] === "add") {
            throw new Error("Worktree creation failed");
          }
          return { stdout: "", stderr: "", exitCode: 0 };
        },
      );

      const config = {
        branchName: "feature/test",
        worktreePath: "/path/to/worktree",
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      };

      await expect(worktree.createWorktree(config)).rejects.toThrow(
        "Failed to create worktree",
      );
    });

    it("should handle branch listing errors", async () => {
      (execa as any).mockRejectedValue(new Error("Permission denied"));

      await expect(git.getLocalBranches()).rejects.toThrow(
        "Failed to get local branches",
      );
    });
  });
});
