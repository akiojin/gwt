import { useState, useEffect, useCallback } from 'react';
import { getAllBranches } from '../../git.js';
import { listAdditionalWorktrees } from '../../worktree.js';
import type { BranchInfo, WorktreeInfo } from '../types.js';

export interface UseGitDataResult {
  branches: BranchInfo[];
  loading: boolean;
  error: Error | null;
  refresh: () => void;
}

/**
 * Hook to fetch and manage Git data (branches and worktrees)
 */
export function useGitData(): UseGitDataResult {
  const [branches, setBranches] = useState<BranchInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);

    try {
      const [branchesData, worktreesData] = await Promise.all([
        getAllBranches(),
        listAdditionalWorktrees(),
      ]);

      // Map worktrees to branches
      const worktreeMap = new Map<string, WorktreeInfo>();
      for (const worktree of worktreesData) {
        // Convert worktree.ts WorktreeInfo to ui/types.ts WorktreeInfo
        const uiWorktreeInfo: WorktreeInfo = {
          path: worktree.path,
          locked: false, // worktree.ts doesn't expose locked status
          prunable: worktree.isAccessible === false,
        };
        worktreeMap.set(worktree.branch, uiWorktreeInfo);
      }

      // Attach worktree info to matching branches
      const enrichedBranches = branchesData.map((branch) => {
        const worktreeInfo = worktreeMap.get(branch.name);
        if (worktreeInfo) {
          return {
            ...branch,
            worktree: worktreeInfo,
          };
        }
        return branch;
      });

      setBranches(enrichedBranches);
    } catch (err) {
      setError(err instanceof Error ? err : new Error(String(err)));
      setBranches([]);
    } finally {
      setLoading(false);
    }
  }, []);

  const refresh = useCallback(() => {
    loadData();
  }, [loadData]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  return {
    branches,
    loading,
    error,
    refresh,
  };
}
