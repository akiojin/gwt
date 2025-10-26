import type { BranchInfo, Statistics } from "../types.js";

/**
 * Calculates statistics from branch data
 * @param branches Array of BranchInfo
 * @param changedBranches Optional set of branch names with uncommitted changes
 * @returns Statistics object
 */
export function calculateStatistics(
  branches: BranchInfo[],
  changedBranches: Set<string> = new Set(),
): Statistics {
  let localCount = 0;
  let remoteCount = 0;
  let worktreeCount = 0;
  let changesCount = 0;

  for (const branch of branches) {
    // Count by type
    if (branch.type === "local") {
      localCount++;

      // Count worktrees (only for local branches)
      if (branch.worktree) {
        worktreeCount++;

        // Count changes (only for branches with worktrees)
        if (changedBranches.has(branch.name)) {
          changesCount++;
        }
      }
    } else if (branch.type === "remote") {
      remoteCount++;
    }
  }

  return {
    localCount,
    remoteCount,
    worktreeCount,
    changesCount,
    lastUpdated: new Date(),
  };
}
