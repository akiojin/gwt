import { execa } from "execa";
import path from "node:path";
import { BranchInfo } from "./cli/ui/types.js";

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
  const fs = await import("node:fs");
  const path = await import("node:path");

  const toExistingDir = (p: string): string => {
    let current = path.resolve(p);
    while (!fs.existsSync(current) && path.dirname(current) !== current) {
      current = path.dirname(current);
    }
    if (!fs.existsSync(current)) {
      return current;
    }
    try {
      const stat = fs.statSync(current);
      return stat.isDirectory() ? current : path.dirname(current);
    } catch {
      return current;
    }
  };

  // 1) show-toplevel を最優先
  try {
    const { stdout } = await execa("git", ["rev-parse", "--show-toplevel"]);
    const top = stdout.trim();
    if (top) {
      const marker = `${path.sep}.worktrees${path.sep}`;
      const idx = top.indexOf(marker);
      if (idx >= 0) {
        // /repo/.worktrees/<name> → repo root = /repo
        const parent = top.slice(0, idx);
        const upTwo = path.resolve(top, "..", "..");
        if (fs.existsSync(parent)) return toExistingDir(parent);
        if (fs.existsSync(upTwo)) return toExistingDir(upTwo);
      }
      if (fs.existsSync(top)) {
        return toExistingDir(top);
      }
      return toExistingDir(path.resolve(top, ".."));
    }
  } catch {
    // fallback
  }

  // 2) git-common-dir 経由
  try {
    const { stdout: gitCommonDir } = await execa("git", [
      "rev-parse",
      "--git-common-dir",
    ]);
    const gitDir = path.resolve(gitCommonDir.trim());
    const parts = gitDir.split(path.sep);
    const idxWorktrees = parts.lastIndexOf("worktrees");
    if (idxWorktrees > 0) {
      const repo = parts.slice(0, idxWorktrees).join(path.sep);
      if (fs.existsSync(repo)) return toExistingDir(repo);
    }
    const idxGit = parts.lastIndexOf(".git");
    if (idxGit > 0) {
      const repo = parts.slice(0, idxGit).join(path.sep);
      if (fs.existsSync(repo)) return toExistingDir(repo);
    }
    return toExistingDir(path.dirname(gitDir));
  } catch {
    // fallback
  }

  // 3) .git を上に辿って探す
  let current = process.cwd();
  const root = path.parse(current).root;
  while (true) {
    const candidate = path.join(current, ".git");
    if (fs.existsSync(candidate)) {
      if (fs.statSync(candidate).isDirectory()) {
        return toExistingDir(current);
      }
      try {
        const content = fs.readFileSync(candidate, "utf-8");
        const match = content.match(/gitdir:\s*(.+)\s*/i);
        const gitDirMatch = match?.[1];
        if (gitDirMatch) {
          const gitDirPath = path.resolve(current, gitDirMatch.trim());
          const parts = gitDirPath.split(path.sep);
          const idxWT = parts.lastIndexOf("worktrees");
          if (idxWT > 0) {
            const repo = parts.slice(0, idxWT).join(path.sep);
            return toExistingDir(repo);
          }
          return toExistingDir(path.dirname(gitDirPath));
        }
      } catch {
        // ignore and continue
      }
    }
    if (current === root) break;
    current = path.dirname(current);
  }

  throw new GitError("Failed to get repository root");
}

/**
 * 現在の作業ツリー(root branch か worktree かを問わず)のルートディレクトリを取得
 * @returns {Promise<string>} カレントWorktreeのルートパス
 * @throws {GitError} 取得に失敗した場合
 */
export async function getWorktreeRoot(): Promise<string> {
  try {
    const { stdout } = await execa("git", ["rev-parse", "--show-toplevel"]);
    return stdout.trim();
  } catch (error) {
    throw new GitError("Failed to get worktree root", error);
  }
}

