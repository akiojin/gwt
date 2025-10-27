import { execa } from "execa";
import path from "node:path";
import { BranchInfo } from "./ui/types.js";

export class GitError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "GitError";
  }
}

/**
 * 現在のディレクトリがGitリポジトリかどうかを確認
 * Worktree環境でも動作するように、.gitファイルの存在も確認します
 * @returns {Promise<boolean>} Gitリポジトリの場合true
 */
export async function isGitRepository(): Promise<boolean> {
  try {
    // まず.gitの存在を確認（ディレクトリまたはファイル）
    const fs = await import("node:fs");
    const gitPath = path.join(process.cwd(), ".git");

    if (fs.existsSync(gitPath)) {
      // .gitが存在する場合、Git環境として認識
      if (process.env.DEBUG) {
        const stats = fs.statSync(gitPath);
        console.error(
          `[DEBUG] .git exists: ${gitPath} (${stats.isDirectory() ? "directory" : "file"})`,
        );
      }
      return true;
    }

    // .gitが存在しない場合、git rev-parseで確認
    const result = await execa("git", ["rev-parse", "--git-dir"]);
    if (process.env.DEBUG) {
      console.error(`[DEBUG] git rev-parse --git-dir: ${result.stdout}`);
    }
    return true;
  } catch (error: any) {
    // Debug: log the error for troubleshooting
    if (process.env.DEBUG) {
      console.error(`[DEBUG] git rev-parse --git-dir failed:`, error.message);
      if (error.stderr) {
        console.error(`[DEBUG] stderr:`, error.stderr);
      }
    }
    return false;
  }
}

/**
 * Gitリポジトリのルートディレクトリを取得
 * @returns {Promise<string>} リポジトリのルートパス
 * @throws {GitError} リポジトリルートの取得に失敗した場合
 */
export async function getRepositoryRoot(): Promise<string> {
  try {
    // git rev-parse --git-common-dirを使用してメインリポジトリの.gitディレクトリを取得
    const { stdout: gitCommonDir } = await execa("git", [
      "rev-parse",
      "--git-common-dir",
    ]);
    const gitDir = gitCommonDir.trim();

    // .gitディレクトリの親ディレクトリがリポジトリルート
    const path = await import("node:path");
    const repoRoot = path.dirname(gitDir);

    // 相対パスが返された場合（.gitなど）は、現在のディレクトリからの相対パスとして解決
    if (!path.isAbsolute(repoRoot)) {
      return path.resolve(repoRoot);
    }

    return repoRoot;
  } catch (error) {
    throw new GitError("Failed to get repository root", error);
  }
}

