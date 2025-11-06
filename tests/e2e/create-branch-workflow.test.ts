import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// Mock all dependencies
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
import * as git from "../../src/git";
import * as worktree from "../../src/worktree";

describe("E2E: Create Branch Workflow (T209)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mkdirMock.mockClear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("Complete Branch Creation Workflows", () => {
    it("should handle feature branch creation end-to-end", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          // getLocalBranches
          if (
            args?.[0] === "branch" &&
            args.includes("--format=%(refname:short)")
          ) {
            return {
              stdout: "main\ndevelop",
              stderr: "",
              exitCode: 0,
            };
          }

          // getRemoteBranches
          if (args?.[0] === "branch" && args.includes("-r")) {
            return {
              stdout: "origin/main\norigin/develop",
              stderr: "",
              exitCode: 0,
            };
          }

          // getCurrentBranch
          if (args?.[0] === "branch" && args.includes("--show-current")) {
            return { stdout: "develop", stderr: "", exitCode: 0 };
          }

          // branchExists - new branch doesn't exist
          if (args?.[0] === "show-ref") {
            throw new Error("Branch not found");
          }

          // createBranch
          if (args?.[0] === "checkout" && args?.[1] === "-b") {
            return { stdout: "", stderr: "", exitCode: 0 };
          }

          // worktree list
          if (args?.[0] === "worktree" && args[1] === "list") {
            return {
              stdout: `worktree /path/to/repo
HEAD abc1234
branch refs/heads/develop
`,
              stderr: "",
              exitCode: 0,
            };
          }

          // worktree add
          if (args?.[0] === "worktree" && args[1] === "add") {
            return { stdout: "", stderr: "", exitCode: 0 };
          }

          if (args?.[0] === "rev-parse") {
            return { stdout: ".git", stderr: "", exitCode: 0 };
          }

          return { stdout: "", stderr: "", exitCode: 0 };
        },
      );

      // === USER WORKFLOW ===

      // Step 1: User decides to create new feature branch
      const branchType = "feature";
      const branchName = "feature/new-dashboard";
      const baseBranch = "develop";

      // Step 2: Verify branch doesn't exist
      const exists = await git.branchExists(branchName);
      expect(exists).toBe(false);

      // Step 3: Create branch
      await git.createBranch(branchName, baseBranch);
      expect(execa).toHaveBeenCalledWith("git", [
        "checkout",
        "-b",
        branchName,
        baseBranch,
      ]);

      // Step 4: Generate worktree path
      const repoRoot = "/path/to/repo";
      const worktreePath = await worktree.generateWorktreePath(
        repoRoot,
        branchName,
      );
      expect(worktreePath).toContain("feature-new-dashboard");

      // Step 5: Create worktree
      await worktree.createWorktree({
        branchName,
        worktreePath,
        repoRoot,
        isNewBranch: false,
        baseBranch,
      });

      expect(execa).toHaveBeenCalledWith(
        "git",
        expect.arrayContaining(["worktree", "add"]),
      );
    });

    it("should handle hotfix branch creation end-to-end", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          if (
            args?.[0] === "branch" &&
            args.includes("--format=%(refname:short)")
          ) {
            return { stdout: "main", stderr: "", exitCode: 0 };
          }

          if (args?.[0] === "branch" && args.includes("-r")) {
            return { stdout: "origin/main", stderr: "", exitCode: 0 };
          }

          if (args?.[0] === "branch" && args.includes("--show-current")) {
            return { stdout: "main", stderr: "", exitCode: 0 };
          }

          if (args?.[0] === "show-ref") {
            throw new Error("Branch not found");
          }

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

          if (args?.[0] === "rev-parse") {
            return { stdout: ".git", stderr: "", exitCode: 0 };
          }

          return { stdout: "", stderr: "", exitCode: 0 };
        },
      );

      // Hotfix workflow
      const branchName = "hotfix/security-patch";
      const baseBranch = "main";

      await git.createBranch(branchName, baseBranch);

      const worktreePath = await worktree.generateWorktreePath(
        "/path/to/repo",
        branchName,
      );
      await worktree.createWorktree({
        branchName,
        worktreePath,
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch,
      });

      expect(execa).toHaveBeenCalledWith("git", [
        "checkout",
        "-b",
        branchName,
        baseBranch,
      ]);
    });

    it("should handle release branch creation with version bump", async () => {
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          if (args?.[0] === "checkout" && args?.[1] === "-b") {
            return { stdout: "", stderr: "", exitCode: 0 };
          }

          if (args?.[0] === "worktree" && args[1] === "list") {
            return {
              stdout: `worktree /path/to/repo
HEAD abc1234
branch refs/heads/develop
`,
              stderr: "",
              exitCode: 0,
            };
          }

          if (args?.[0] === "worktree" && args[1] === "add") {
            return { stdout: "", stderr: "", exitCode: 0 };
          }

          if (args?.[0] === "rev-parse") {
            return { stdout: ".git", stderr: "", exitCode: 0 };
          }

          return { stdout: "", stderr: "", exitCode: 0 };
        },
      );

      // Release workflow
      const currentVersion = "1.2.3";
      const newVersion = git.calculateNewVersion(currentVersion, "minor");
      expect(newVersion).toBe("1.3.0");

      const branchName = `release/${newVersion}`;
      await git.createBranch(branchName, "develop");

      const worktreePath = await worktree.generateWorktreePath(
        "/path/to/repo",
        branchName,
      );
      await worktree.createWorktree({
        branchName,
        worktreePath,
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "develop",
      });

      expect(execa).toHaveBeenCalledWith("git", [
        "checkout",
        "-b",
        branchName,
        "develop",
      ]);
    });
  });

  describe("Branch Type Workflows", () => {
    it("should create all branch types successfully", async () => {
      (execa as any).mockResolvedValue({ stdout: "", stderr: "", exitCode: 0 });

      const branchTypes = [
        { name: "feature/api-integration", base: "develop" },
        { name: "hotfix/critical-fix", base: "main" },
        { name: "bugfix/login-error", base: "develop" },
        { name: "release/2.0.0", base: "develop" },
      ];

      for (const { name, base } of branchTypes) {
        await git.createBranch(name, base);
        expect(execa).toHaveBeenCalledWith("git", [
          "checkout",
          "-b",
          name,
          base,
        ]);
      }
    });
  });

  describe("Error Recovery", () => {
    it("should handle branch already exists error", async () => {
      (execa as any).mockRejectedValue(new Error("Branch already exists"));

      await expect(git.createBranch("feature/existing")).rejects.toThrow(
        "Failed to create branch",
      );
    });

    it("should handle worktree creation failure gracefully", async () => {
      let callCount = 0;
      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          callCount++;
          if (callCount === 1) {
            // Branch creation succeeds
            return { stdout: "", stderr: "", exitCode: 0 };
          } else if (args?.[0] === "worktree" && args[1] === "add") {
            // Worktree creation fails
            throw new Error("Path already exists");
          }
          return { stdout: "", stderr: "", exitCode: 0 };
        },
      );

      await git.createBranch("feature/test");

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

  describe("Version Management", () => {
    it("should calculate versions correctly for releases", () => {
      const scenarios = [
        { from: "1.0.0", bump: "patch" as const, to: "1.0.1" },
        { from: "1.0.0", bump: "minor" as const, to: "1.1.0" },
        { from: "1.0.0", bump: "major" as const, to: "2.0.0" },
        { from: "0.9.9", bump: "minor" as const, to: "0.10.0" },
      ];

      scenarios.forEach(({ from, bump, to }) => {
        const result = git.calculateNewVersion(from, bump);
        expect(result).toBe(to);
      });
    });
  });
});
