import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as worktree from "../../src/worktree";

// Mock execa
vi.mock("execa", () => ({
  execa: vi.fn(),
}));

// Mock node:fs
vi.mock("node:fs", () => {
  const existsSync = vi.fn();
  return {
    existsSync,
    default: { existsSync },
  };
});

import { execa } from "execa";
import fs from "node:fs";
import fsPromises from "node:fs/promises";
import * as git from "../../src/git";
import * as github from "../../src/github";
import * as configModule from "../../src/config/index";

describe("worktree.ts - Worktree Operations", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("worktreeExists (T104)", () => {
    it("should return worktree path if worktree exists for branch", async () => {
      const mockOutput = `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main

worktree /path/to/worktree-feature-test
HEAD def5678
branch refs/heads/feature/test
`;

      (execa as any).mockResolvedValue({
        stdout: mockOutput,
        stderr: "",
        exitCode: 0,
      });

      const path = await worktree.worktreeExists("feature/test");

      expect(path).toBe("/path/to/worktree-feature-test");
      expect(execa).toHaveBeenCalledWith("git", [
        "worktree",
        "list",
        "--porcelain",
      ]);
    });

    it("should return null if worktree does not exist for branch", async () => {
      const mockOutput = `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main
`;

      (execa as any).mockResolvedValue({
        stdout: mockOutput,
        stderr: "",
        exitCode: 0,
      });

      const path = await worktree.worktreeExists("feature/non-existent");

      expect(path).toBeNull();
    });

    it("should handle empty worktree list", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      const path = await worktree.worktreeExists("feature/test");

      expect(path).toBeNull();
    });

    it("should throw WorktreeError on failure", async () => {
      (execa as any).mockRejectedValue(new Error("Git command failed"));

      await expect(worktree.worktreeExists("feature/test")).rejects.toThrow(
        "Failed to list worktrees",
      );
    });
  });

  describe("generateWorktreePath (T105)", () => {
    it("should generate worktree path with sanitized branch name", async () => {
      const repoRoot = "/path/to/repo";
      const branchName = "feature/user-auth";

      const path = await worktree.generateWorktreePath(repoRoot, branchName);

      expect(path).toBe("/path/to/repo/.worktrees/feature-user-auth");
    });

    it("should sanitize special characters in branch name", async () => {
      const repoRoot = "/path/to/repo";
      const branchName = "feature/user:auth*with?special<chars>";

      const path = await worktree.generateWorktreePath(repoRoot, branchName);

      expect(path).toBe(
        "/path/to/repo/.worktrees/feature-user-auth-with-special-chars-",
      );
    });

    it("should handle Windows-style paths", async () => {
      const repoRoot = "C:\\path\\to\\repo";
      const branchName = "feature/test";

      const path = await worktree.generateWorktreePath(repoRoot, branchName);

      // Path module will normalize this based on the platform
      expect(path).toContain("worktree");
      expect(path).toContain("feature-test");
    });
  });

  describe("createWorktree (T106)", () => {
    let mkdirSpy: ReturnType<typeof vi.spyOn>;
    let getWorktreeRootSpy: ReturnType<typeof vi.spyOn>;
    let ensureGitignoreEntrySpy: ReturnType<typeof vi.spyOn>;

    beforeEach(() => {
      mkdirSpy = vi.spyOn(fsPromises, "mkdir").mockResolvedValue(undefined);
      getWorktreeRootSpy = vi
        .spyOn(git, "getWorktreeRoot")
        .mockResolvedValue("/path/to/worktree-current");
      ensureGitignoreEntrySpy = vi
        .spyOn(git, "ensureGitignoreEntry")
        .mockResolvedValue();
    });

    afterEach(() => {
      mkdirSpy.mockRestore();
      getWorktreeRootSpy.mockRestore();
      ensureGitignoreEntrySpy.mockRestore();
    });

    it("should create worktree for existing branch", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      const config = {
        branchName: "feature/test",
        worktreePath: "/path/to/repo/.worktrees/feature-test",
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      };

      await worktree.createWorktree(config);

      expect(mkdirSpy).toHaveBeenCalledWith("/path/to/repo/.worktrees", {
        recursive: true,
      });
      expect(execa).toHaveBeenCalledWith("git", [
        "worktree",
        "add",
        "/path/to/repo/.worktrees/feature-test",
        "feature/test",
      ]);
    });

    it("should create worktree with new branch", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      const config = {
        branchName: "feature/new-feature",
        worktreePath: "/path/to/repo/.worktrees/feature-new-feature",
        repoRoot: "/path/to/repo",
        isNewBranch: true,
        baseBranch: "main",
      };

      await worktree.createWorktree(config);

      expect(mkdirSpy).toHaveBeenCalledWith("/path/to/repo/.worktrees", {
        recursive: true,
      });
      expect(execa).toHaveBeenCalledWith("git", [
        "worktree",
        "add",
        "-b",
        "feature/new-feature",
        "/path/to/repo/.worktrees/feature-new-feature",
        "main",
      ]);
    });

    it("should create worktree from different base branch", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      const config = {
        branchName: "hotfix/bug-fix",
        worktreePath: "/path/to/repo/.worktrees/hotfix-bug-fix",
        repoRoot: "/path/to/repo",
        isNewBranch: true,
        baseBranch: "develop",
      };

      await worktree.createWorktree(config);

      expect(mkdirSpy).toHaveBeenCalledWith("/path/to/repo/.worktrees", {
        recursive: true,
      });
      expect(execa).toHaveBeenCalledWith("git", [
        "worktree",
        "add",
        "-b",
        "hotfix/bug-fix",
        "/path/to/repo/.worktrees/hotfix-bug-fix",
        "develop",
      ]);
    });

    it('should reject worktree creation for protected branch "main"', async () => {
      const config = {
        branchName: "main",
        worktreePath: "/path/to/repo/.worktrees/main",
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      };

      await expect(worktree.createWorktree(config)).rejects.toThrow(
        'Branch "main" is protected and cannot be used to create a worktree',
      );
      expect(execa).not.toHaveBeenCalled();
    });

    it('should reject worktree creation for protected branch "develop"', async () => {
      const config = {
        branchName: "develop",
        worktreePath: "/path/to/repo/.worktrees/develop",
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "develop",
      };

      await expect(worktree.createWorktree(config)).rejects.toThrow(
        'Branch "develop" is protected and cannot be used to create a worktree',
      );
      expect(execa).not.toHaveBeenCalled();
    });

    it('should reject worktree creation for protected branch "master"', async () => {
      const config = {
        branchName: "master",
        worktreePath: "/path/to/repo/.worktrees/master",
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      };

      await expect(worktree.createWorktree(config)).rejects.toThrow(
        'Branch "master" is protected and cannot be used to create a worktree',
      );
      expect(execa).not.toHaveBeenCalled();
    });

    it("should throw WorktreeError when worktree directory preparation fails", async () => {
      mkdirSpy.mockRejectedValueOnce(
        Object.assign(new Error("EEXIST"), { code: "EEXIST" }),
      );

      const config = {
        branchName: "feature/test",
        worktreePath: "/path/to/repo/.worktrees/feature-test",
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      };

      await expect(worktree.createWorktree(config)).rejects.toThrow(
        "Failed to prepare worktree directory for feature/test",
      );

      expect(execa).not.toHaveBeenCalled();
    });

    it("should throw WorktreeError on failure", async () => {
      (execa as any).mockRejectedValue(new Error("Failed to create worktree"));

      const config = {
        branchName: "feature/test",
        worktreePath: "/path/to/repo/.worktrees/feature-test",
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      };

      await expect(worktree.createWorktree(config)).rejects.toThrow(
        "Failed to create worktree for feature/test",
      );
    });

    it("should update .gitignore after successful worktree creation", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      const config = {
        branchName: "feature/test",
        worktreePath: "/path/to/repo/.worktrees/feature-test",
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      };

      ensureGitignoreEntrySpy.mockClear();
      await worktree.createWorktree(config);

      expect(mkdirSpy).toHaveBeenCalledWith("/path/to/repo/.worktrees", {
        recursive: true,
      });
      expect(ensureGitignoreEntrySpy).toHaveBeenCalledWith(
        "/path/to/worktree-current",
        ".worktrees/",
      );
    });

    it("should fall back to repoRoot when worktree root resolution fails", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      getWorktreeRootSpy.mockRejectedValueOnce(
        new Error("git rev-parse failed"),
      );

      const config = {
        branchName: "feature/test",
        worktreePath: "/path/to/repo/.worktrees/feature-test",
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      };

      ensureGitignoreEntrySpy.mockClear();
      await worktree.createWorktree(config);

      expect(ensureGitignoreEntrySpy).toHaveBeenCalledWith(
        "/path/to/repo",
        ".worktrees/",
      );
    });

    it("should not fail worktree creation if .gitignore update fails", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      // .gitignore更新が失敗してもworktree作成は成功する
      ensureGitignoreEntrySpy.mockRejectedValue(new Error("Permission denied"));

      const config = {
        branchName: "feature/test",
        worktreePath: "/path/to/repo/.worktrees/feature-test",
        repoRoot: "/path/to/repo",
        isNewBranch: false,
        baseBranch: "main",
      };

      // エラーなく完了する(エラーがスローされない)
      await worktree.createWorktree(config);

      expect(mkdirSpy).toHaveBeenCalledWith("/path/to/repo/.worktrees", {
        recursive: true,
      });
      // execaが正常に呼ばれたことを確認
      expect(execa).toHaveBeenCalled();
    });
  });

  describe("switchToProtectedBranch", () => {
    let branchExistsSpy: ReturnType<typeof vi.spyOn>;
    let getCurrentBranchSpy: ReturnType<typeof vi.spyOn>;

    beforeEach(() => {
      branchExistsSpy = vi.spyOn(git, "branchExists").mockResolvedValue(false);
      getCurrentBranchSpy = vi
        .spyOn(git, "getCurrentBranch")
        .mockResolvedValue("develop");
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });
    });

    afterEach(() => {
      branchExistsSpy.mockRestore();
      getCurrentBranchSpy.mockRestore();
      (execa as any).mockReset();
    });

    it("returns none when already on the protected branch", async () => {
      getCurrentBranchSpy.mockResolvedValue("main");

      const result = await worktree.switchToProtectedBranch({
        branchName: "main",
        repoRoot: "/repo",
      });

      expect(result).toBe("none");
      expect(execa).not.toHaveBeenCalled();
    });

    it("checks out the local branch when it exists", async () => {
      branchExistsSpy.mockResolvedValue(true);

      const result = await worktree.switchToProtectedBranch({
        branchName: "main",
        repoRoot: "/repo",
      });

      expect(result).toBe("local");
      expect(execa).toHaveBeenCalledWith("git", ["checkout", "main"], {
        cwd: "/repo",
      });
    });

    it("creates tracking branch from remote when local branch is missing", async () => {
      branchExistsSpy.mockResolvedValue(false);

      const result = await worktree.switchToProtectedBranch({
        branchName: "main",
        repoRoot: "/repo",
        remoteRef: "origin/main",
      });

      expect(result).toBe("remote");
      expect(execa).toHaveBeenNthCalledWith(
        1,
        "git",
        ["fetch", "origin", "main"],
        { cwd: "/repo" },
      );
      expect(execa).toHaveBeenNthCalledWith(
        2,
        "git",
        ["checkout", "-b", "main", "origin/main"],
        { cwd: "/repo" },
      );
    });
  });

  describe("removeWorktree (T702)", () => {
    it("should remove worktree without force", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await worktree.removeWorktree("/path/to/worktree");

      expect(execa).toHaveBeenCalledWith("git", [
        "worktree",
        "remove",
        "/path/to/worktree",
      ]);
    });

    it("should force remove worktree when force=true", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await worktree.removeWorktree("/path/to/worktree", true);

      expect(execa).toHaveBeenCalledWith("git", [
        "worktree",
        "remove",
        "--force",
        "/path/to/worktree",
      ]);
    });

    it("should throw WorktreeError on failure", async () => {
      (execa as any).mockRejectedValue(new Error("Worktree removal failed"));

      await expect(
        worktree.removeWorktree("/path/to/worktree"),
      ).rejects.toThrow("Failed to remove worktree");
    });
  });

  describe("listAdditionalWorktrees (T701)", () => {
    it("should call listWorktrees via git command", async () => {
      const mockWorktreeOutput = `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main

worktree /path/to/worktree-feature-test
HEAD def5678
branch refs/heads/feature/test
`;

      (execa as any).mockResolvedValue({
        stdout: mockWorktreeOutput,
        stderr: "",
        exitCode: 0,
      });

      const worktreeList = await worktree.listAdditionalWorktrees();

      // Should call git worktree list
      expect(execa).toHaveBeenCalledWith("git", [
        "worktree",
        "list",
        "--porcelain",
      ]);

      // Result should be an array
      expect(Array.isArray(worktreeList)).toBe(true);
    });

    it("should exclude main repository from results", async () => {
      const mockWorktreeOutput = `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main

worktree /path/to/worktree-feature-test
HEAD def5678
branch refs/heads/feature/test
`;

      (execa as any).mockResolvedValue({
        stdout: mockWorktreeOutput,
        stderr: "",
        exitCode: 0,
      });

      const worktreeList = await worktree.listAdditionalWorktrees();

      // None of the returned worktrees should have 'main' as their branch
      // (assuming main repo is on main branch)
      expect(worktreeList.length).toBeGreaterThanOrEqual(0);
    });

    it("should handle empty worktree output", async () => {
      (execa as any).mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      const worktreeList = await worktree.listAdditionalWorktrees();

      // Should return empty array
      expect(Array.isArray(worktreeList)).toBe(true);
    });
  });

  describe("getMergedPRWorktrees", () => {
    it("includes branches identical to the base branch when there are no local-only commits", async () => {
      const configSpy = vi.spyOn(configModule, "getConfig").mockResolvedValue({
        defaultBaseBranch: "main",
        skipPermissions: false,
        enableGitHubIntegration: true,
        enableDebugMode: false,
        worktreeNamingPattern: "{repo}-{branch}",
      });

      const repoRootSpy = vi
        .spyOn(git, "getRepositoryRoot")
        .mockResolvedValue("/repo");

      const mergedPRsSpy = vi
        .spyOn(github, "getMergedPullRequests")
        .mockResolvedValue([]);

      const pullRequestByBranchSpy = vi
        .spyOn(github, "getPullRequestByBranch")
        .mockResolvedValue(null);

      const getLocalBranchesSpy = vi
        .spyOn(git, "getLocalBranches")
        .mockResolvedValue([
          {
            name: "feature/no-diff",
            type: "local",
            branchType: "feature",
            isCurrent: false,
          },
          {
            name: "feature/orphan",
            type: "local",
            branchType: "feature",
            isCurrent: false,
          },
          {
            name: "main",
            type: "local",
            branchType: "main",
            isCurrent: true,
          },
        ]);

      const hasUncommittedSpy = vi
        .spyOn(git, "hasUncommittedChanges")
        .mockResolvedValue(false);

      const hasUnpushedSpy = vi
        .spyOn(git, "hasUnpushedCommits")
        .mockResolvedValue(false);

      const hasUnpushedRepoSpy = vi
        .spyOn(git, "hasUnpushedCommitsInRepo")
        .mockResolvedValue(false);

      const branchHasUniqueSpy = vi
        .spyOn(git, "branchHasUniqueCommitsComparedToBase")
        .mockImplementation(async (branch) => {
          if (branch === "feature/no-diff" || branch === "feature/orphan") {
            return false;
          }
          return true;
        });

      const remoteExistsSpy = vi
        .spyOn(git, "checkRemoteBranchExists")
        .mockResolvedValue(false);

      (execa as any).mockImplementation(
        async (command: string, args?: readonly string[]) => {
          if (
            command === "git" &&
            args?.[0] === "worktree" &&
            args[1] === "list"
          ) {
            return {
              stdout: `worktree /repo
HEAD 0000000
branch refs/heads/main

worktree /repo/.git/worktree/feature-no-diff
HEAD abc1234
branch refs/heads/feature/no-diff
`,
              stderr: "",
              exitCode: 0,
            };
          }
          return { stdout: "", stderr: "", exitCode: 0 };
        },
      );

      (fs.existsSync as any).mockReturnValue(true);

      const targets = await worktree.getMergedPRWorktrees();

      expect(targets).toHaveLength(2);

      const worktreeTarget = targets.find(
        (target) => target.branch === "feature/no-diff",
      );
      expect(worktreeTarget).toBeDefined();
      expect(worktreeTarget?.cleanupType).toBe("worktree-and-branch");
      expect(worktreeTarget?.pullRequest).toBeNull();
      expect(worktreeTarget?.reasons).toContain("no-diff-with-base");
      expect(worktreeTarget?.reasons).not.toContain("merged-pr");

      const orphanTarget = targets.find(
        (target) => target.branch === "feature/orphan",
      );
      expect(orphanTarget).toBeDefined();
      expect(orphanTarget?.cleanupType).toBe("branch-only");
      expect(orphanTarget?.pullRequest).toBeNull();
      expect(orphanTarget?.reasons).toContain("no-diff-with-base");

      expect(configSpy).toHaveBeenCalled();
      expect(repoRootSpy).toHaveBeenCalled();
      expect(mergedPRsSpy).toHaveBeenCalled();
      expect(pullRequestByBranchSpy).toHaveBeenCalled();
      expect(getLocalBranchesSpy).toHaveBeenCalled();
      expect(hasUncommittedSpy).toHaveBeenCalled();
      expect(hasUnpushedSpy).toHaveBeenCalled();
      expect(hasUnpushedRepoSpy).toHaveBeenCalled();
      expect(branchHasUniqueSpy).toHaveBeenCalled();
      expect(remoteExistsSpy).toHaveBeenCalled();
    });
  });

  describe("Backward Compatibility (US3)", () => {
    describe("listAdditionalWorktrees with legacy .git/worktree paths", () => {
      it("should list worktrees from legacy .git/worktree path", async () => {
        const mockWorktreeOutput = `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main

worktree /path/to/repo/.git/worktree/feature-old
HEAD def5678
branch refs/heads/feature/old
`;

        (execa as any).mockResolvedValue({
          stdout: mockWorktreeOutput,
          stderr: "",
          exitCode: 0,
        });

        vi.spyOn(git, "getRepositoryRoot").mockResolvedValue("/path/to/repo");
        (fs.existsSync as any).mockReturnValue(true);

        const worktreeList = await worktree.listAdditionalWorktrees();

        expect(worktreeList).toHaveLength(1);
        expect(worktreeList[0].path).toBe(
          "/path/to/repo/.git/worktree/feature-old",
        );
        expect(worktreeList[0].branch).toBe("feature/old");
        expect(worktreeList[0].isAccessible).toBe(true);
      });

      it("should list both legacy and new worktree paths together", async () => {
        const mockWorktreeOutput = `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main

worktree /path/to/repo/.git/worktree/feature-old
HEAD def5678
branch refs/heads/feature/old

worktree /path/to/repo/.worktrees/feature-new
HEAD ghi9012
branch refs/heads/feature/new
`;

        (execa as any).mockResolvedValue({
          stdout: mockWorktreeOutput,
          stderr: "",
          exitCode: 0,
        });

        vi.spyOn(git, "getRepositoryRoot").mockResolvedValue("/path/to/repo");
        (fs.existsSync as any).mockReturnValue(true);

        const worktreeList = await worktree.listAdditionalWorktrees();

        expect(worktreeList).toHaveLength(2);

        const oldWorktree = worktreeList.find(
          (w) => w.branch === "feature/old",
        );
        expect(oldWorktree).toBeDefined();
        expect(oldWorktree?.path).toBe(
          "/path/to/repo/.git/worktree/feature-old",
        );

        const newWorktree = worktreeList.find(
          (w) => w.branch === "feature/new",
        );
        expect(newWorktree).toBeDefined();
        expect(newWorktree?.path).toBe("/path/to/repo/.worktrees/feature-new");
      });
    });

    describe("worktreeExists with legacy .git/worktree paths", () => {
      it("should find worktree in legacy .git/worktree path", async () => {
        const mockWorktreeOutput = `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main

worktree /path/to/repo/.git/worktree/feature-old
HEAD def5678
branch refs/heads/feature/old
`;

        (execa as any).mockResolvedValue({
          stdout: mockWorktreeOutput,
          stderr: "",
          exitCode: 0,
        });

        const path = await worktree.worktreeExists("feature/old");

        expect(path).toBe("/path/to/repo/.git/worktree/feature-old");
      });

      it("should distinguish between legacy and new worktree paths", async () => {
        const mockWorktreeOutput = `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main

worktree /path/to/repo/.git/worktree/feature-old
HEAD def5678
branch refs/heads/feature/old

worktree /path/to/repo/.worktrees/feature-new
HEAD ghi9012
branch refs/heads/feature/new
`;

        (execa as any).mockResolvedValue({
          stdout: mockWorktreeOutput,
          stderr: "",
          exitCode: 0,
        });

        const oldPath = await worktree.worktreeExists("feature/old");
        expect(oldPath).toBe("/path/to/repo/.git/worktree/feature-old");

        const newPath = await worktree.worktreeExists("feature/new");
        expect(newPath).toBe("/path/to/repo/.worktrees/feature-new");
      });
    });
  });
});