export async function getCurrentBranch(): Promise<string | null> {
  try {
    const repoRoot = await getRepositoryRoot();
    const { stdout } = await execa("git", ["branch", "--show-current"], {
      cwd: repoRoot,
    });
    return stdout.trim() || null;
  } catch {
    try {
      const { stdout } = await execa("git", ["branch", "--show-current"]);
      return stdout.trim() || null;
    } catch {
      return null;
    }
  }
}

async function getBranchCommitTimestamps(
  refs: string[],
  cwd?: string,
): Promise<Map<string, number>> {
  try {
    const { stdout = "" } = (await execa(
      "git",
      [
        "for-each-ref",
        "--format=%(refname:short)%00%(committerdate:unix)",
        ...refs,
      ],
      cwd ? { cwd } : undefined,
    )) ?? { stdout: "" };

    const map = new Map<string, number>();

    for (const line of stdout.split("\n")) {
      if (!line) continue;
      const [ref, timestamp] = line.split("\0");
      if (!ref || !timestamp) continue;
      if (ref.endsWith("/HEAD")) continue;
      const parsed = Number.parseInt(timestamp, 10);
      if (Number.isNaN(parsed)) continue;
      map.set(ref, parsed);
    }

    return map;
  } catch (error) {
    throw new GitError("Failed to get branch commit timestamps", error);
  }
}

export async function getLocalBranches(): Promise<BranchInfo[]> {
  try {
    const commitMap = await getBranchCommitTimestamps(["refs/heads"]);
    const { stdout = "" } = (await execa("git", [
      "branch",
      "--format=%(refname:short)",
    ])) ?? { stdout: "" };
    return stdout
      .split("\n")
      .filter((line) => line.trim())
      .map((name) => {
        const trimmed = name.trim();
        const timestamp = commitMap.get(trimmed);

        return {
          name: trimmed,
          type: "local" as const,
          branchType: getBranchType(trimmed),
          isCurrent: false,
          ...(timestamp !== undefined
            ? { latestCommitTimestamp: timestamp }
            : {}),
        } satisfies BranchInfo;
      });
  } catch (primaryError) {
    throw new GitError("Failed to get local branches", primaryError);
  }
}

