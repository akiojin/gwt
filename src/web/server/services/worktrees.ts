/**
 * Worktree Service
 *
 * Worktree管理のビジネスロジック。
 * 既存のworktree.tsの機能を活用します。
 */

import {
  getWorktrees as getWorktreesFromGit,
  createWorktree,
  deleteWorktree,
} from "../../../worktree.js";
import type { Worktree } from "../../../types/api.js";

/**
 * すべてのWorktree一覧を取得
 */
export async function listWorktrees(): Promise<Worktree[]> {
  const worktrees = await getWorktreesFromGit();

  return worktrees.map((wt) => ({
    path: wt.path,
    branchName: wt.branch,
    head: wt.head,
    isLocked: wt.locked,
    isPrunable: wt.prunable,
    createdAt: null, // git worktreeからは取得不可
    lastAccessedAt: null, // git worktreeからは取得不可
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
  const worktreePath = await createWorktree(branchName, createBranch);

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
  await deleteWorktree(path);
}
