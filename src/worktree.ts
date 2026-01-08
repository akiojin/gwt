import { execa } from "execa";
import fs from "node:fs/promises";
import path from "node:path";
import chalk from "chalk";
import { createLogger } from "./logging/logger.js";

const logger = createLogger({ category: "worktree" });
import {
  WorktreeConfig,
  WorktreeWithPR,
  CleanupTarget,
  CleanupReason,
  CleanupStatus,
} from "./cli/ui/types.js";
import { getPullRequestByBranch } from "./github.js";
import {
  hasUncommittedChanges,
  hasUnpushedCommits,
  hasUnpushedCommitsInRepo,
  getLocalBranches,
  checkRemoteBranchExists,
  branchHasUniqueCommitsComparedToBase,
  getRepositoryRoot,
  getWorktreeRoot,
  ensureGitignoreEntry,
  branchExists,
  getCurrentBranch,
  getCurrentBranchName,
} from "./git.js";
import { getConfig } from "./config/index.js";
import { GIT_CONFIG } from "./config/constants.js";
import { startSpinner } from "./utils/spinner.js";

async function getUpstreamBranch(branch: string): Promise<string | null> {
  try {
    const result = await execa("git", [
      "rev-parse",
      "--abbrev-ref",
      `${branch}@{upstream}`,
    ]);
    const stdout =
      typeof (result as { stdout?: unknown })?.stdout === "string"
        ? (result as { stdout: string }).stdout.trim()
        : "";
    return stdout.length ? stdout : null;
  } catch {
    return null;
  }
}

const parseRemoteRef = (
  ref: string,
): { remote: string; branch: string } | null => {
  const segments = ref.split("/");
  if (segments.length < 2) {
    return null;
  }
  const [remote, ...rest] = segments;
  const branch = rest.join("/");
  if (!remote || !branch) {
    return null;
  }
  return { remote, branch };
};

async function resolveUpstreamStatus(
  branch: string,
  repoRoot: string,
): Promise<{ upstream: string | null; hasUpstream: boolean }> {
  const upstream = await getUpstreamBranch(branch);
  if (!upstream) {
    return { upstream: null, hasUpstream: false };
  }
  const parsed = parseRemoteRef(upstream);
  if (!parsed) {
    return { upstream, hasUpstream: false };
  }
  if (parsed.branch !== branch) {
    return { upstream, hasUpstream: false };
  }
  const exists = await checkRemoteBranchExists(parsed.branch, parsed.remote, {
    cwd: repoRoot,
  });
  return { upstream, hasUpstream: exists };
}

const buildCleanupReasons = ({
  hasUniqueCommits,
  hasUncommitted,
  hasUnpushed,
  hasUpstream,
}: {
  hasUniqueCommits: boolean;
  hasUncommitted: boolean;
  hasUnpushed: boolean;
  hasUpstream: boolean;
}): CleanupReason[] => {
  if (!hasUpstream) {
    return [];
  }
  if (!hasUniqueCommits && !hasUncommitted && !hasUnpushed) {
    return ["no-diff-with-base"];
  }
  if (hasUniqueCommits && !hasUncommitted && !hasUnpushed) {
    return ["remote-synced"];
  }
  return [];
};

// Re-export WorktreeConfig for external use
export type { WorktreeConfig };

// 保護対象のブランチ（クリーンアップから除外）
export const PROTECTED_BRANCHES = ["main", "master", "develop"];