export async function getRemoteBranches(): Promise<BranchInfo[]> {
  try {
    const { stdout } = await execa("git", [
      "branch",
      "-r",
      "--format=%(refname:short)",
    ]);
    return stdout
      .split("\n")
      .filter((line) => line.trim() && !line.includes("HEAD"))
      .map((line) => {
        const name = line.trim();
        const branchName = name.replace(/^origin\//, "");
        return {
          name,
          type: "remote" as const,
          branchType: getBranchType(branchName),
          isCurrent: false,
        };
      });
  } catch (error) {
    throw new GitError("Failed to get remote branches", error);
  }
}

async function getCurrentBranch(): Promise<string | null> {
  try {
    const { stdout } = await execa("git", ["branch", "--show-current"]);
    return stdout.trim() || null;
  } catch {
    return null;
  }
}

export async function getLocalBranches(): Promise<BranchInfo[]> {
  try {
    const { stdout } = await execa("git", [
      "branch",
      "--format=%(refname:short)",
    ]);
    return stdout
      .split("\n")
      .filter((line) => line.trim())
      .map((name) => ({
        name: name.trim(),
        type: "local" as const,
        branchType: getBranchType(name.trim()),
        isCurrent: false,
      }));
  } catch (error) {
    throw new GitError("Failed to get local branches", error);
  }
}

/**
 * ローカルとリモートのすべてのブランチ情報を取得
 * @returns {Promise<BranchInfo[]>} ブランチ情報の配列
 */
export async function getAllBranches(): Promise<BranchInfo[]> {
  const [localBranches, remoteBranches, currentBranch] = await Promise.all([
    getLocalBranches(),
    getRemoteBranches(),
    getCurrentBranch(),
  ]);

  // 現在のブランチ情報を設定
  if (currentBranch) {
    localBranches.forEach((branch) => {
      if (branch.name === currentBranch) {
        branch.isCurrent = true;
      }
    });
  }

  return [...localBranches, ...remoteBranches];
}

export async function createBranch(
  branchName: string,
  baseBranch = "main",
): Promise<void> {
  try {
    await execa("git", ["checkout", "-b", branchName, baseBranch]);
  } catch (error) {
    throw new GitError(`Failed to create branch ${branchName}`, error);
  }
}

export async function branchExists(branchName: string): Promise<boolean> {
  try {
    await execa("git", [
      "show-ref",
      "--verify",
      "--quiet",
      `refs/heads/${branchName}`,
    ]);
    return true;
  } catch {
    return false;
  }
}

export async function deleteBranch(
  branchName: string,
  force = false,
): Promise<void> {
  try {
    const args = ["branch", force ? "-D" : "-d", branchName];
    await execa("git", args);
  } catch (error) {
    throw new GitError(`Failed to delete branch ${branchName}`, error);
  }
}

interface WorktreeStatusResult {
  hasChanges: boolean;
  changedFilesCount: number;
}

async function getWorkdirStatus(
  worktreePath: string,
): Promise<WorktreeStatusResult> {
  try {
    // ファイルシステムの存在確認のためにfs.existsSyncを使用
    const fs = await import("node:fs");
    if (!fs.existsSync(worktreePath)) {
      // worktreeパスが存在しない場合はデフォルト値を返す
      return {
        hasChanges: false,
        changedFilesCount: 0,
      };
    }

    const { stdout } = await execa("git", ["status", "--porcelain"], {
      cwd: worktreePath,
    });
    const lines = stdout.split("\n").filter((line) => line.trim());
    return {
      hasChanges: lines.length > 0,
      changedFilesCount: lines.length,
    };
  } catch (error) {
    throw new GitError(
      `Failed to get worktree status for path: ${worktreePath}`,
      error,
    );
  }
}

export async function hasUncommittedChanges(
  worktreePath: string,
): Promise<boolean> {
  const status = await getWorkdirStatus(worktreePath);
  return status.hasChanges;
}

export async function getChangedFilesCount(
  worktreePath: string,
): Promise<number> {
  const status = await getWorkdirStatus(worktreePath);
  return status.changedFilesCount;
}

export async function showStatus(worktreePath: string): Promise<string> {
  try {
    const { stdout } = await execa("git", ["status"], { cwd: worktreePath });
    return stdout;
  } catch (error) {
    throw new GitError("Failed to show status", error);
  }
}

export async function stashChanges(
  worktreePath: string,
  message?: string,
): Promise<void> {
  try {
    const args = message ? ["stash", "push", "-m", message] : ["stash"];
    await execa("git", args, { cwd: worktreePath });
  } catch (error) {
    throw new GitError("Failed to stash changes", error);
  }
}

export async function discardAllChanges(worktreePath: string): Promise<void> {
  try {
    // Reset tracked files
    await execa("git", ["reset", "--hard"], { cwd: worktreePath });
    // Clean untracked files
    await execa("git", ["clean", "-fd"], { cwd: worktreePath });
  } catch (error) {
    throw new GitError("Failed to discard changes", error);
  }
}

export async function commitChanges(
  worktreePath: string,
  message: string,
): Promise<void> {
  try {
    // Add all changes
    await execa("git", ["add", "-A"], { cwd: worktreePath });
    // Commit
    await execa("git", ["commit", "-m", message], { cwd: worktreePath });
  } catch (error) {
    throw new GitError("Failed to commit changes", error);
  }
}

function getBranchType(branchName: string): BranchInfo["branchType"] {
  if (branchName === "main" || branchName === "master") return "main";
  if (branchName === "develop" || branchName === "dev") return "develop";
  if (branchName.startsWith("feature/")) return "feature";
  if (branchName.startsWith("hotfix/")) return "hotfix";
  if (branchName.startsWith("release/")) return "release";
  return "other";
}

export async function hasUnpushedCommits(
  worktreePath: string,
  branch: string,
): Promise<boolean> {
  try {
    const { stdout } = await execa(
      "git",
      ["log", `origin/${branch}..${branch}`, "--oneline"],
      { cwd: worktreePath },
    );
    return stdout.trim().length > 0;
  } catch {
    const candidates = [
      `origin/${branch}`,
      "origin/main",
      "origin/master",
      "origin/develop",
      "origin/dev",
      branch,
      "main",
      "master",
      "develop",
      "dev",
    ];

    for (const candidate of candidates) {
      try {
        await execa("git", ["rev-parse", "--verify", candidate], {
          cwd: worktreePath,
        });

        // If we are checking the same branch again, we already know the remote ref is missing.
        if (candidate === `origin/${branch}` || candidate === branch) {
          continue;
        }

        try {
          await execa(
            "git",
            ["merge-base", "--is-ancestor", branch, candidate],
            { cwd: worktreePath },
          );
          return false;
        } catch {
          // Not merged into this candidate, try next one.
        }
      } catch {
        // Candidate ref does not exist. Try the next candidate.
      }
    }

    // Could not prove that the branch is merged anywhere safe, treat as unpushed commits.
    return true;
  }
}

/**
 * Get the latest commit message for a specific branch in a worktree
 */
export async function getLatestCommitMessage(
  worktreePath: string,
  branch: string,
): Promise<string | null> {
  try {
    const { stdout } = await execa(
      "git",
      ["log", "-1", "--pretty=format:%s", branch],
      { cwd: worktreePath },
    );
    return stdout.trim() || null;
  } catch {
    return null;
  }
}

/**
 * Get the count of unpushed commits
 */
export async function getUnpushedCommitsCount(
  worktreePath: string,
  branch: string,
): Promise<number> {
  try {
    const { stdout } = await execa(
      "git",
      ["rev-list", "--count", `origin/${branch}..${branch}`],
      { cwd: worktreePath },
    );
    return parseInt(stdout.trim()) || 0;
  } catch {
    return 0;
  }
}

/**
 * Get the count of uncommitted changes (staged + unstaged)
 */
export async function getUncommittedChangesCount(
  worktreePath: string,
): Promise<number> {
  try {
    const { stdout } = await execa("git", ["status", "--porcelain"], {
      cwd: worktreePath,
    });
    return stdout
      .trim()
      .split("\n")
      .filter((line) => line.trim()).length;
  } catch {
    return 0;
  }
}

/**
 * Enhanced session information for better display
 */
export interface EnhancedSessionInfo {
  hasUncommittedChanges: boolean;
  uncommittedChangesCount: number;
  hasUnpushedCommits: boolean;
  unpushedCommitsCount: number;
  latestCommitMessage: string | null;
  branchType:
    | "feature"
    | "bugfix"
    | "hotfix"
    | "develop"
    | "main"
    | "master"
    | "other";
}

/**
 * Get enhanced session information for display
 */
export async function getEnhancedSessionInfo(
  worktreePath: string,
  branch: string,
): Promise<EnhancedSessionInfo> {
  try {
    const [
      hasUncommitted,
      uncommittedCount,
      hasUnpushed,
      unpushedCount,
      latestCommit,
    ] = await Promise.all([
      hasUncommittedChanges(worktreePath),
      getUncommittedChangesCount(worktreePath),
      hasUnpushedCommits(worktreePath, branch),
      getUnpushedCommitsCount(worktreePath, branch),
      getLatestCommitMessage(worktreePath, branch),
    ]);

    // Determine branch type based on branch name
    let branchType: EnhancedSessionInfo["branchType"] = "other";
    const lowerBranch = branch.toLowerCase();

    if (lowerBranch.startsWith("feature/") || lowerBranch.startsWith("feat/")) {
      branchType = "feature";
    } else if (
      lowerBranch.startsWith("bugfix/") ||
      lowerBranch.startsWith("bug/") ||
      lowerBranch.startsWith("fix/")
    ) {
      branchType = "bugfix";
    } else if (lowerBranch.startsWith("hotfix/")) {
      branchType = "hotfix";
    } else if (lowerBranch === "develop" || lowerBranch === "development") {
      branchType = "develop";
    } else if (lowerBranch === "main") {
      branchType = "main";
    } else if (lowerBranch === "master") {
      branchType = "master";
    }

    return {
      hasUncommittedChanges: hasUncommitted,
      uncommittedChangesCount: uncommittedCount,
      hasUnpushedCommits: hasUnpushed,
      unpushedCommitsCount: unpushedCount,
      latestCommitMessage: latestCommit,
      branchType,
    };
  } catch (error) {
    // Return safe defaults if any operation fails
    return {
      hasUncommittedChanges: false,
      uncommittedChangesCount: 0,
      hasUnpushedCommits: false,
      unpushedCommitsCount: 0,
      latestCommitMessage: null,
      branchType: "other",
    };
  }
}

export async function fetchAllRemotes(): Promise<void> {
  try {
    await execa("git", ["fetch", "--all", "--prune"]);
  } catch (error) {
    throw new GitError("Failed to fetch remote branches", error);
  }
}

export async function getCurrentVersion(repoRoot: string): Promise<string> {
  try {
    const packageJsonPath = path.join(repoRoot, "package.json");
    const fs = await import("node:fs");
    const packageJson = JSON.parse(
      await fs.promises.readFile(packageJsonPath, "utf-8"),
    );
    return packageJson.version || "0.0.0";
  } catch (error) {
    // package.jsonが存在しない場合はデフォルトバージョンを返す
    return "0.0.0";
  }
}

export function calculateNewVersion(
  currentVersion: string,
  versionBump: "patch" | "minor" | "major",
): string {
  const versionParts = currentVersion.split(".");
  const major = parseInt(versionParts[0] || "0");
  const minor = parseInt(versionParts[1] || "0");
  const patch = parseInt(versionParts[2] || "0");

  switch (versionBump) {
    case "major":
      return `${major + 1}.0.0`;
    case "minor":
      return `${major}.${minor + 1}.0`;
    case "patch":
      return `${major}.${minor}.${patch + 1}`;
  }
}

export async function executeNpmVersionInWorktree(
  worktreePath: string,
  newVersion: string,
): Promise<void> {
  try {
    // まずpackage.jsonが存在するか確認
    const fs = await import("node:fs");
    const packageJsonPath = path.join(worktreePath, "package.json");

    if (!fs.existsSync(packageJsonPath)) {
      // package.jsonが存在しない場合は作成
      const packageJson = {
        name: path.basename(worktreePath),
        version: newVersion,
        description: "",
        main: "index.js",
        scripts: {},
        keywords: [],
        author: "",
        license: "ISC",
      };
      await fs.promises.writeFile(
        packageJsonPath,
        JSON.stringify(packageJson, null, 2) + "\n",
      );

      // 新規作成したpackage.jsonをコミット
      await execa("git", ["add", "package.json"], { cwd: worktreePath });
      await execa(
        "git",
        [
          "commit",
          "-m",
          `chore: create package.json with version ${newVersion}`,
        ],
        { cwd: worktreePath },
      );
    } else {
      // package.json の version を直接書き換え（外部PMに依存しない）
      const content = await fs.promises.readFile(packageJsonPath, "utf-8");
      const json = JSON.parse(content);
      json.version = newVersion;
      await fs.promises.writeFile(
        packageJsonPath,
        JSON.stringify(json, null, 2) + "\n",
      );

      // 変更をコミット
      await execa("git", ["add", "package.json"], { cwd: worktreePath });
      await execa(
        "git",
        ["commit", "-m", `chore: bump version to ${newVersion}`],
        { cwd: worktreePath },
      );
    }
  } catch (error: any) {
    // エラーの詳細情報を含める
    const errorMessage = error instanceof Error ? error.message : String(error);
    const errorDetails = error?.stderr ? ` (stderr: ${error.stderr})` : "";
    const errorStdout = error?.stdout ? ` (stdout: ${error.stdout})` : "";
    throw new GitError(
      `Failed to update version to ${newVersion} in worktree: ${errorMessage}${errorDetails}${errorStdout}`,
      error,
    );
  }
}

export async function deleteRemoteBranch(
  branchName: string,
  remote = "origin",
): Promise<void> {
  try {
    await execa("git", ["push", remote, "--delete", branchName]);
  } catch (error) {
    throw new GitError(
      `Failed to delete remote branch ${remote}/${branchName}`,
      error,
    );
  }
}

export async function getCurrentBranchName(
  worktreePath: string,
): Promise<string> {
  try {
    const { stdout } = await execa("git", ["branch", "--show-current"], {
      cwd: worktreePath,
    });
    return stdout.trim();
  } catch (error) {
    throw new GitError("Failed to get current branch name", error);
  }
}

export async function pushBranchToRemote(
  worktreePath: string,
  branchName: string,
  remote = "origin",
): Promise<void> {
  try {
    // Check if the remote branch exists
    const remoteBranchExists = await checkRemoteBranchExists(
      branchName,
      remote,
    );

    if (remoteBranchExists) {
      // Push to existing remote branch
      await execa("git", ["push", remote, branchName], { cwd: worktreePath });
    } else {
      // Push and set upstream for new remote branch
      await execa("git", ["push", "--set-upstream", remote, branchName], {
        cwd: worktreePath,
      });
    }
  } catch (error) {
    throw new GitError(
      `Failed to push branch ${branchName} to ${remote}`,
      error,
    );
  }
}

export async function checkRemoteBranchExists(
  branchName: string,
  remote = "origin",
): Promise<boolean> {
  try {
    await execa("git", [
      "show-ref",
      "--verify",
      "--quiet",
      `refs/remotes/${remote}/${branchName}`,
    ]);
    return true;
  } catch {
    return false;
  }
}

/**
 * 現在のディレクトリがworktreeディレクトリかどうかを確認
 * @returns {Promise<boolean>} worktreeディレクトリの場合true
 */
export async function isInWorktree(): Promise<boolean> {
  try {
    // git rev-parse --show-toplevelとgit rev-parse --git-common-dirの結果を比較
    const [toplevelResult, gitCommonDirResult] = await Promise.all([
      execa("git", ["rev-parse", "--show-toplevel"]),
      execa("git", ["rev-parse", "--git-common-dir"]),
    ]);

    const toplevel = toplevelResult.stdout.trim();
    const gitCommonDir = gitCommonDirResult.stdout.trim();

    // gitCommonDirが絶対パスで、toplevelと異なる親ディレクトリを持つ場合はworktree
    const path = await import("node:path");
    if (path.isAbsolute(gitCommonDir)) {
      const mainRepoRoot = path.dirname(gitCommonDir);
      return toplevel !== mainRepoRoot;
    }

    // gitCommonDirが相対パス（.git）の場合はメインリポジトリ
    return false;
  } catch {
    return false;
  }
}
