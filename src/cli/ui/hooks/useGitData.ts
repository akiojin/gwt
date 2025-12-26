import { useState, useEffect, useCallback, useRef } from "react";
import {
  getAllBranches,
  hasUnpushedCommitsInRepo,
  getRepositoryRoot,
  fetchAllRemotes,
  collectUpstreamMap,
  getBranchDivergenceStatuses,
} from "../../../git.js";
import { GIT_CONFIG } from "../../../config/constants.js";
import { listAdditionalWorktrees } from "../../../worktree.js";
import { getPullRequestByBranch } from "../../../github.js";
import type { BranchInfo, WorktreeInfo } from "../types.js";
import type { WorktreeInfo as GitWorktreeInfo } from "../../../worktree.js";
import { getLastToolUsageMap } from "../../../config/index.js";
import { hasUncommittedChanges } from "../../../git.js";

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

export const GIT_DATA_TIMEOUT_MS = 30000; // 30 seconds for large repositories
const PER_BRANCH_TIMEOUT_MS = 5000; // 5 seconds per branch operation

async function withTimeout<T>(
  label: string,
  promise: Promise<T>,
  timeoutMs: number,
  fallback: T,
): Promise<T> {
  let timeoutId: ReturnType<typeof setTimeout> | null = null;
  let timedOut = false;

  const timeoutPromise = new Promise<T>((resolve) => {
    timeoutId = setTimeout(() => {
      timedOut = true;
      resolve(fallback);
    }, timeoutMs);
  });

  const guarded = promise.catch((error) => {
    if (process.env.DEBUG) {
      console.warn(`Failed to resolve ${label}`, error);
    }
    return fallback;
  });

  const result = await Promise.race([guarded, timeoutPromise]);

  if (timedOut && process.env.DEBUG) {
    console.warn(`Timed out waiting for ${label}`);
  }

  if (timeoutId) {
    clearTimeout(timeoutId);
  }

  return result;
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

  // キャッシュ機構: 初回ロード完了を追跡（useRef で再レンダリングを防ぐ）
  const hasLoadedOnceRef = useRef(false);

  const loadData = useCallback(async (forceRefresh = false) => {
    // キャッシュがあり、強制リフレッシュでなければスキップ
    if (hasLoadedOnceRef.current && !forceRefresh) {
      return;
    }

    setLoading(true);
    setError(null);

    try {
      const repoRoot = await withTimeout(
        "repository root",
        getRepositoryRoot(),
        GIT_DATA_TIMEOUT_MS,
        process.cwd(),
      );

      // リモートブランチの最新情報を取得（失敗してもローカル表示は継続）
      void fetchAllRemotes({
        cwd: repoRoot,
        timeoutMs: GIT_CONFIG.FETCH_TIMEOUT,
      }).catch((fetchError) => {
        if (process.env.DEBUG) {
          console.warn("Failed to fetch remote branches", fetchError);
        }
      });

      const branchesData = await withTimeout(
        "branches",
        getAllBranches(repoRoot),
        GIT_DATA_TIMEOUT_MS,
        [],
      );
      let worktreesData: GitWorktreeInfo[] = [];
      try {
        worktreesData = await withTimeout(
          "worktrees",
          listAdditionalWorktrees(),
          GIT_DATA_TIMEOUT_MS,
          [],
        );
      } catch (err) {
        if (process.env.DEBUG) {
          console.error("Failed to list additional worktrees:", err);
        }
        worktreesData = [];
      }

      // enrich worktrees with uncommitted status (only for accessible paths)
      worktreesData = await Promise.all(
        worktreesData.map(async (wt) => {
          if (wt.isAccessible === false) {
            return wt;
          }
          try {
            const hasUncommitted = await withTimeout(
              "worktree status",
              hasUncommittedChanges(wt.path),
              PER_BRANCH_TIMEOUT_MS,
              false,
            );
            return { ...wt, hasUncommittedChanges: hasUncommitted };
          } catch {
            return wt;
          }
        }),
      );

      const lastToolUsageMap = await withTimeout(
        "last tool usage",
        getLastToolUsageMap(repoRoot),
        GIT_DATA_TIMEOUT_MS,
        new Map(),
      );

      // upstream情報とdivergence情報を取得
      const [upstreamMap, divergenceStatuses] = await Promise.all([
        withTimeout(
          "upstream map",
          collectUpstreamMap(repoRoot),
          GIT_DATA_TIMEOUT_MS,
          new Map<string, string>(),
        ),
        withTimeout(
          "divergence",
          getBranchDivergenceStatuses({ cwd: repoRoot }).catch(() => []),
          GIT_DATA_TIMEOUT_MS,
          [],
        ),
      ]);

      // divergenceをMapに変換
      const divergenceMap = new Map(
        divergenceStatuses.map((s) => [
          s.branch,
          {
            ahead: s.localAhead,
            behind: s.remoteAhead,
            upToDate: s.localAhead === 0 && s.remoteAhead === 0,
          },
        ]),
      );

      // ローカルブランチ名のSet（重複除去用）
      const localBranchNames = new Set(
        branchesData.filter((b) => b.type === "local").map((b) => b.name),
      );

      // リモートブランチ名のSet（hasRemoteCounterpart判定用）
      const remoteBranchNames = new Set(
        branchesData
          .filter((b) => b.type === "remote")
          .map((b) => b.name.replace(/^origin\//, "")),
      );

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
          ...(worktree.hasUncommittedChanges !== undefined
            ? { hasUncommittedChanges: worktree.hasUncommittedChanges }
            : {}),
        };
        worktreeMap.set(worktree.branch, uiWorktreeInfo);
      }

      // リモートブランチの重複除去（ローカルに同名がある場合は除外）
      const filteredBranches = branchesData.filter((branch) => {
        if (branch.type === "remote") {
          const remoteBranchBaseName = branch.name.replace(/^origin\//, "");
          return !localBranchNames.has(remoteBranchBaseName);
        }
        return true;
      });

      // Attach worktree info and check unpushed/PR status for local branches
      const enrichedBranches = await Promise.all(
        filteredBranches.map(async (branch) => {
          const worktreeInfo = worktreeMap.get(branch.name);
          let hasUnpushed = false;
          let prInfo = null;

          // Only check unpushed commits and PR status for local branches
          if (branch.type === "local") {
            try {
              // Check for unpushed commits
              [hasUnpushed, prInfo] = await Promise.all([
                withTimeout(
                  "unpushed commits",
                  hasUnpushedCommitsInRepo(branch.name, repoRoot),
                  PER_BRANCH_TIMEOUT_MS,
                  false,
                ),
                withTimeout(
                  "pull request",
                  getPullRequestByBranch(branch.name),
                  PER_BRANCH_TIMEOUT_MS,
                  null,
                ),
              ]);
            } catch (error) {
              // Silently ignore errors to avoid breaking the UI
              if (process.env.DEBUG) {
                console.error(
                  `Failed to check status for ${branch.name}:`,
                  error,
                );
              }
            }
          }

          // upstream/divergence/hasRemoteCounterpart情報を付加
          const upstream =
            branch.type === "local"
              ? (upstreamMap.get(branch.name) ?? null)
              : null;
          const divergence =
            branch.type === "local"
              ? (divergenceMap.get(branch.name) ?? null)
              : null;
          const hasRemoteCounterpart =
            branch.type === "local"
              ? remoteBranchNames.has(branch.name)
              : false;

          return {
            ...branch,
            ...(worktreeInfo ? { worktree: worktreeInfo } : {}),
            ...(hasUnpushed ? { hasUnpushedCommits: true } : {}),
            ...(prInfo?.state === "OPEN"
              ? { openPR: { number: prInfo.number, title: prInfo.title } }
              : {}),
            ...(prInfo?.state === "MERGED" && prInfo.mergedAt
              ? {
                  mergedPR: {
                    number: prInfo.number,
                    mergedAt: prInfo.mergedAt,
                  },
                }
              : {}),
            ...(lastToolUsageMap.get(branch.name)
              ? { lastToolUsage: lastToolUsageMap.get(branch.name) ?? null }
              : {}),
            ...(upstream !== null ? { upstream } : {}),
            ...(divergence !== null ? { divergence } : {}),
            ...(hasRemoteCounterpart ? { hasRemoteCounterpart } : {}),
          };
        }),
      );

      setBranches(enrichedBranches);
      setLastUpdated(new Date());
      hasLoadedOnceRef.current = true; // 初回ロード完了をマーク
    } catch (err) {
      setError(err instanceof Error ? err : new Error(String(err)));
      setBranches([]);
      setWorktrees([]);
    } finally {
      setLoading(false);
    }
  }, []);

  const refresh = useCallback(() => {
    loadData(true); // forceRefresh = true で強制的にデータを再取得
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
