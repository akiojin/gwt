import { execa } from "execa";
import path from "node:path";
import chalk from "chalk";
import {
  WorktreeConfig,
  WorktreeWithPR,
  CleanupTarget,
  MergedPullRequest,
  CleanupReason,
} from "./ui/types.js";
import { getPullRequestByBranch, getMergedPullRequests } from "./github.js";
import {
  hasUncommittedChanges,
  hasUnpushedCommits,
  hasUnpushedCommitsInRepo,
  getLocalBranches,
  checkRemoteBranchExists,
  branchHasUniqueCommitsComparedToBase,
  getRepositoryRoot,
  ensureGitignoreEntry,
} from "./git.js";
import { getConfig } from "./config/index.js";
import { GIT_CONFIG } from "./config/constants.js";

// Re-export WorktreeConfig for external use
export type { WorktreeConfig };

// 保護対象のブランチ（クリーンアップから除外）
const PROTECTED_BRANCHES = ["main", "master", "develop"];
export class WorktreeError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "WorktreeError";
  }
}

export interface WorktreeInfo {
  path: string;
  branch: string;
  head: string;
  isAccessible?: boolean;
  invalidReason?: string;
}

async function listWorktrees(): Promise<WorktreeInfo[]> {
  try {
    const { stdout } = await execa("git", ["worktree", "list", "--porcelain"]);
    const worktrees: WorktreeInfo[] = [];
    const lines = stdout.split("\n");

    let currentWorktree: Partial<WorktreeInfo> = {};

    for (const line of lines) {
      if (line.startsWith("worktree ")) {
        if (currentWorktree.path) {
          worktrees.push(currentWorktree as WorktreeInfo);
        }
        currentWorktree = { path: line.substring(9) };
      } else if (line.startsWith("HEAD ")) {
        currentWorktree.head = line.substring(5);
      } else if (line.startsWith("branch ")) {
        currentWorktree.branch = line.substring(7).replace("refs/heads/", "");
      } else if (line === "") {
        if (currentWorktree.path) {
          worktrees.push(currentWorktree as WorktreeInfo);
          currentWorktree = {};
        }
      }
    }

    if (currentWorktree.path) {
      worktrees.push(currentWorktree as WorktreeInfo);
    }

    return worktrees;
  } catch (error) {
    throw new WorktreeError("Failed to list worktrees", error);
  }
}

/**
 * 追加のworktree（メインworktreeを除く）の一覧を取得
 * @returns {Promise<WorktreeInfo[]>} worktree情報の配列
 * @throws {WorktreeError} worktree一覧の取得に失敗した場合
 */
export async function listAdditionalWorktrees(): Promise<WorktreeInfo[]> {
  try {
    const [allWorktrees, repoRoot] = await Promise.all([
      listWorktrees(),
      import("./git.js").then((m) => m.getRepositoryRoot()),
    ]);

    const fs = await import("node:fs");

    // Filter out the main worktree (repository root) and add accessibility info
    const additionalWorktrees = allWorktrees
      .filter((worktree) => worktree.path !== repoRoot)
      .map((worktree) => {
        // パスの存在を確認
        const isAccessible = fs.existsSync(worktree.path);

        const result: WorktreeInfo = {
          ...worktree,
          isAccessible,
        };

        if (!isAccessible) {
          result.invalidReason = "Path not accessible in current environment";
        }

        return result;
      });

    return additionalWorktrees;
  } catch (error) {
    throw new WorktreeError("Failed to list additional worktrees", error);
  }
}

export async function worktreeExists(
  branchName: string,
): Promise<string | null> {
  const worktrees = await listWorktrees();
  const worktree = worktrees.find((w) => w.branch === branchName);
  return worktree ? worktree.path : null;
}

