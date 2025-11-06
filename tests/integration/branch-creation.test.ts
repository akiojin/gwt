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

describe("Integration: Branch Creation Workflow (T207)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mkdirMock.mockClear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("Feature Branch Creation Flow", () => {
    it("should create feature branch and worktree", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          // createBranch
          if (args?.[0] === "checkout" && args?.[1] === "-b") {
            return { stdout: "", stderr: "", exitCode: 0 };
          }

          // worktree list
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

          // worktree add
          if (args?.[0] === "worktree" && args[1] === "add") {
            return { stdout: "", stderr: "", exitCode: 0 };
          }

          return { stdout: "", stderr: "", exitCode: 0 };
        },
      );

      // Step 1: Create feature branch
      await git.createBranch("feature/new-ui", "develop");

      // Step 2: Verify branch was created
      expect(execa).toHaveBeenCalledWith("git", [
        "checkout",
        "-b",
        "feature/new-ui",
        "develop",
      ]);

      // Step 3: Create worktree for the branch
      const worktreePath = await worktree.generateWorktreePath(
        "/path/to/repo",
        "feature/new-ui",
      );
      await worktree.createWorktree({
        branchName: "feature/new-ui",
        worktreePath,
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "develop",
      });

      // Step 4: Verify worktree was created
      expect(execa).toHaveBeenCalledWith(
        "git",
        expect.arrayContaining(["worktree", "add"]),
      );
    });
  });

  describe("Hotfix Branch Creation Flow", () => {
    it("should create hotfix branch from main", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          if (args?.[0] === "checkout" && args?.[1] === "-b") {
            return { stdout: "", stderr: "", exitCode: 0 };
          }

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

          if (args?.[0] === "worktree" && args[1] === "add") {
            return { stdout: "", stderr: "", exitCode: 0 };
          }

          return { stdout: "", stderr: "", exitCode: 0 };
        },
      );

      // Hotfix branches should be created from main
      await git.createBranch("hotfix/critical-bug", "main");

      expect(execa).toHaveBeenCalledWith("git", [
        "checkout",
        "-b",
        "hotfix/critical-bug",
        "main",
      ]);

      // Create worktree
      const worktreePath = await worktree.generateWorktreePath(
        "/path/to/repo",
        "hotfix/critical-bug",
      );
      await worktree.createWorktree({
        branchName: "hotfix/critical-bug",
        worktreePath,
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      });

      expect(execa).toHaveBeenCalledWith(
        "git",
        expect.arrayContaining(["worktree", "add"]),
      );
    });
  });

  describe("Branch Type Validation", () => {
    it("should handle branch name patterns correctly", async () => {
      const testCases = [
        { name: "feature/user-auth", baseBranch: "develop" },
        { name: "hotfix/security-patch", baseBranch: "main" },
        { name: "bugfix/login-error", baseBranch: "develop" },
      ];

      (execa as any).mockResolvedValue({ stdout: "", stderr: "", exitCode: 0 });

      for (const { name, baseBranch } of testCases) {
        await git.createBranch(name, baseBranch);
        expect(execa).toHaveBeenCalledWith("git", [
          "checkout",
          "-b",
          name,
          baseBranch,
        ]);
      }
    });
  });

  describe("Error Handling", () => {
    it("should handle branch creation failure", async () => {
      (execa as any).mockRejectedValue(new Error("Branch already exists"));

      await expect(git.createBranch("feature/duplicate")).rejects.toThrow(
        "Failed to create branch",
      );
    });

    it("should handle worktree creation failure after branch creation", async () => {
      let callCount = 0;
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          callCount++;
          if (callCount === 1) {
            // First call: branch creation succeeds
            return { stdout: "", stderr: "", exitCode: 0 };
          } else {
            // Subsequent calls: worktree creation fails
            throw new Error("Worktree creation failed");
          }
        },
      );

      // Create branch (succeeds)
      await git.createBranch("feature/test");

      // Try to create worktree (fails)
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
  });
});
