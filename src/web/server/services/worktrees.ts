/**
 * Worktree Service
 *
 * Worktree管理のビジネスロジック。
 * 既存のworktree.tsの機能を活用します。
 */

import {
  listAdditionalWorktrees,
  createWorktree as createWorktreeCore,
  removeWorktree as removeWorktreeCore,
  generateWorktreePath,
  isProtectedBranchName,
  type WorktreeInfo,
} from "../../../worktree.js";
import type { Worktree } from "../../../types/api.js";

/**
 * すべてのWorktree一覧を取得
 */
export async function listWorktrees(): Promise<Worktree[]> {
  const worktrees = await listAdditionalWorktrees();

  return worktrees.map((wt: WorktreeInfo) => ({
    path: wt.path,
    branchName: wt.branch,
    head: wt.head,
    isLocked: false, // TODO: locked情報を取得
    isPrunable: false, // TODO: prunable情報を取得
    isProtected: isProtectedBranchName(wt.branch),
    createdAt: null, // git worktreeからは取得不可
    lastAccessedAt: null, // git worktreeからは取得不可
    divergence: null,
    prInfo: null,
  }));
}

/**
 * 特定のWorktree情報を取得
 */
export async function getWorktreeByPath(
  path: string,
): Promise<Worktree | null> {
  const worktrees = await listWorktrees();
  return worktrees.find((wt) => wt.path === path) || null;
}

/**
 * 新しいWorktreeを作成
 */
export async function createNewWorktree(
  branchName: string,
  createBranch: boolean,
): Promise<Worktree> {
  const { getRepositoryRoot, getCurrentBranch } = await import(
    "../../../git.js"
  );

  const [repoRoot, currentBranch] = await Promise.all([
    getRepositoryRoot(),
    getCurrentBranch(),
  ]);

  const worktreePath = await generateWorktreePath(branchName, repoRoot);

  await createWorktreeCore({
    branchName,
    worktreePath,
    repoRoot,
    isNewBranch: createBranch,
    baseBranch: currentBranch || "main",
  });

  const worktree = await getWorktreeByPath(worktreePath);
  if (!worktree) {
    throw new Error(`Failed to create worktree for branch ${branchName}`);
  }

  return worktree;
}

/**
 * Worktreeを削除
 */
export async function removeWorktree(path: string): Promise<void> {
  await removeWorktreeCore(path);
}