export async function generateWorktreePath(
  repoRoot: string,
  branchName: string,
): Promise<string> {
  const sanitizedBranchName = branchName.replace(/[\/\\:*?"<>|]/g, "-");
  const worktreeDir = path.join(repoRoot, ".worktrees");
  return path.join(worktreeDir, sanitizedBranchName);
}

/**
 * 指定されたパスに既存のworktreeが存在するか確認
 * @param {string} targetPath - 確認するパス
 * @returns {Promise<WorktreeInfo | null>} 既存のworktree情報、または存在しない場合はnull
 */
export async function checkWorktreePathConflict(
  targetPath: string,
): Promise<WorktreeInfo | null> {
  const worktrees = await listWorktrees();
  const existingWorktree = worktrees.find((w) => w.path === targetPath);
  return existingWorktree || null;
}

/**
 * 衝突を避けるため、代替のworktreeパスを生成
 * @param {string} basePath - 元のパス
 * @returns {Promise<string>} 利用可能な代替パス
 */
export async function generateAlternativeWorktreePath(
  basePath: string,
): Promise<string> {
  let counter = 2;
  let alternativePath = `${basePath}-${counter}`;

  // 衝突しないパスが見つかるまで試行
  while (await checkWorktreePathConflict(alternativePath)) {
    counter++;
    alternativePath = `${basePath}-${counter}`;
  }

  return alternativePath;
}

/**
 * 新しいworktreeを作成
 * @param {WorktreeConfig} config - worktreeの設定
 * @throws {WorktreeError} worktreeの作成に失敗した場合
 */
export async function createWorktree(config: WorktreeConfig): Promise<void> {
  try {
    const args = ["worktree", "add"];

    if (config.isNewBranch) {
      args.push("-b", config.branchName);
    }

    args.push(config.worktreePath);

    if (config.isNewBranch) {
      args.push(config.baseBranch);
    } else {
      args.push(config.branchName);
    }

    const gitProcess = execa("git", args);

    // パイプでリアルタイムに進捗を表示する
    if (gitProcess.stdout && typeof gitProcess.stdout.pipe === "function") {
      gitProcess.stdout.pipe(process.stdout);
    }
    if (gitProcess.stderr && typeof gitProcess.stderr.pipe === "function") {
      gitProcess.stderr.pipe(process.stderr);
    }

    await gitProcess;

    // .gitignoreに.worktrees/を追加(エラーは警告として扱う)
    try {
      await ensureGitignoreEntry(config.repoRoot, ".worktrees/");
    } catch (error: any) {
      // .gitignoreの更新失敗は警告としてログに出すが、worktree作成は成功とする
      console.warn(
        `Warning: Failed to update .gitignore: ${error.message || error}`,
      );
    }
  } catch (error: any) {
    // Extract more detailed error information from git command
    const gitError =
      error?.stderr || error?.stdout || error?.message || String(error);
    const errorMessage = `Failed to create worktree for ${config.branchName}\nGit error: ${gitError}`;
    throw new WorktreeError(errorMessage, error);
  }
}

export async function removeWorktree(
  worktreePath: string,
  force = false,
): Promise<void> {
  try {
    const args = ["worktree", "remove"];
    if (force) {
      args.push("--force");
    }
    args.push(worktreePath);

    await execa("git", args);
  } catch (error) {
    throw new WorktreeError(
      `Failed to remove worktree at ${worktreePath}`,
      error,
    );
  }
}

async function getWorktreesWithPRStatus(): Promise<WorktreeWithPR[]> {
  const worktrees = await listAdditionalWorktrees();
  const worktreesWithPR: WorktreeWithPR[] = [];

  for (const worktree of worktrees) {
    if (worktree.branch) {
      const pullRequest = await getPullRequestByBranch(worktree.branch);
      worktreesWithPR.push({
        worktreePath: worktree.path,
        branch: worktree.branch,
        pullRequest,
      });
    }
  }

  return worktreesWithPR;
}

/**
 * worktreeに存在しないローカルブランチの中でマージ済みPRに関連するクリーンアップ候補を取得
 * @returns {Promise<CleanupTarget[]>} クリーンアップ候補の配列
 */
async function getOrphanedLocalBranches({
  mergedPRs,
  baseBranch,
  repoRoot,
}: {
  mergedPRs: MergedPullRequest[];
  baseBranch: string;
  repoRoot: string;
}): Promise<CleanupTarget[]> {
  try {
    // 並列実行で高速化
    const [localBranches, worktrees] = await Promise.all([
      getLocalBranches(),
      listAdditionalWorktrees(),
    ]);

    const cleanupTargets: CleanupTarget[] = [];
    const worktreeBranches = new Set(
      worktrees.map((w) => w.branch).filter(Boolean),
    );

    if (process.env.DEBUG_CLEANUP) {
      console.log(
        chalk.cyan("Debug: Orphaned branch scan - Available local branches:"),
      );
      localBranches.forEach((b) =>
        console.log(`  ${b.name} (type: ${b.type})`),
      );
      console.log(
        chalk.cyan(
          `Debug: Worktree branches: ${Array.from(worktreeBranches).join(", ")}`,
        ),
      );
    }

    for (const localBranch of localBranches) {
      // 保護対象ブランチはスキップ
      if (PROTECTED_BRANCHES.includes(localBranch.name)) {
        if (process.env.DEBUG_CLEANUP) {
          console.log(
            chalk.yellow(
              `Debug: Skipping protected branch ${localBranch.name}`,
            ),
          );
        }
        continue;
      }

      // worktreeに存在しないローカルブランチのみ対象
      if (!worktreeBranches.has(localBranch.name)) {
        const mergedPR = findMatchingPR(localBranch.name, mergedPRs);
        let hasUnpushed = false;
        try {
          hasUnpushed = await hasUnpushedCommitsInRepo(
            localBranch.name,
            repoRoot,
          );
        } catch {
          hasUnpushed = true;
        }

        const reasons: CleanupReason[] = [];

        if (mergedPR) {
          reasons.push("merged-pr");
        }

        if (!hasUnpushed) {
          const hasUniqueCommits = await branchHasUniqueCommitsComparedToBase(
            localBranch.name,
            baseBranch,
            repoRoot,
          );

          if (!hasUniqueCommits) {
            reasons.push("no-diff-with-base");
          }
        }

        if (process.env.DEBUG_CLEANUP) {
          console.log(
            chalk.gray(
              `Debug: Checking orphaned branch ${localBranch.name} -> PR: ${mergedPR ? "MATCH" : "NO MATCH"}, reasons: ${reasons.join(", ")}`,
            ),
          );
        }

        if (reasons.length > 0) {
          let hasRemoteBranch = false;
          try {
            hasRemoteBranch = await checkRemoteBranchExists(localBranch.name);
          } catch {
            hasRemoteBranch = false;
          }

          cleanupTargets.push({
            worktreePath: null, // worktreeは存在しない
            branch: localBranch.name,
            pullRequest: mergedPR ?? null,
            hasUncommittedChanges: false, // worktreeが存在しないため常にfalse
            hasUnpushedCommits: hasUnpushed,
            cleanupType: "branch-only",
            hasRemoteBranch,
            reasons,
          });
        }
      }
    }

    if (process.env.DEBUG_CLEANUP) {
      console.log(
        chalk.cyan(
          `Debug: Found ${cleanupTargets.length} orphaned branch cleanup targets`,
        ),
      );
    }

    return cleanupTargets;
  } catch (error) {
    console.error(chalk.red("Error: Failed to get orphaned local branches"));
    if (process.env.DEBUG_CLEANUP) {
      console.error(chalk.red("Debug: Full error details:"), error);
    }
    return [];
  }
}

function normalizeBranchName(branchName: string): string {
  return branchName
    .replace(/^origin\//, "")
    .replace(/^refs\/heads\//, "")
    .replace(/^refs\/remotes\/origin\//, "")
    .trim();
}

function findMatchingPR(
  worktreeBranch: string,
  mergedPRs: MergedPullRequest[],
): MergedPullRequest | null {
  const normalizedWorktreeBranch = normalizeBranchName(worktreeBranch);

  for (const pr of mergedPRs) {
    const normalizedPRBranch = normalizeBranchName(pr.branch);

    if (normalizedWorktreeBranch === normalizedPRBranch) {
      return pr;
    }
  }

  return null;
}

/**
 * マージ済みPRに関連するworktreeおよびローカルブランチのクリーンアップ候補を取得
 * @returns {Promise<CleanupTarget[]>} クリーンアップ候補の配列
 */
export async function getMergedPRWorktrees(): Promise<CleanupTarget[]> {
  const [config, repoRoot] = await Promise.all([
    getConfig(),
    getRepositoryRoot(),
  ]);
  const baseBranch = config.defaultBaseBranch || GIT_CONFIG.DEFAULT_BASE_BRANCH;

  // 並列実行で高速化 - worktreeとマージ済みPRの両方を取得
  const [mergedPRs, worktreesWithPR] = await Promise.all([
    getMergedPullRequests(),
    getWorktreesWithPRStatus(),
  ]);
  const orphanedBranches = await getOrphanedLocalBranches({
    mergedPRs,
    baseBranch,
    repoRoot,
  });
  const cleanupTargets: CleanupTarget[] = [];

  if (process.env.DEBUG_CLEANUP) {
    console.log(chalk.cyan("Debug: Available worktrees:"));
    worktreesWithPR.forEach((w) =>
      console.log(`  ${w.branch} -> ${w.worktreePath}`),
    );
    console.log(chalk.cyan("Debug: Merged PRs:"));
    mergedPRs.forEach((pr) => console.log(`  ${pr.branch} (PR #${pr.number})`));
  }

  for (const worktree of worktreesWithPR) {
    // 保護対象ブランチはスキップ
    if (PROTECTED_BRANCHES.includes(worktree.branch)) {
      if (process.env.DEBUG_CLEANUP) {
        console.log(
          chalk.yellow(`Debug: Skipping protected branch ${worktree.branch}`),
        );
      }
      continue;
    }

    const mergedPR = findMatchingPR(worktree.branch, mergedPRs);

    if (process.env.DEBUG_CLEANUP) {
      const normalizedWorktree = normalizeBranchName(worktree.branch);
      console.log(
        chalk.gray(
          `Debug: Checking worktree ${worktree.branch} (normalized: ${normalizedWorktree}) -> ${mergedPR ? "MATCH" : "NO MATCH"}`,
        ),
      );
    }

    const cleanupReasons: CleanupReason[] = [];

    if (mergedPR) {
      cleanupReasons.push("merged-pr");
    }

    // worktreeパスの存在を確認
    const fs = await import("node:fs");
    const isAccessible = fs.existsSync(worktree.worktreePath);

    let hasUncommitted = false;
    let hasUnpushed = false;

    if (isAccessible) {
      // worktreeが存在する場合のみ状態をチェック
      try {
        [hasUncommitted, hasUnpushed] = await Promise.all([
          hasUncommittedChanges(worktree.worktreePath),
          hasUnpushedCommits(worktree.worktreePath, worktree.branch),
        ]);
      } catch (error) {
        // エラーが発生した場合はデフォルト値を使用
        if (process.env.DEBUG_CLEANUP) {
          console.log(
            chalk.yellow(
              `Debug: Failed to check status for worktree ${worktree.worktreePath}: ${error instanceof Error ? error.message : String(error)}`,
            ),
          );
        }
      }
    }

    if (!hasUnpushed) {
      const hasUniqueCommits = await branchHasUniqueCommitsComparedToBase(
        worktree.branch,
        baseBranch,
        repoRoot,
      );

      if (!hasUniqueCommits) {
        cleanupReasons.push("no-diff-with-base");
      }
    }

    if (process.env.DEBUG_CLEANUP) {
      console.log(
        chalk.gray(
          `Debug: Cleanup reasons for ${worktree.branch}: ${cleanupReasons.length > 0 ? cleanupReasons.join(", ") : "none"}`,
        ),
      );
    }

    if (cleanupReasons.length === 0) {
      continue;
    }

    const hasRemoteBranch = await checkRemoteBranchExists(worktree.branch);

    const target: CleanupTarget = {
      worktreePath: worktree.worktreePath,
      branch: worktree.branch,
      pullRequest: mergedPR ?? null,
      hasUncommittedChanges: hasUncommitted,
      hasUnpushedCommits: hasUnpushed,
      cleanupType: "worktree-and-branch",
      hasRemoteBranch,
      isAccessible,
      reasons: cleanupReasons,
    };

    if (!isAccessible) {
      target.invalidReason = "Path not accessible in current environment";
    }

    cleanupTargets.push(target);
  }

  // orphanedBranches (ローカルブランチのみの削除対象) を追加
  cleanupTargets.push(...orphanedBranches);

  if (process.env.DEBUG_CLEANUP) {
    const worktreeTargets = cleanupTargets.filter(
      (t) => t.cleanupType === "worktree-and-branch",
    ).length;
    const branchOnlyTargets = cleanupTargets.filter(
      (t) => t.cleanupType === "branch-only",
    ).length;
    console.log(
      chalk.cyan(
        `Debug: Found ${cleanupTargets.length} cleanup targets (${worktreeTargets} worktree+branch, ${branchOnlyTargets} branch-only)`,
      ),
    );
  }

  return cleanupTargets;
}
