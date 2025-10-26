import { useState, useEffect, useCallback } from 'react';
import { getAllBranches } from '../../git.js';
import { listAdditionalWorktrees } from '../../worktree.js';
import type { BranchInfo, WorktreeInfo } from '../types.js';
import type { WorktreeInfo as GitWorktreeInfo } from '../../worktree.js';

export interface UseGitDataOptions {
  enableAutoRefresh?: boolean;
  refreshInterval?: number; // milliseconds (default: 5000ms = 5s)
}

export interface UseGitDataResult {
  branches: BranchInfo[];
  worktrees: GitWorktreeInfo[];
  loading: boolean;
  error: Error | null;
  refresh: () => void;
  lastUpdated: Date | null;
}

/**
 * Hook to fetch and manage Git data (branches and worktrees)
 * @param options - Configuration options for auto-refresh and polling interval
 */
export function useGitData(options?: UseGitDataOptions): UseGitDataResult {
  const { enableAutoRefresh = false, refreshInterval = 5000 } = options || {};
  const [branches, setBranches] = useState<BranchInfo[]>([]);
  const [worktrees, setWorktrees] = useState<GitWorktreeInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);

  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);

    try {
      const [branchesData, worktreesData] = await Promise.all([
        getAllBranches(),
        listAdditionalWorktrees(),
      ]);

      // Store worktrees separately
      setWorktrees(worktreesData);

      // Map worktrees to branches
      const worktreeMap = new Map<string, WorktreeInfo>();
      for (const worktree of worktreesData) {
        // Convert worktree.ts WorktreeInfo to ui/types.ts WorktreeInfo
        const uiWorktreeInfo: WorktreeInfo = {
          path: worktree.path,
          locked: false, // worktree.ts doesn't expose locked status
          prunable: worktree.isAccessible === false,
          isAccessible: worktree.isAccessible ?? true, // Default to true if undefined
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
      setLastUpdated(new Date());
    } catch (err) {
      setError(err instanceof Error ? err : new Error(String(err)));
      setBranches([]);
      setWorktrees([]);
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

  // Auto-refresh polling (if enabled)
  useEffect(() => {
    if (!enableAutoRefresh) {
      return;
    }

    const intervalId = setInterval(() => {
      loadData();
    }, refreshInterval);

    return () => {
      clearInterval(intervalId);
    };
  }, [enableAutoRefresh, refreshInterval, loadData]);

  return {
    branches,
    worktrees,
    loading,
    error,
    refresh,
    lastUpdated,
  };
}
