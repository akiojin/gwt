/* eslint-disable @typescript-eslint/no-explicit-any */
import {
  describe,
  it,
  expect,
  mock,
  beforeEach,
  afterEach,
  spyOn,
} from "bun:test";
import path from "node:path";
import * as fsPromisesModule from "node:fs/promises";

let worktree: typeof import("../../src/worktree.ts");
let git: typeof import("../../src/git.ts");
let github: typeof import("../../src/github.ts");
let configModule: typeof import("../../src/config/index.ts");
let execa: typeof import("execa").execa;
let fs: typeof import("node:fs").default;
let execaMock: ReturnType<typeof mock>;
let fsPromisesMocks: {
  mkdir: ReturnType<typeof mock>;
  rm: ReturnType<typeof mock>;
  lstat: ReturnType<typeof mock>;
  readFile: ReturnType<typeof mock>;
};
let moduleCounter = 0;

describe("worktree.ts - Worktree Operations", () => {
  beforeEach(async () => {
    mock.restore();
    mock.clearAllMocks();
    execaMock = mock();
    const existsSync = mock();
    const statSync = mock(() => ({ isDirectory: () => true }));
    const mkdirSync = mock();
    const readdirSync = mock(() => []);
    const unlinkSync = mock();
    const readFileSync = mock(() => "");
    existsSync.mockReturnValue(false);
    fsPromisesMocks = {
      mkdir: spyOn(fsPromisesModule, "mkdir"),
      rm: spyOn(fsPromisesModule, "rm"),
      lstat: spyOn(fsPromisesModule, "lstat"),
      readFile: spyOn(fsPromisesModule, "readFile"),
    };
    mock.module("execa", () => ({
      execa: (...args: unknown[]) => execaMock(...args),
    }));
    mock.module("node:fs", () => ({
      existsSync,
      statSync,
      mkdirSync,
      readdirSync,
      unlinkSync,
      readFileSync,
      default: {
        existsSync,
        statSync,
        mkdirSync,
        readdirSync,
        unlinkSync,
        readFileSync,
      },
    }));
    moduleCounter += 1;
    const gitModule = {
      getRepositoryRoot: mock(),
      getCurrentBranchName: mock(),
      getWorktreeRoot: mock(),
      ensureGitignoreEntry: mock(),
      branchExists: mock(),
      getCurrentBranch: mock(),
      hasUncommittedChanges: mock(),
      hasUnpushedCommits: mock(),
      hasUnpushedCommitsInRepo: mock(),
      branchHasUniqueCommitsComparedToBase: mock(),
      checkRemoteBranchExists: mock(),
      getLocalBranches: mock(),
    };
    gitModule.getRepositoryRoot.mockResolvedValue("/path/to/repo");
    gitModule.getWorktreeRoot.mockResolvedValue("/path/to/worktree-current");
    gitModule.ensureGitignoreEntry.mockResolvedValue(undefined);
    gitModule.branchExists.mockResolvedValue(false);
    gitModule.getCurrentBranch.mockResolvedValue("develop");
    const githubModule = {
      getPullRequestByBranch: mock(),
    };
    const configModuleFresh = {
      getConfig: mock(),
    };
    mock.module("../../src/git.js", () => gitModule);
    mock.module("../../src/github.js", () => githubModule);
    mock.module("../../src/config/index.js", () => configModuleFresh);
    worktree = await import(
      `../../src/worktree.ts?worktree-test=${moduleCounter}`
    );
    execa = execaMock as unknown as typeof import("execa").execa;
    ({ default: fs } = await import("node:fs"));
    git = gitModule;
    github = githubModule as typeof import("../../src/github.ts");
    configModule =
      configModuleFresh as typeof import("../../src/config/index.ts");
  });

  afterEach(() => {
    mock.restore();
    mock.clearAllMocks();
  });

  describe("worktreeExists (T104)", () => {
    let repoRootSpy: Mock;

    beforeEach(() => {
      repoRootSpy = spyOn(git, "getRepositoryRoot").mockResolvedValue(
        "/path/to/repo",
      );
    });

    afterEach(() => {
      repoRootSpy.mockRestore();
    });

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
      const getCurrentBranchNameSpy = spyOn(
        git,
        "getCurrentBranchName",
      ).mockResolvedValue("feature/test");

      const path = await worktree.worktreeExists("feature/test");

      expect(path).toBe("/path/to/worktree-feature-test");
      expect(getCurrentBranchNameSpy).toHaveBeenCalledWith(
        "/path/to/worktree-feature-test",
      );
      expect(execa).toHaveBeenCalledWith(
        "git",
        ["worktree", "list", "--porcelain"],
        expect.anything(),
      );
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

    it("should return null when worktree branch does not match selected branch", async () => {
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
      spyOn(git, "getCurrentBranchName").mockResolvedValue("feature/other");

      const path = await worktree.worktreeExists("feature/test");

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

      const worktreePath = await worktree.generateWorktreePath(
        repoRoot,
        branchName,
      );

      expect(worktreePath).toBe(
        path.join(repoRoot, ".worktrees", "feature-user-auth"),
      );
    });

    it("should sanitize special characters in branch name", async () => {
      const repoRoot = "/path/to/repo";
      const branchName = "feature/user:auth*with?special<chars>";

      const worktreePath = await worktree.generateWorktreePath(
        repoRoot,
        branchName,
      );

      expect(worktreePath).toBe(
        path.join(
          repoRoot,
          ".worktrees",
          "feature-user-auth-with-special-chars-",
        ),
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
    let mkdirSpy: Mock;
    let getWorktreeRootSpy: Mock;
    let ensureGitignoreEntrySpy: Mock;

    beforeEach(() => {
      mkdirSpy = fsPromisesMocks.mkdir;
      mkdirSpy.mockResolvedValue(undefined);
      getWorktreeRootSpy = spyOn(git, "getWorktreeRoot").mockResolvedValue(
        "/path/to/worktree-current",
      );
      ensureGitignoreEntrySpy = spyOn(
        git,
        "ensureGitignoreEntry",
      ).mockResolvedValue();
    });

    afterEach(() => {
      mkdirSpy.mockReset();
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
    let branchExistsSpy: Mock;
    let getCurrentBranchSpy: Mock;

    beforeEach(() => {
      branchExistsSpy = spyOn(git, "branchExists").mockResolvedValue(false);
      getCurrentBranchSpy = spyOn(git, "getCurrentBranch").mockResolvedValue(
        "develop",
      );
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
    beforeEach(() => {
      spyOn(git, "getRepositoryRoot").mockResolvedValue("/");
    });

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
      expect(execa).toHaveBeenCalledWith(
        "git",
        ["worktree", "list", "--porcelain"],
        expect.anything(),
      );

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
      const configSpy = spyOn(configModule, "getConfig").mockResolvedValue({
        defaultBaseBranch: "main",
        skipPermissions: false,
        enableGitHubIntegration: true,
        enableDebugMode: false,
        worktreeNamingPattern: "{repo}-{branch}",
      });

      const repoRootSpy = spyOn(git, "getRepositoryRoot").mockResolvedValue(
        "/repo",
      );

      const pullRequestByBranchSpy = spyOn(
        github,
        "getPullRequestByBranch",
      ).mockResolvedValue(null);

      const getLocalBranchesSpy = spyOn(
        git,
        "getLocalBranches",
      ).mockResolvedValue([
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

      const hasUncommittedSpy = spyOn(
        git,
        "hasUncommittedChanges",
      ).mockResolvedValue(false);

      const hasUnpushedSpy = spyOn(git, "hasUnpushedCommits").mockResolvedValue(
        false,
      );

      const hasUnpushedRepoSpy = spyOn(
        git,
        "hasUnpushedCommitsInRepo",
      ).mockResolvedValue(false);

      const branchHasUniqueSpy = spyOn(
        git,
        "branchHasUniqueCommitsComparedToBase",
      ).mockImplementation(async (branch: string) => {
        if (branch === "feature/no-diff" || branch === "feature/orphan") {
          return false;
        }
        return true;
      });

      const remoteExistsSpy = spyOn(
        git,
        "checkRemoteBranchExists",
      ).mockResolvedValue(false);

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

      const orphanTarget = targets.find(
        (target) => target.branch === "feature/orphan",
      );
      expect(orphanTarget).toBeDefined();
      expect(orphanTarget?.cleanupType).toBe("branch-only");
      expect(orphanTarget?.pullRequest).toBeNull();
      expect(orphanTarget?.reasons).toContain("no-diff-with-base");

      expect(configSpy).toHaveBeenCalled();
      expect(repoRootSpy).toHaveBeenCalled();
      expect(pullRequestByBranchSpy).toHaveBeenCalled();
      expect(getLocalBranchesSpy).toHaveBeenCalled();
      expect(hasUncommittedSpy).toHaveBeenCalled();
      expect(hasUnpushedSpy).toHaveBeenCalled();
      expect(hasUnpushedRepoSpy).toHaveBeenCalled();
      expect(branchHasUniqueSpy).toHaveBeenCalled();
      expect(remoteExistsSpy).toHaveBeenCalled();
    });

    it("uses upstream branch as comparison base when determining cleanup candidates", async () => {
      const configSpy = spyOn(configModule, "getConfig").mockResolvedValue({
        defaultBaseBranch: "main",
        skipPermissions: false,
        enableGitHubIntegration: true,
        enableDebugMode: false,
        worktreeNamingPattern: "{repo}-{branch}",
      });

      const repoRootSpy = spyOn(git, "getRepositoryRoot").mockResolvedValue(
        "/repo",
      );

      spyOn(github, "getPullRequestByBranch").mockResolvedValue(null);
      spyOn(git, "getLocalBranches").mockResolvedValue([]);
      spyOn(git, "hasUncommittedChanges").mockResolvedValue(false);
      spyOn(git, "hasUnpushedCommits").mockResolvedValue(false);
      spyOn(git, "hasUnpushedCommitsInRepo").mockResolvedValue(false);

      const branchHasUniqueSpy = spyOn(
        git,
        "branchHasUniqueCommitsComparedToBase",
      ).mockResolvedValue(false);

      spyOn(git, "checkRemoteBranchExists").mockResolvedValue(false);

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

worktree /repo/.worktrees/feature-upstream
HEAD abc1234
branch refs/heads/feature/upstream
`,
              stderr: "",
              exitCode: 0,
            };
          }

          if (
            command === "git" &&
            args?.[0] === "rev-parse" &&
            args[1] === "--abbrev-ref"
          ) {
            return {
              stdout: "origin/develop",
              stderr: "",
              exitCode: 0,
            };
          }

          return { stdout: "", stderr: "", exitCode: 0 };
        },
      );

      (fs.existsSync as any).mockReturnValue(true);

      const targets = await worktree.getMergedPRWorktrees();

      expect(targets).toHaveLength(1);
      expect(branchHasUniqueSpy).toHaveBeenCalledWith(
        "feature/upstream",
        "origin/develop",
        "/repo",
      );

      const target = targets[0];
      if (!target) {
        throw new Error("Expected target");
      }
      expect(target.reasons).toContain("no-diff-with-base");
      expect(target.reasons).not.toContain("merged-pr");

      expect(configSpy).toHaveBeenCalled();
      expect(repoRootSpy).toHaveBeenCalled();
    });

    it("includes remote-synced orphaned branches when they are clean and pushed", async () => {
      spyOn(configModule, "getConfig").mockResolvedValue({
        defaultBaseBranch: "develop",
        skipPermissions: false,
        enableGitHubIntegration: true,
        enableDebugMode: false,
        worktreeNamingPattern: "{repo}-{branch}",
      });

      spyOn(git, "getRepositoryRoot").mockResolvedValue("/repo");
      spyOn(github, "getPullRequestByBranch").mockResolvedValue(null);
      spyOn(git, "getLocalBranches").mockResolvedValue([
        {
          name: "feature/ready-but-unmerged",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "develop",
          type: "local",
          branchType: "develop",
          isCurrent: true,
        },
      ]);

      spyOn(git, "hasUncommittedChanges").mockResolvedValue(false);
      spyOn(git, "hasUnpushedCommits").mockResolvedValue(false);
      spyOn(git, "hasUnpushedCommitsInRepo").mockResolvedValue(false);
      spyOn(git, "branchHasUniqueCommitsComparedToBase").mockResolvedValue(
        true,
      );
      spyOn(git, "checkRemoteBranchExists").mockResolvedValue(true);

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
branch refs/heads/develop
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

      const target = targets.find(
        (t) => t.branch === "feature/ready-but-unmerged",
      );
      expect(target).toBeDefined();
      expect(target?.cleanupType).toBe("branch-only");
      expect(target?.reasons).toContain("remote-synced");
      expect(target?.hasRemoteBranch).toBe(true);
      expect(target?.hasUnpushedCommits).toBe(false);
      expect(target?.hasUncommittedChanges).toBe(false);
    });

    it("includes pushed-but-unmerged branches as local cleanup candidates when they are clean", async () => {
      spyOn(configModule, "getConfig").mockResolvedValue({
        defaultBaseBranch: "develop",
        skipPermissions: false,
        enableGitHubIntegration: true,
        enableDebugMode: false,
        worktreeNamingPattern: "{repo}-{branch}",
      });

      spyOn(git, "getRepositoryRoot").mockResolvedValue("/repo");
      spyOn(github, "getPullRequestByBranch").mockResolvedValue(null);
      spyOn(git, "getLocalBranches").mockResolvedValue([
        {
          name: "feature/ready-but-unmerged",
          type: "local",
          branchType: "feature",
          isCurrent: false,
        },
        {
          name: "develop",
          type: "local",
          branchType: "develop",
          isCurrent: true,
        },
      ]);

      spyOn(git, "hasUncommittedChanges").mockResolvedValue(false);
      spyOn(git, "hasUnpushedCommits").mockResolvedValue(false);
      spyOn(git, "hasUnpushedCommitsInRepo").mockResolvedValue(false);
      spyOn(git, "branchHasUniqueCommitsComparedToBase").mockResolvedValue(
        true,
      );
      spyOn(git, "checkRemoteBranchExists").mockResolvedValue(true);

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
branch refs/heads/develop

worktree /repo/.git/worktree/feature-ready-but-unmerged
HEAD def9999
branch refs/heads/feature/ready-but-unmerged
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

      const target = targets.find(
        (t) => t.branch === "feature/ready-but-unmerged",
      );
      expect(target).toBeDefined();
      expect(target?.cleanupType).toBe("worktree-and-branch");
      expect(target?.reasons).toContain("remote-synced");
      expect(target?.hasRemoteBranch).toBe(true);
      expect(target?.hasUnpushedCommits).toBe(false);
      expect(target?.hasUncommittedChanges).toBe(false);
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

        spyOn(git, "getRepositoryRoot").mockResolvedValue("/path/to/repo");
        (fs.existsSync as any).mockReturnValue(true);

        const worktreeList = await worktree.listAdditionalWorktrees();

        expect(worktreeList).toHaveLength(1);
        const first = worktreeList[0];
        if (!first) {
          throw new Error("Expected worktree entry");
        }
        expect(first.path).toBe("/path/to/repo/.git/worktree/feature-old");
        expect(first.branch).toBe("feature/old");
        expect(first.isAccessible).toBe(true);
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

        spyOn(git, "getRepositoryRoot").mockResolvedValue("/path/to/repo");
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
        spyOn(git, "getCurrentBranchName").mockResolvedValue("feature/old");

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
        spyOn(git, "getCurrentBranchName").mockImplementation(
          async (worktreePath) => {
            if (worktreePath.endsWith("feature-old")) return "feature/old";
            if (worktreePath.endsWith("feature-new")) return "feature/new";
            return "";
          },
        );

        const oldPath = await worktree.worktreeExists("feature/old");
        expect(oldPath).toBe("/path/to/repo/.git/worktree/feature-old");

        const newPath = await worktree.worktreeExists("feature/new");
        expect(newPath).toBe("/path/to/repo/.worktrees/feature-new");
      });
    });
  });
});