export async function getRemoteBranches(): Promise<BranchInfo[]> {
  try {
    const commitMap = await getBranchCommitTimestamps(["refs/remotes"]);
    const { stdout = "" } = (await execa("git", [
      "branch",
      "-r",
      "--format=%(refname:short)",
    ])) ?? { stdout: "" };
    return stdout
      .split("\n")
      .filter((line) => line.trim() && !line.includes("HEAD"))
      .map((line) => {
        const name = line.trim();
        const branchName = name.replace(/^origin\//, "");
        const timestamp = commitMap.get(name);

        return {
          name,
          type: "remote" as const,
          branchType: getBranchType(branchName),
          isCurrent: false,
          ...(timestamp !== undefined
            ? { latestCommitTimestamp: timestamp }
            : {}),
        } satisfies BranchInfo;
      });
  } catch (primaryError) {
    throw new GitError("Failed to get remote branches", primaryError);
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

/**
 * ローカルブランチのupstream（追跡ブランチ）情報を取得
 * @param cwd - 作業ディレクトリ（省略時はリポジトリルート）
 * @returns Map<ローカルブランチ名, upstreamブランチ名>
 */
export async function collectUpstreamMap(
  cwd?: string,
): Promise<Map<string, string>> {
  const workDir = cwd ?? (await getRepositoryRoot());
  try {
    const { stdout } = await execa(
      "git",
      [
        "for-each-ref",
        "--format=%(refname:short)|%(upstream:short)",
        "refs/heads",
      ],
      { cwd: workDir },
    );

    return stdout
      .split("\n")
      .filter((line) => line.includes("|"))
      .reduce((map, line) => {
        const [branch, upstream] = line.split("|");
        if (branch?.trim() && upstream?.trim()) {
          map.set(branch.trim(), upstream.trim());
        }
        return map;
      }, new Map<string, string>());
  } catch {
    return new Map();
  }
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
  if (branchName.startsWith("bugfix/") || branchName.startsWith("bug/"))
    return "bugfix";
  if (branchName.startsWith("hotfix/")) return "hotfix";
  if (branchName.startsWith("release/")) return "release";
  return "other";
}

async function hasUnpushedCommitsInternal(
  branch: string,
  options: { cwd?: string } = {},
): Promise<boolean> {
  const { cwd } = options;
  const execOptions = cwd ? { cwd } : undefined;
  try {
    const { stdout } = await execa(
      "git",
      ["log", `origin/${branch}..${branch}`, "--oneline"],
      execOptions,
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
        await execa("git", ["rev-parse", "--verify", candidate], execOptions);

        // If we are checking the same branch again, we already know the remote ref is missing.
        if (candidate === `origin/${branch}` || candidate === branch) {
          continue;
        }

        try {
          await execa(
            "git",
            ["merge-base", "--is-ancestor", branch, candidate],
            execOptions,
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

export async function hasUnpushedCommits(
  worktreePath: string,
  branch: string,
): Promise<boolean> {
  return hasUnpushedCommitsInternal(branch, { cwd: worktreePath });
}

export async function hasUnpushedCommitsInRepo(
  branch: string,
  repoRoot?: string,
): Promise<boolean> {
  return hasUnpushedCommitsInternal(branch, repoRoot ? { cwd: repoRoot } : {});
}

export async function branchHasUniqueCommitsComparedToBase(
  branch: string,
  baseBranch: string,
  repoRoot?: string,
): Promise<boolean> {
  const execOptions = repoRoot ? { cwd: repoRoot } : undefined;
  try {
    await execa("git", ["rev-parse", "--verify", branch], execOptions);
  } catch {
    return true;
  }

  const normalizedBase = baseBranch.trim();
  if (!normalizedBase) {
    return true;
  }

  const candidates = new Set<string>();
  candidates.add(normalizedBase);

  if (!normalizedBase.startsWith("origin/")) {
    candidates.add(`origin/${normalizedBase}`);
  } else {
    const localEquivalent = normalizedBase.replace(/^origin\//, "");
    if (localEquivalent) {
      candidates.add(localEquivalent);
    }
  }

  for (const candidate of candidates) {
    try {
      await execa("git", ["rev-parse", "--verify", candidate], execOptions);
    } catch {
      continue;
    }

    try {
      const { stdout } = await execa(
        "git",
        ["log", `${candidate}..${branch}`, "--oneline"],
        execOptions,
      );

      if (stdout.trim().length > 0) {
        return true;
      }

      return false;
    } catch {
      // Comparison failed for this candidate, try next one.
    }
  }

  // If no valid base candidate was found, treat the branch as having unique commits.
  return true;
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
    return parseInt(stdout.trim(), 10) || 0;
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
  } catch {
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

export async function fetchAllRemotes(options?: {
  cwd?: string;
}): Promise<void> {
  try {
    const execOptions = options?.cwd ? { cwd: options.cwd } : undefined;
    const args = ["fetch", "--all", "--prune"];
    if (execOptions) {
      await execa("git", args, execOptions);
    } else {
      await execa("git", args);
    }
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
  } catch {
    // package.jsonが存在しない場合はデフォルトバージョンを返す
    return "0.0.0";
  }
}

export function calculateNewVersion(
  currentVersion: string,
  versionBump: "patch" | "minor" | "major",
): string {
  const versionParts = currentVersion.split(".");
  const major = parseInt(versionParts[0] || "0", 10);
  const minor = parseInt(versionParts[1] || "0", 10);
  const patch = parseInt(versionParts[2] || "0", 10);

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
      { cwd: worktreePath },
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
  options?: { cwd?: string },
): Promise<boolean> {
  try {
    const execOptions = options?.cwd ? { cwd: options.cwd } : undefined;
    const args = [
      "show-ref",
      "--verify",
      "--quiet",
      `refs/remotes/${remote}/${branchName}`,
    ];
    if (execOptions) {
      await execa("git", args, execOptions);
    } else {
      await execa("git", args);
    }
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

/**
 * .gitignoreファイルに指定されたエントリーが存在することを保証します
 * エントリーが既に存在する場合は何もしません
 * @param {string} repoRoot - リポジトリのルートディレクトリ
 * @param {string} entry - 追加するエントリー（例: ".worktrees/"）
 * @throws {GitError} ファイルの読み書きに失敗した場合
 */
// ========================================
// Batch Merge Operations (SPEC-ee33ca26)
// ========================================

/**
 * Merge from source branch to current branch in worktree
 * @param worktreePath - Path to worktree directory
 * @param sourceBranch - Source branch to merge from
 * @param dryRun - If true, use --no-commit flag for dry-run mode
 * @see specs/SPEC-ee33ca26/research.md - Decision 3: Dry-run implementation
 */
export async function mergeFromBranch(
  worktreePath: string,
  sourceBranch: string,
  dryRun = false,
): Promise<void> {
  try {
    const args = ["merge"];
    if (dryRun) {
      args.push("--no-commit");
    }
    args.push(sourceBranch);

    await execa("git", args, { cwd: worktreePath });
  } catch (error) {
    throw new GitError(
      `Failed to merge from ${sourceBranch} in ${worktreePath}`,
      error,
    );
  }
}

/**
 * Check if there is a merge in progress in worktree
 * @param worktreePath - Path to worktree directory
 * @returns true if MERGE_HEAD exists (merge in progress)
 * @see specs/SPEC-ee33ca26/research.md - Best practices: Git state confirmation
 */
export async function hasMergeConflict(worktreePath: string): Promise<boolean> {
  try {
    await execa("git", ["rev-parse", "--git-path", "MERGE_HEAD"], {
      cwd: worktreePath,
    });
    return true;
  } catch {
    return false;
  }
}

/**
 * Abort current merge operation in worktree
 * @param worktreePath - Path to worktree directory
 * @see specs/SPEC-ee33ca26/research.md - Decision 3: Dry-run rollback
 */
export async function abortMerge(worktreePath: string): Promise<void> {
  try {
    await execa("git", ["merge", "--abort"], { cwd: worktreePath });
  } catch (error) {
    throw new GitError(`Failed to abort merge in ${worktreePath}`, error);
  }
}

/**
 * Get current merge status in worktree
 * @param worktreePath - Path to worktree directory
 * @returns Object with inProgress and hasConflict flags
 * @see specs/SPEC-ee33ca26/research.md - Best practices: Git state confirmation
 */
export async function getMergeStatus(worktreePath: string): Promise<{
  inProgress: boolean;
  hasConflict: boolean;
}> {
  // Check if merge is in progress (MERGE_HEAD exists)
  const inProgress = await hasMergeConflict(worktreePath);

  // Check if there are conflicts (git status --porcelain shows UU)
  let hasConflict = false;
  if (inProgress) {
    try {
      const { stdout } = await execa("git", ["status", "--porcelain"], {
        cwd: worktreePath,
      });
      // UU indicates unmerged paths (conflicts)
      hasConflict = stdout.includes("UU ");
    } catch {
      hasConflict = false;
    }
  }

  return {
    inProgress,
    hasConflict,
  };
}

/**
 * Reset worktree to HEAD (rollback all changes)
 * Used for dry-run cleanup after git merge --no-commit
 * @param worktreePath - Path to worktree directory
 * @see specs/SPEC-ee33ca26/research.md - Dry-run implementation: --no-commit + rollback
 */
export async function resetToHead(worktreePath: string): Promise<void> {
  try {
    await execa("git", ["reset", "--hard", "HEAD"], {
      cwd: worktreePath,
    });
  } catch (error) {
    throw new GitError(
      `Failed to reset worktree to HEAD in ${worktreePath}`,
      error,
    );
  }
}

export interface BranchDivergenceStatus {
  branch: string;
  remoteAhead: number;
  localAhead: number;
}

export async function getBranchDivergenceStatuses(options?: {
  cwd?: string;
  remote?: string;
  branches?: string[];
}): Promise<BranchDivergenceStatus[]> {
  const cwd = options?.cwd;
  const remote = options?.remote ?? "origin";
  const execOptions = cwd ? { cwd } : undefined;
  const branchFilter = options?.branches?.filter(
    (name) => name.trim().length > 0,
  );
  const filterSet =
    branchFilter && branchFilter.length > 0 ? new Set(branchFilter) : null;

  const branchArgs = ["branch", "--format=%(refname:short)"];
  const { stdout: localBranchOutput } = execOptions
    ? await execa("git", branchArgs, execOptions)
    : await execa("git", branchArgs);

  const branchNames = localBranchOutput
    .split("\n")
    .map((name) => name.trim())
    .filter(Boolean)
    .filter((name) => !filterSet || filterSet.has(name));

  if (filterSet && branchNames.length === 0) {
    return [];
  }

  const results: BranchDivergenceStatus[] = [];

  for (const branchName of branchNames) {
    const remoteExists = await checkRemoteBranchExists(
      branchName,
      remote,
      cwd ? { cwd } : undefined,
    );

    if (!remoteExists) {
      continue;
    }

    try {
      const revListArgs = [
        "rev-list",
        "--left-right",
        "--count",
        `${remote}/${branchName}...${branchName}`,
      ];
      const { stdout } = execOptions
        ? await execa("git", revListArgs, execOptions)
        : await execa("git", revListArgs);

      const [remoteAheadRaw, localAheadRaw] = stdout.trim().split(/\s+/);
      const remoteAhead = Number.parseInt(remoteAheadRaw || "0", 10) || 0;
      const localAhead = Number.parseInt(localAheadRaw || "0", 10) || 0;

      results.push({ branch: branchName, remoteAhead, localAhead });
    } catch (error) {
      throw new GitError(
        `Failed to inspect divergence for ${branchName}`,
        error,
      );
    }
  }

  return results;
}

export async function pullFastForward(
  worktreePath: string,
  remote = "origin",
): Promise<void> {
  try {
    await execa("git", ["pull", "--ff-only", remote], {
      cwd: worktreePath,
    });
  } catch (error) {
    throw new GitError(`Failed to fast-forward pull in ${worktreePath}`, error);
  }
}

export async function ensureGitignoreEntry(
  repoRoot: string,
  entry: string,
): Promise<void> {
  const fs = await import("node:fs/promises");
  const gitignorePath = path.join(repoRoot, ".gitignore");

  try {
    // .gitignoreファイルを読み込む（存在しない場合は空文字列）
    let content = "";
    let eol = "\n";
    try {
      content = await fs.readFile(gitignorePath, "utf-8");
      if (content.includes("\r\n")) {
        eol = "\r\n";
      }
    } catch (error: any) {
      // ENOENTエラー（ファイルが存在しない）は無視
      if (error.code !== "ENOENT") {
        throw error;
      }
    }

    const normalizedEntry = entry.trim();
    const normalizedLines = content.split(/\r?\n/).map((line) => line.trim());

    if (normalizedLines.includes(normalizedEntry)) {
      // 既に存在する場合は何もしない
      return;
    }

    const needsSeparator =
      content.length > 0 && !content.endsWith("\n") && !content.endsWith("\r");
    const separator = needsSeparator ? eol : "";

    const newContent = `${content}${separator}${entry}${eol}`;
    await fs.writeFile(gitignorePath, newContent, "utf-8");
  } catch (error: any) {
    throw new GitError(`Failed to update .gitignore: ${error.message}`, error);
  }
}
