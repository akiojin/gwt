import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useApp } from 'ink';
import { ErrorBoundary } from './common/ErrorBoundary.js';
import { BranchListScreen } from './screens/BranchListScreen.js';
import { WorktreeManagerScreen } from './screens/WorktreeManagerScreen.js';
import { BranchCreatorScreen } from './screens/BranchCreatorScreen.js';
import { PRCleanupScreen } from './screens/PRCleanupScreen.js';
import { AIToolSelectorScreen } from './screens/AIToolSelectorScreen.js';
import { SessionSelectorScreen } from './screens/SessionSelectorScreen.js';
import { ExecutionModeSelectorScreen } from './screens/ExecutionModeSelectorScreen.js';
import type { AITool } from './screens/AIToolSelectorScreen.js';
import type { ExecutionMode } from './screens/ExecutionModeSelectorScreen.js';
import type { WorktreeItem } from './screens/WorktreeManagerScreen.js';
import { useGitData } from '../hooks/useGitData.js';
import { useScreenState } from '../hooks/useScreenState.js';
import { formatBranchItems } from '../utils/branchFormatter.js';
import { calculateStatistics } from '../utils/statisticsCalculator.js';
import type { BranchItem, CleanupTarget } from '../types.js';
import { getRepositoryRoot, deleteBranch } from '../../git.js';
import {
  createWorktree,
  generateWorktreePath,
  getMergedPRWorktrees,
  removeWorktree,
} from '../../worktree.js';

export interface SelectionResult {
  branch: string;
  tool: AITool;
  mode: ExecutionMode;
  skipPermissions: boolean;
}

export interface AppProps {
  onExit: (result?: SelectionResult) => void;
  loadingIndicatorDelay?: number;
}

/**
 * App - Top-level component for Ink.js UI
 * Integrates ErrorBoundary, data fetching, screen navigation, and all screens
 */