export function isProtectedBranchName(branchName: string): boolean {
  const normalized = branchName
    .replace(/^refs\/heads\//, "")
    .replace(/^origin\//, "");
  return PROTECTED_BRANCHES.includes(normalized);
}

export async function switchToProtectedBranch({
  branchName,
  repoRoot,
  remoteRef,
}: {
  branchName: string;
  repoRoot: string;
  remoteRef?: string | null;
}): Promise<"none" | "local" | "remote"> {
  const currentBranch = await getCurrentBranch();
  if (currentBranch === branchName) {
    return "none";
  }

  const runGit = async (args: string[]) => {
    try {
      await execa("git", args, { cwd: repoRoot });
    } catch (error) {
      throw new WorktreeError(
        `Failed to execute git ${args.join(" ")} for protected branch ${branchName}`,
        error,
      );
    }
  };

  if (await branchExists(branchName)) {
    await runGit(["checkout", branchName]);
    return "local";
  }

  const targetRemote = remoteRef ?? `origin/${branchName}`;
  const fetchRef = targetRemote.replace(/^origin\//, "");

  await runGit(["fetch", "origin", fetchRef]);
  await runGit(["checkout", "-b", branchName, targetRemote]);

  return "remote";
}
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
  locked?: boolean;
  prunable?: boolean;
  isAccessible?: boolean;
  invalidReason?: string;
  hasUncommittedChanges?: boolean;
  isLocked?: boolean;
  isPrunable?: boolean;
}

async function listWorktrees(): Promise<WorktreeInfo[]> {
  try {
    const { getRepositoryRoot } = await import("./git.js");
    const repoRoot = await getRepositoryRoot();
    const { stdout } = await execa("git", ["worktree", "list", "--porcelain"], {
      cwd: repoRoot,
    });
    const worktrees: WorktreeInfo[] = [];
    const lines = stdout.split("\n");

    let currentWorktree: Partial<WorktreeInfo> = {};

    for (const line of lines) {
      if (line.startsWith("worktree ")) {
        if (currentWorktree.path) {
          worktrees.push(currentWorktree as WorktreeInfo);
        }
        currentWorktree = {
          path: line.substring(9),
          locked: false,
          prunable: false,
          isLocked: false,
          isPrunable: false,
        };
      } else if (line.startsWith("HEAD ")) {
        currentWorktree.head = line.substring(5);
      } else if (line.startsWith("branch ")) {
        currentWorktree.branch = line.substring(7).replace("refs/heads/", "");
      } else if (line === "locked" || line.startsWith("locked ")) {
        currentWorktree.locked = true;
        currentWorktree.isLocked = true;
        if (line.startsWith("locked ")) {
          const reason = line.substring(7).trim();
          if (reason) {
            currentWorktree.invalidReason ??= reason;
          }
        }
      } else if (line === "prunable" || line.startsWith("prunable ")) {
        currentWorktree.prunable = true;
        currentWorktree.isPrunable = true;
        if (line.startsWith("prunable ")) {
          const reason = line.substring(9).trim();
          if (reason) {
            currentWorktree.invalidReason ??= reason;
          }
        }
      } else if (line.startsWith("reason ")) {
        currentWorktree.invalidReason = line.substring(7);
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
        const exists = fs.existsSync(worktree.path);
        const isAccessible = exists && !worktree.prunable;

        const result: WorktreeInfo = {
          ...worktree,
          isAccessible,
        };

        if (!isAccessible) {
          if (!exists) {
            result.invalidReason = "Path not accessible in current environment";
          } else if (worktree.prunable && !result.invalidReason) {
            result.invalidReason = "Worktree is marked prunable";
          }
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
  const resolution = await resolveWorktreePathForBranch(branchName);
  return resolution.path;
}

/**
 * Resolution result for a branch-associated worktree path.
 */
export interface WorktreePathResolution {
  path: string | null;
  mismatch?: {
    path: string;
    actualBranch: string | null;
  };
}

/**
 * Resolve a worktree path for the selected branch and verify the actual checkout.
 */
export async function resolveWorktreePathForBranch(
  branchName: string,
): Promise<WorktreePathResolution> {
  const worktrees = await listWorktrees();
  const worktree = worktrees.find((w) => w.branch === branchName);
  if (!worktree) {
    return { path: null };
  }

  try {
    const actualBranch = (await getCurrentBranchName(worktree.path)).trim();
    if (!actualBranch || actualBranch !== branchName) {
      return {
        path: null,
        mismatch: {
          path: worktree.path,
          actualBranch: actualBranch || null,
        },
      };
    }
  } catch {
    return {
      path: null,
      mismatch: {
        path: worktree.path,
        actualBranch: null,
      },
    };
  }

  return { path: worktree.path };
}

export async function generateWorktreePath(
  repoRoot: string,
  branchName: string,
): Promise<string> {
  const sanitizedBranchName = branchName.replace(/[/\\:*?"<>|]/g, "-");
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

type StaleWorktreeAssessment = {
  status: "absent" | "registered" | "stale" | "unknown";
  reason?: string;
};

async function assessStaleWorktreeDirectory(
  targetPath: string,
): Promise<StaleWorktreeAssessment> {
  const fsSync = await import("node:fs");

  if (!fsSync.existsSync(targetPath)) {
    return { status: "absent" };
  }

  const registered = await checkWorktreePathConflict(targetPath);
  if (registered) {
    return { status: "registered" };
  }

  const gitMetaPath = path.join(targetPath, ".git");
  if (!fsSync.existsSync(gitMetaPath)) {
    return { status: "stale", reason: "missing .git" };
  }

  let gitMetaStat: Awaited<ReturnType<typeof fs.lstat>>;
  try {
    gitMetaStat = await fs.lstat(gitMetaPath);
  } catch {
    return { status: "unknown", reason: "unable to stat .git" };
  }

  if (gitMetaStat.isDirectory()) {
    return { status: "unknown", reason: ".git is a directory" };
  }

  if (!gitMetaStat.isFile()) {
    return { status: "unknown", reason: ".git is not a file" };
  }

  let gitMetaContents = "";
  try {
    gitMetaContents = await fs.readFile(gitMetaPath, "utf8");
  } catch {
    return { status: "unknown", reason: "unable to read .git" };
  }

  const gitdirMatch = gitMetaContents.match(/^\s*gitdir:\s*(.+)\s*$/m);
  if (!gitdirMatch) {
    return { status: "unknown", reason: "missing gitdir entry" };
  }

  const rawGitdir = gitdirMatch[1]?.trim();
  if (!rawGitdir) {
    return { status: "unknown", reason: "empty gitdir entry" };
  }

  const gitdirPath = path.isAbsolute(rawGitdir)
    ? rawGitdir
    : path.resolve(targetPath, rawGitdir);

  if (!fsSync.existsSync(gitdirPath)) {
    return { status: "stale", reason: "missing gitdir path" };
  }

  return { status: "unknown", reason: "gitdir exists" };
}

/**
 * 新しいworktreeを作成
 * @param {WorktreeConfig} config - worktreeの設定
 * @throws {WorktreeError} worktreeの作成に失敗した場合
 */
export async function createWorktree(config: WorktreeConfig): Promise<void> {
  if (isProtectedBranchName(config.branchName)) {
    throw new WorktreeError(
      `Branch "${config.branchName}" is protected and cannot be used to create a worktree`,
    );
  }

  try {
    const staleness = await assessStaleWorktreeDirectory(config.worktreePath);
    if (staleness.status === "stale") {
      await fs.rm(config.worktreePath, { recursive: true, force: true });
    } else if (staleness.status === "unknown") {
      const reason = staleness.reason ? ` (${staleness.reason})` : "";
      throw new WorktreeError(
        `Worktree path already exists but is not registered as a git worktree, and stale status could not be confirmed${reason}. Remove the directory manually and retry: ${config.worktreePath}`,
      );
    }

    const worktreeParentDir = path.dirname(config.worktreePath);

    try {
      await fs.mkdir(worktreeParentDir, { recursive: true });
    } catch (error: unknown) {
      const errorWithCode = error as { code?: unknown; message?: unknown };
      const code =
        errorWithCode && typeof errorWithCode.code === "string"
          ? errorWithCode.code
          : undefined;
      const message =
        error instanceof Error
          ? error.message
          : typeof errorWithCode?.message === "string"
            ? errorWithCode.message
            : String(error);
      const reason =
        code === "EEXIST"
          ? `${worktreeParentDir} already exists and is not a directory`
          : message;

      throw new WorktreeError(
        `Failed to prepare worktree directory for ${config.branchName}: ${reason}`,
        error,
      );
    }

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

    const spinnerMessage = `Creating worktree for ${config.branchName}`;
    let stopSpinner: (() => void) | undefined;

    try {
      stopSpinner = startSpinner(spinnerMessage);
    } catch {
      stopSpinner = undefined;
    }

    const stopActiveSpinner = () => {
      if (stopSpinner) {
        stopSpinner();
        stopSpinner = undefined;
      }
    };

    const gitProcess = execa("git", args);

    // パイプでリアルタイムに進捗を表示する
    const childProcess = gitProcess as typeof gitProcess & {
      stdout?: NodeJS.ReadableStream;
      stderr?: NodeJS.ReadableStream;
    };

    const attachStream = (
      stream: NodeJS.ReadableStream | undefined,
      pipeTarget: NodeJS.WriteStream,
    ) => {
      if (!stream) return;
      if (typeof stream.once === "function") {
        stream.once("data", stopActiveSpinner);
      }
      if (typeof stream.pipe === "function") {
        stream.pipe(pipeTarget);
      }
    };

    attachStream(childProcess.stdout, process.stdout);
    attachStream(childProcess.stderr, process.stderr);

    try {
      await gitProcess;
    } finally {
      stopActiveSpinner();
    }

    // 新規ブランチの場合、ベースブランチを追跡ブランチとして設定
    if (config.isNewBranch && config.baseBranch) {
      try {
        await execa(
          "git",
          ["branch", "--set-upstream-to", config.baseBranch, config.branchName],
          { cwd: config.worktreePath },
        );
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        console.warn(`Warning: Failed to set upstream branch: ${message}`);
      }
    }

    // .gitignoreに.worktrees/を追加(エラーは警告として扱う)
    try {
      let gitignoreRoot = config.repoRoot;
      try {
        gitignoreRoot = await getWorktreeRoot();
      } catch (resolveError) {
        if (process.env.DEBUG) {
          const reason =
            resolveError instanceof Error
              ? resolveError.message
              : String(resolveError);
          console.warn(
            `Debug: Failed to resolve current worktree root for .gitignore update. Falling back to ${gitignoreRoot}. Reason: ${reason}`,
          );
        }
      }

      await ensureGitignoreEntry(gitignoreRoot, ".worktrees/");
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      // .gitignoreの更新失敗は警告としてログに出すが、worktree作成は成功とする
      console.warn(`Warning: Failed to update .gitignore: ${message}`);
    }
  } catch (error: unknown) {
    // Extract more detailed error information from git command
    const errorOutput = (value: unknown) =>
      typeof value === "string" && value.trim().length > 0 ? value : null;
    const gitError =
      errorOutput((error as { stderr?: unknown })?.stderr) ??
      errorOutput((error as { stdout?: unknown })?.stdout) ??
      (error instanceof Error ? error.message : String(error));
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

export interface RepairResult {
  repairedCount: number;
  failedCount: number;
  failures: Array<{ path: string; error: string }>;
}

/**
 * Worktreeパス修復の結果
 */
export interface RepairPathResult {
  /** 修復成功 (git worktree repair) */
  repaired: boolean;
  /** 古いメタデータ削除成功 (git worktree remove --force) */
  removed: boolean;
  /** 修復後の新しいパス (repaired=trueの場合のみ) */
  newPath?: string;
}

/**
 * 単一のworktreeパスを修復する共通関数
 *
 * パターンA: 期待されるパスにディレクトリが存在する
 *   → git worktree repair <expectedPath> で修復
 *
 * パターンB: ディレクトリが存在しない（リモート環境で作成されたメタデータのみ残っている）
 *   → git worktree remove --force <currentPath> でメタデータを削除
 *
 * @param branch ブランチ名
 * @param currentPath 現在gitメタデータに記録されているパス
 * @param repoRoot リポジトリルートパス
 * @returns 修復結果
 */
export async function repairWorktreePath(
  branch: string,
  currentPath: string,
  repoRoot: string,
): Promise<RepairPathResult> {
  const expectedPath = await generateWorktreePath(repoRoot, branch);
  const fsSync = await import("node:fs");

  if (fsSync.existsSync(expectedPath)) {
    // パターンA: ディレクトリが存在する → repair
    logger.info(
      { branch, expectedPath, currentPath },
      "repairWorktreePath: directory exists, attempting repair",
    );
    try {
      await execa("git", ["worktree", "repair", expectedPath], {
        cwd: repoRoot,
      });
      return { repaired: true, removed: false, newPath: expectedPath };
    } catch (error) {
      logger.warn(
        { branch, expectedPath, error },
        "repairWorktreePath: repair failed",
      );
      return { repaired: false, removed: false };
    }
  } else {
    // パターンB: ディレクトリが存在しない → 古いメタデータを削除
    logger.info(
      { branch, currentPath, expectedPath },
      "repairWorktreePath: directory not found, removing stale metadata",
    );
    try {
      await execa("git", ["worktree", "remove", "--force", currentPath], {
        cwd: repoRoot,
      });
      return { repaired: false, removed: true };
    } catch (error) {
      logger.warn(
        { branch, currentPath, error },
        "repairWorktreePath: remove failed",
      );
      return { repaired: false, removed: false };
    }
  }
}

/**
 * Worktreeのパス不整合を修復
 * 共通関数repairWorktreePathを使用して各ブランチを修復
 * @param targetBranches 修復対象のブランチ名配列
 * @returns 修復結果（成功数、失敗数、失敗詳細）
 */
export async function repairWorktrees(
  targetBranches: string[],
): Promise<RepairResult> {
  const result: RepairResult = {
    repairedCount: 0,
    failedCount: 0,
    failures: [],
  };

  if (targetBranches.length === 0) {
    return result;
  }

  // 修復前のworktree一覧を取得（現在のパスを知るため）
  const beforeWorktrees = await listAdditionalWorktrees();
  const worktreePathMap = new Map(
    beforeWorktrees.map((w) => [w.branch, w.path]),
  );

  // デバッグログ: worktreeリストのブランチ名を出力
  logger.info(
    {
      targetBranches,
      worktreeBranches: beforeWorktrees.map((w) => ({
        branch: w.branch,
        path: w.path,
        isAccessible: w.isAccessible,
      })),
    },
    "repairWorktrees: before state",
  );

  const repoRoot = await getRepositoryRoot();

  // 対象ブランチごとに共通関数を使用して修復
  for (const branch of targetBranches) {
    const currentPath = worktreePathMap.get(branch);
    if (!currentPath) {
      logger.warn({ branch }, "repairWorktrees: branch not found in worktrees");
      result.failedCount++;
      result.failures.push({
        path: branch,
        error: "Branch not found in worktrees",
      });
      continue;
    }

    const repairResult = await repairWorktreePath(
      branch,
      currentPath,
      repoRoot,
    );

    if (repairResult.repaired) {
      result.repairedCount++;
    } else if (repairResult.removed) {
      // メタデータ削除は「修復」としてカウント（新規作成が可能になるため）
      result.repairedCount++;
    } else {
      result.failedCount++;
      result.failures.push({
        path: branch,
        error: "Worktree repair failed",
      });
    }
  }

  // 修復後のアクセス可能性を確認（デバッグログ用）
  const afterWorktrees = await listAdditionalWorktrees();
  logger.info(
    {
      afterWorktrees: afterWorktrees.map((w) => ({
        branch: w.branch,
        path: w.path,
        isAccessible: w.isAccessible,
      })),
    },
    "repairWorktrees: after state",
  );

  return result;
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
 * worktreeに存在しないローカルブランチのクリーンアップ候補を取得
 * @returns {Promise<CleanupTarget[]>} クリーンアップ候補の配列
 */
async function getOrphanedLocalBranchStatuses({
  baseBranch,
  repoRoot,
}: {
  baseBranch: string;
  repoRoot: string;
}): Promise<CleanupStatus[]> {
  try {
    // 並列実行で高速化
    const [localBranches, worktrees] = await Promise.all([
      getLocalBranches(),
      listAdditionalWorktrees(),
    ]);

    const statuses: CleanupStatus[] = [];
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
      if (isProtectedBranchName(localBranch.name)) {
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
        let hasUnpushed = false;
        try {
          hasUnpushed = await hasUnpushedCommitsInRepo(
            localBranch.name,
            repoRoot,
          );
        } catch {
          hasUnpushed = true;
        }

        const { upstream, hasUpstream } = await resolveUpstreamStatus(
          localBranch.name,
          repoRoot,
        );
        const comparisonBase = upstream ?? baseBranch;

        const hasUniqueCommits = await branchHasUniqueCommitsComparedToBase(
          localBranch.name,
          comparisonBase,
          repoRoot,
        );

        const reasons = buildCleanupReasons({
          hasUniqueCommits,
          hasUncommitted: false,
          hasUnpushed,
          hasUpstream,
        });

        if (process.env.DEBUG_CLEANUP) {
          console.log(
            chalk.gray(
              `Debug: Checking orphaned branch ${localBranch.name} -> reasons: ${reasons.join(", ")}`,
            ),
          );
        }

        statuses.push({
          worktreePath: null, // worktreeは存在しない
          branch: localBranch.name,
          hasUncommittedChanges: false, // worktreeが存在しないため常にfalse
          hasUnpushedCommits: hasUnpushed,
          cleanupType: "branch-only",
          hasRemoteBranch: hasUpstream,
          hasUniqueCommits,
          hasUpstream,
          upstream,
          reasons,
        });
      }
    }

    if (process.env.DEBUG_CLEANUP) {
      console.log(
        chalk.cyan(`Debug: Found ${statuses.length} orphaned branch statuses`),
      );
    }

    return statuses;
  } catch (error) {
    console.error(chalk.red("Error: Failed to get orphaned local branches"));
    if (process.env.DEBUG_CLEANUP) {
      console.error(chalk.red("Debug: Full error details:"), error);
    }
    return [];
  }
}

export async function getCleanupStatus(): Promise<CleanupStatus[]> {
  const [config, repoRoot, worktreesWithPR] = await Promise.all([
    getConfig(),
    getRepositoryRoot(),
    getWorktreesWithPRStatus(),
  ]);
  const baseBranch = config.defaultBaseBranch || GIT_CONFIG.DEFAULT_BASE_BRANCH;

  const orphanedStatuses = await getOrphanedLocalBranchStatuses({
    baseBranch,
    repoRoot,
  });
  const statuses: CleanupStatus[] = [];

  if (process.env.DEBUG_CLEANUP) {
    console.log(chalk.cyan("Debug: Available worktrees:"));
    worktreesWithPR.forEach((w) =>
      console.log(`  ${w.branch} -> ${w.worktreePath}`),
    );
  }

  for (const worktree of worktreesWithPR) {
    if (process.env.DEBUG_CLEANUP) {
      console.log(chalk.gray(`Debug: Checking worktree ${worktree.branch}`));
    }

    // worktreeパスの存在を確認
    const fsSync = await import("node:fs");
    const existsSync =
      typeof fsSync.existsSync === "function"
        ? fsSync.existsSync
        : typeof (fsSync as { default?: { existsSync?: unknown } }).default
              ?.existsSync === "function"
          ? (fsSync as { default: { existsSync: (p: string) => boolean } })
              .default.existsSync
          : null;

    const isAccessible = existsSync ? existsSync(worktree.worktreePath) : false;

    let hasUncommitted = false;
    let hasUnpushed = false;

    if (isAccessible) {
      try {
        [hasUncommitted, hasUnpushed] = await Promise.all([
          hasUncommittedChanges(worktree.worktreePath),
          hasUnpushedCommits(worktree.worktreePath, worktree.branch),
        ]);
      } catch (error) {
        if (process.env.DEBUG_CLEANUP) {
          console.log(
            chalk.yellow(
              `Debug: Failed to check status for worktree ${worktree.worktreePath}: ${error instanceof Error ? error.message : String(error)}`,
            ),
          );
        }
      }
    }

    const { upstream, hasUpstream } = await resolveUpstreamStatus(
      worktree.branch,
      repoRoot,
    );
    const comparisonBase = upstream ?? baseBranch;

    const hasUniqueCommits = await branchHasUniqueCommitsComparedToBase(
      worktree.branch,
      comparisonBase,
      repoRoot,
    );

    const reasons = buildCleanupReasons({
      hasUniqueCommits,
      hasUncommitted,
      hasUnpushed,
      hasUpstream,
    });

    if (process.env.DEBUG_CLEANUP) {
      console.log(
        chalk.gray(
          `Debug: Cleanup reasons for ${worktree.branch}: ${reasons.length > 0 ? reasons.join(", ") : "none"}`,
        ),
      );
    }

    statuses.push({
      worktreePath: worktree.worktreePath,
      branch: worktree.branch,
      hasUncommittedChanges: hasUncommitted,
      hasUnpushedCommits: hasUnpushed,
      cleanupType: "worktree-and-branch",
      hasRemoteBranch: hasUpstream,
      hasUniqueCommits,
      hasUpstream,
      upstream,
      isAccessible,
      reasons,
      ...(isAccessible
        ? {}
        : { invalidReason: "Path not accessible in current environment" }),
    });
  }

  statuses.push(...orphanedStatuses);

  if (process.env.DEBUG_CLEANUP) {
    const worktreeTargets = statuses.filter(
      (t) => t.cleanupType === "worktree-and-branch" && t.reasons.length > 0,
    ).length;
    const branchOnlyTargets = statuses.filter(
      (t) => t.cleanupType === "branch-only" && t.reasons.length > 0,
    ).length;
    console.log(
      chalk.cyan(
        `Debug: Found ${worktreeTargets + branchOnlyTargets} cleanup targets (${worktreeTargets} worktree+branch, ${branchOnlyTargets} branch-only)`,
      ),
    );
  }

  return statuses;
}

/**
 * マージ済みPRに関連するworktreeおよびローカルブランチのクリーンアップ候補を取得
 * @returns {Promise<CleanupTarget[]>} クリーンアップ候補の配列
 */
export async function getMergedPRWorktrees(): Promise<CleanupTarget[]> {
  const statuses = await getCleanupStatus();
  const cleanupTargets = statuses
    .filter((status) => status.reasons.length > 0)
    .filter((status) => !isProtectedBranchName(status.branch))
    .map((status) => {
      const target: CleanupTarget = {
        worktreePath: status.worktreePath,
        branch: status.branch,
        pullRequest: null,
        hasUncommittedChanges: status.hasUncommittedChanges,
        hasUnpushedCommits: status.hasUnpushedCommits,
        cleanupType: status.cleanupType,
        hasRemoteBranch: status.hasRemoteBranch,
        isAccessible: status.isAccessible,
        reasons: status.reasons,
      };
      if (status.invalidReason) {
        target.invalidReason = status.invalidReason;
      }
      return target;
    });

  return cleanupTargets;
}
