/**
 * Branch Service
 *
 * ブランチ一覧取得とマージステータス判定のビジネスロジック。
 * 既存のgit.tsとgithub.tsの機能を活用します。
 */

import { getBranches, getMainBranch } from "../../../git.js";
import { getPullRequestByBranch } from "../../../github.js";
import { getWorktrees } from "../../../worktree.js";
import type { Branch } from "../../../types/api.js";

/**
 * すべてのブランチ一覧を取得（マージステータスとWorktree情報付き）
 */
export async function listBranches(): Promise<Branch[]> {
  const branches = await getBranches();
  const worktrees = await getWorktrees();
  const mainBranch = await getMainBranch();

  const branchList: Branch[] = await Promise.all(
    branches.map(async (branchInfo) => {
      // Worktree情報
      const worktree = worktrees.find((wt) => wt.branch === branchInfo.name);

      // マージステータス判定（GitHub PRベース）
      let mergeStatus: "unmerged" | "merged" | "unknown" = "unknown";
      const pr = await getPullRequestByBranch(branchInfo.name);
      if (pr) {
        mergeStatus = pr.state === "MERGED" ? "merged" : "unmerged";
      } else if (branchInfo.name === mainBranch) {
        // メインブランチは常にmerged扱い
        mergeStatus = "merged";
      }

      return {
        name: branchInfo.name,
        type: branchInfo.isRemote ? "remote" : "local",
        commitHash: branchInfo.hash,
        commitMessage: branchInfo.message || null,
        author: branchInfo.author || null,
        commitDate: branchInfo.date || null,
        mergeStatus,
        worktreePath: worktree?.path || null,
        divergence: branchInfo.divergence || null,
      };
    }),
  );

  return branchList;
}

/**
 * 特定のブランチ情報を取得
 */
export async function getBranchByName(
  branchName: string,
): Promise<Branch | null> {
  const branches = await listBranches();
  return branches.find((b) => b.name === branchName) || null;
}
