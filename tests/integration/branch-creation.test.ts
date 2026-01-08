import { describe, it, expect, mock, beforeEach, afterEach } from "bun:test";
import * as git from "../../src/git";
import * as worktree from "../../src/worktree";
import { existsSync } from "node:fs";
import { mkdir, lstat, readFile, rm, writeFile } from "node:fs/promises";

// Mock execa
mock.module("execa", () => ({
  execa: mock(),
}));

mock.module("node:fs", () => ({
  existsSync: mock(() => false),
}));

mock.module("node:fs/promises", () => {
  const mkdir = mock(async () => undefined);
  const lstat = mock();
  const readFile = mock();
  const rm = mock(async () => undefined);
  const writeFile = mock(async () => undefined);

  return {
    mkdir,
    lstat,
    readFile,
    rm,
    writeFile,
    default: {
      mkdir,
      lstat,
      readFile,
      rm,
      writeFile,
    },
  };
});

import { execa } from "execa";

const execaMock = execa as unknown as Mock;
const existsSyncMock = existsSync as unknown as Mock;
const mkdirMock = mkdir as unknown as Mock;
const lstatMock = lstat as unknown as Mock;
const readFileMock = readFile as unknown as Mock;
const rmMock = rm as unknown as Mock;
const writeFileMock = writeFile as unknown as Mock;
const normalizePath = (value: string) => value.replace(/\\/g, "/");

describe("Integration: Branch Creation Workflow (T207)", () => {
  beforeEach(() => {
    mock.restore();
    mkdirMock.mockClear();
    rmMock.mockClear();
    writeFileMock.mockClear();
    existsSyncMock.mockReset();
    existsSyncMock.mockReturnValue(false);
    lstatMock.mockReset();
    readFileMock.mockReset();
    readFileMock.mockResolvedValue("");
  });

  afterEach(() => {
    mock.restore();
  });

  describe("Feature Branch Creation Flow", () => {
    it("should create feature branch and worktree", async () => {
      execaMock.mockImplementation(
        async (_command: string, args?: readonly string[]) => {
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
      execaMock.mockImplementation(
        async (_command: string, args?: readonly string[]) => {
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

      execaMock.mockResolvedValue({ stdout: "", stderr: "", exitCode: 0 });

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
      execaMock.mockRejectedValue(new Error("Branch already exists"));

      await expect(git.createBranch("feature/duplicate")).rejects.toThrow(
        "Failed to create branch",
      );
    });

    it("should handle worktree creation failure after branch creation", async () => {
      let callCount = 0;
      execaMock.mockImplementation(
        async (_command: string, _args?: readonly string[]) => {
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

  describe("Stale Worktree Directory Recovery", () => {
    it("should remove stale directory and retry worktree creation", async () => {
      const repoRoot = "/path/to/repo";
      const branchName = "feature/stale";
      const worktreePath = await worktree.generateWorktreePath(
        repoRoot,
        branchName,
      );

      execaMock.mockImplementation(
        async (_command: string, args?: readonly string[]) => {
          if (args?.[0] === "worktree" && args[1] === "list") {
            return {
              stdout: `worktree ${repoRoot}
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

      existsSyncMock.mockImplementation((targetPath: string) => {
        const normalizedTarget = normalizePath(targetPath);
        const normalizedWorktree = normalizePath(worktreePath);
        if (normalizedTarget === normalizedWorktree) return true;
        if (normalizedTarget === `${normalizedWorktree}/.git`) return false;
        return false;
      });

      await worktree.createWorktree({
        branchName,
        worktreePath,
        repoRoot,
        isNewBranch: false,
        baseBranch: "develop",
      });

      expect(rmMock).toHaveBeenCalledWith(worktreePath, {
        recursive: true,
        force: true,
      });
      expect(execa).toHaveBeenCalledWith(
        "git",
        expect.arrayContaining(["worktree", "add"]),
      );
    });

    it("should abort when existing directory is not stale", async () => {
      const repoRoot = "/path/to/repo";
      const branchName = "feature/non-stale";
      const worktreePath = await worktree.generateWorktreePath(
        repoRoot,
        branchName,
      );

      execaMock.mockImplementation(
        async (_command: string, args?: readonly string[]) => {
          if (args?.[0] === "worktree" && args[1] === "list") {
            return {
              stdout: `worktree ${repoRoot}
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

      existsSyncMock.mockImplementation((targetPath: string) => {
        const normalizedTarget = normalizePath(targetPath);
        const normalizedWorktree = normalizePath(worktreePath);
        if (normalizedTarget === normalizedWorktree) return true;
        if (normalizedTarget === `${normalizedWorktree}/.git`) return true;
        return false;
      });

      lstatMock.mockResolvedValue({
        isFile: () => false,
        isDirectory: () => true,
      });

      await expect(
        worktree.createWorktree({
          branchName,
          worktreePath,
          repoRoot,
          isNewBranch: false,
          baseBranch: "develop",
        }),
      ).rejects.toThrow("stale");

      expect(rmMock).not.toHaveBeenCalled();
      expect(execa).not.toHaveBeenCalledWith(
        "git",
        expect.arrayContaining(["worktree", "add"]),
      );
    });
  });
});
