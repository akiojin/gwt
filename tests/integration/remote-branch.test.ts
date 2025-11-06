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

  return {
    ...actual,
    mkdir: mkdirMock,
  };
});

import { execa } from "execa";

describe("Integration: Remote Branch to Local Worktree", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mkdirMock.mockClear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("Remote Branch Flow (T109)", () => {
    it("should create local branch from remote and create worktree", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          // getRemoteBranches
          if (args?.[0] === "branch" && args.includes("-r")) {
            return {
              stdout: "origin/main\norigin/feature/remote-feature",
              stderr: "",
              exitCode: 0,
            };
          }

          // branchExists - local branch doesn't exist yet
          if (args?.[0] === "show-ref" && args.includes("--verify")) {
            throw new Error("Branch not found");
          }

          // worktree list - no existing worktrees
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

          // worktree add with new branch from remote
          if (
            args?.[0] === "worktree" &&
            args[1] === "add" &&
            args.includes("-b")
          ) {
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

      // Get remote branches
      const remoteBranches = await git.getRemoteBranches();
      const remoteBranch = remoteBranches.find(
        (b) => b.name === "origin/feature/remote-feature",
      );
      expect(remoteBranch).toBeDefined();

      // Extract local branch name from remote
      const localBranchName = "feature/remote-feature";

      // Check if local branch exists
      const localExists = await git.branchExists(localBranchName);
      expect(localExists).toBe(false);

      // Create worktree with new branch tracking remote
      const config = {
        branchName: localBranchName,
        worktreePath: "/path/to/worktree",
        repoRoot: "/path/to/repo",
        isNewBranch: true,
        baseBranch: "origin/feature/remote-feature",
      };

      await worktree.createWorktree(config);

      // Verify worktree was created with correct flags
      expect(execa).toHaveBeenCalledWith(
        "git",
        expect.arrayContaining(["-b", localBranchName]),
      );
    });

    it("should handle remote branch that already has local counterpart", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          // branchExists - local branch already exists
          if (args?.[0] === "show-ref" && args.includes("--verify")) {
            return {
              stdout: "",
              stderr: "",
              exitCode: 0,
            };
          }

          // worktree add for existing local branch
          if (
            args?.[0] === "worktree" &&
            args[1] === "add" &&
            !args.includes("-b")
          ) {
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

      const localBranchName = "feature/existing";

      // Check if local branch exists
      const localExists = await git.branchExists(localBranchName);
      expect(localExists).toBe(true);

      // Create worktree for existing branch
      const config = {
        branchName: localBranchName,
        worktreePath: "/path/to/worktree",
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      };

      await worktree.createWorktree(config);

      // Should use existing branch (no -b flag)
      expect(execa).toHaveBeenCalledWith(
        "git",
        expect.not.arrayContaining(["-b"]),
      );
    });

    it("should extract local branch name from remote branch name", () => {
      const testCases = [
        { remote: "origin/feature/test", expected: "feature/test" },
        { remote: "origin/hotfix/bug-123", expected: "hotfix/bug-123" },
        { remote: "origin/main", expected: "main" },
      ];

      testCases.forEach(({ remote, expected }) => {
        const localName = remote.replace(/^origin\//, "");
        expect(localName).toBe(expected);
      });
    });

    it("should handle worktree creation with custom base branch", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
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

      const config = {
        branchName: "hotfix/urgent-fix",
        worktreePath: "/path/to/worktree",
        repoRoot: "/path/to/repo",
        isNewBranch: true,
        baseBranch: "origin/develop",
      };

      await worktree.createWorktree(config);

      // Verify base branch was used
      expect(execa).toHaveBeenCalledWith(
        "git",
        expect.arrayContaining(["origin/develop"]),
      );
    });
  });

  describe("Error Handling", () => {
    it("should handle remote branch fetch errors", async () => {
      (execa as any).mockRejectedValue(new Error("Network error"));

      await expect(git.getRemoteBranches()).rejects.toThrow(
        "Failed to get remote branches",
      );
    });

    it("should handle branch creation conflicts", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          if (
            args?.[0] === "worktree" &&
            args[1] === "add" &&
            args.includes("-b")
          ) {
            throw new Error("Branch already exists");
          }
          return {
            stdout: "",
            stderr: "",
            exitCode: 0,
          };
        },
      );

      const config = {
        branchName: "feature/duplicate",
        worktreePath: "/path/to/worktree",
        repoRoot: "/path/to/repo",
        isNewBranch: true,
        baseBranch: "main",
      };

      await expect(worktree.createWorktree(config)).rejects.toThrow(
        "Failed to create worktree",
      );
    });
  });
});