export function App({ onExit, loadingIndicatorDelay = 300 }: AppProps) {
  const { exit } = useApp();
  const { branches, worktrees, loading, error, refresh, lastUpdated } = useGitData();
  const { currentScreen, navigateTo, goBack, reset } = useScreenState();

  // Selection state (for branch → tool → mode flow)
  const [selectedBranch, setSelectedBranch] = useState<string | null>(null);
  const [selectedTool, setSelectedTool] = useState<AITool | null>(null);

  // PR cleanup state
  const [cleanupTargets, setCleanupTargets] = useState<CleanupTarget[]>([]);
  const [cleanupLoading, setCleanupLoading] = useState(false);
  const [cleanupError, setCleanupError] = useState<Error | null>(null);
  const [cleanupStatus, setCleanupStatus] = useState<string | null>(null);

  // Format branches to BranchItems (memoized for performance)
  const branchItems: BranchItem[] = useMemo(() => formatBranchItems(branches), [branches]);

  // Calculate statistics (memoized for performance)
  const stats = useMemo(() => calculateStatistics(branches), [branches]);

  // Format worktrees to WorktreeItems
  const worktreeItems: WorktreeItem[] = useMemo(
    () =>
      worktrees.map((wt): WorktreeItem => ({
        branch: wt.branch,
        path: wt.path,
        isAccessible: wt.isAccessible ?? true,
      })),
    [worktrees]
  );

  const resolveBaseBranch = useCallback(() => {
    const localMain = branches.find(
      (branch) =>
        branch.type === 'local' && (branch.name === 'main' || branch.name === 'master')
    );
    if (localMain) {
      return localMain.name;
    }

    const develop = branches.find(
      (branch) => branch.type === 'local' && (branch.name === 'develop' || branch.name === 'dev')
    );
    if (develop) {
      return develop.name;
    }

    return 'main';
  }, [branches]);

  // Handle branch selection
  const handleSelect = useCallback(
    (item: BranchItem) => {
      setSelectedBranch(item.name);
      navigateTo('ai-tool-selector');
    },
    [navigateTo]
  );

  const loadCleanupTargets = useCallback(async () => {
    setCleanupLoading(true);
    setCleanupError(null);
    try {
      const targets = await getMergedPRWorktrees();
      setCleanupTargets(targets);
    } catch (err) {
      setCleanupTargets([]);
      setCleanupError(err instanceof Error ? err : new Error(String(err)));
    } finally {
      setCleanupLoading(false);
    }
  }, []);

  useEffect(() => {
    if (currentScreen === 'pr-cleanup') {
      loadCleanupTargets();
    }
  }, [currentScreen, loadCleanupTargets]);

  // Handle navigation
  const handleNavigate = useCallback(
    (screen: string) => {
      if (screen === 'pr-cleanup') {
        setCleanupStatus(null);
      }
      navigateTo(screen as any);
    },
    [navigateTo]
  );

  const handleWorktreeSelect = useCallback(
    (worktree: WorktreeItem) => {
      setSelectedBranch(worktree.branch);
      navigateTo('ai-tool-selector');
    },
    [navigateTo]
  );

  const handleCleanupRefresh = useCallback(() => {
    setCleanupStatus(null);
    setCleanupError(null);
    loadCleanupTargets();
  }, [loadCleanupTargets]);

  // Handle quit
  const handleQuit = useCallback(() => {
    onExit();
    exit();
  }, [onExit, exit]);

  // Handle branch creation
  const handleCreate = useCallback(
    async (branchName: string) => {
      try {
        const repoRoot = await getRepositoryRoot();
        const worktreePath = await generateWorktreePath(repoRoot, branchName);
        const baseBranch = resolveBaseBranch();

        await createWorktree({
          branchName,
          worktreePath,
          repoRoot,
          isNewBranch: true,
          baseBranch,
        });

        refresh();
        setSelectedBranch(branchName);

        navigateTo('ai-tool-selector');
      } catch (error) {
        // On error, go back to branch list
        console.error('Failed to create branch:', error);
        goBack();
        refresh();
      }
    },
    [navigateTo, goBack, refresh, resolveBaseBranch]
  );

  // Handle PR cleanup
  const handleCleanup = useCallback(
    async (target: CleanupTarget) => {
      setCleanupError(null);

      if (target.hasUncommittedChanges || target.hasUnpushedCommits) {
        setCleanupStatus(
          `[WARN] ${target.branch} に未コミットまたは未プッシュの変更があるためクリーンアップをスキップしました。`
        );
        return;
      }

      if (target.cleanupType === 'worktree-and-branch') {
        if (!target.worktreePath) {
          setCleanupStatus(
            `[WARN] ${target.branch} のworktreeパスが特定できないためクリーンアップをスキップしました。`
          );
          return;
        }

        if (target.isAccessible === false) {
          setCleanupStatus(
            `[WARN] ${target.branch} のworktreeにアクセスできないためクリーンアップをスキップしました。`
          );
          return;
        }

        try {
          await removeWorktree(target.worktreePath, true);
        } catch (err) {
          const message = err instanceof Error ? err.message : String(err);
          setCleanupError(new Error(message));
          setCleanupStatus(`[ERROR] ${target.branch} のworktree削除に失敗しました。`);
          return;
        }
      }

      try {
        await deleteBranch(target.branch, true);
        setCleanupStatus(`[OK] ${target.branch} をクリーンアップしました。`);
        await loadCleanupTargets();
        refresh();
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setCleanupError(new Error(message));
        setCleanupStatus(`[ERROR] ${target.branch} のブランチ削除に失敗しました。`);
      }
    },
    [deleteBranch, loadCleanupTargets, refresh, removeWorktree]
  );

  // Handle AI tool selection
  const handleToolSelect = useCallback(
    (tool: AITool) => {
      setSelectedTool(tool);
      navigateTo('execution-mode-selector');
    },
    [navigateTo]
  );

  // Handle session selection
  const handleSessionSelect = useCallback(
    (session: string) => {
      // TODO: Load selected session and navigate to next screen
      // For now, just go back to branch list
      goBack();
    },
    [goBack]
  );

  // Handle execution mode and skipPermissions selection
  const handleModeSelect = useCallback(
    (result: { mode: ExecutionMode; skipPermissions: boolean }) => {
      // All selections complete - exit with result
      if (selectedBranch && selectedTool) {
        onExit({
          branch: selectedBranch,
          tool: selectedTool,
          mode: result.mode,
          skipPermissions: result.skipPermissions,
        });
        exit();
      }
    },
    [selectedBranch, selectedTool, onExit, exit]
  );

  // Render screen based on currentScreen
  const renderScreen = () => {
    switch (currentScreen) {
      case 'branch-list':
        return (
          <BranchListScreen
            branches={branchItems}
            stats={stats}
            onSelect={handleSelect}
            onNavigate={handleNavigate}
            onQuit={handleQuit}
            loading={loading}
            error={error}
            lastUpdated={lastUpdated}
            loadingIndicatorDelay={loadingIndicatorDelay}
          />
        );

      case 'worktree-manager':
        return (
          <WorktreeManagerScreen
            worktrees={worktreeItems}
            onBack={goBack}
            onSelect={handleWorktreeSelect}
          />
        );

      case 'branch-creator':
        return <BranchCreatorScreen onBack={goBack} onCreate={handleCreate} />;

      case 'pr-cleanup':
        return (
          <PRCleanupScreen
            targets={cleanupTargets}
            loading={cleanupLoading}
            error={cleanupError}
            statusMessage={cleanupStatus}
            onBack={goBack}
            onRefresh={handleCleanupRefresh}
            onCleanup={handleCleanup}
          />
        );

      case 'ai-tool-selector':
        return <AIToolSelectorScreen onBack={goBack} onSelect={handleToolSelect} />;

      case 'session-selector':
        // TODO: Implement session data fetching
        return (
          <SessionSelectorScreen
            sessions={[]}
            onBack={goBack}
            onSelect={handleSessionSelect}
          />
        );

      case 'execution-mode-selector':
        return (
          <ExecutionModeSelectorScreen onBack={goBack} onSelect={handleModeSelect} />
        );

      default:
        return (
          <BranchListScreen
            branches={branchItems}
            stats={stats}
            onSelect={handleSelect}
            onNavigate={handleNavigate}
            onQuit={handleQuit}
            loading={loading}
            error={error}
            lastUpdated={lastUpdated}
            loadingIndicatorDelay={loadingIndicatorDelay}
          />
        );
    }
  };

  return <ErrorBoundary>{renderScreen()}</ErrorBoundary>;
}
