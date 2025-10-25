import React, { useCallback, useMemo } from 'react';
import { ErrorBoundary } from './common/ErrorBoundary.js';
import { BranchListScreen } from './screens/BranchListScreen.js';
import { WorktreeManagerScreen } from './screens/WorktreeManagerScreen.js';
import { BranchCreatorScreen } from './screens/BranchCreatorScreen.js';
import type { WorktreeItem } from './screens/WorktreeManagerScreen.js';
import { useGitData } from '../hooks/useGitData.js';
import { useScreenState } from '../hooks/useScreenState.js';
import { formatBranchItems } from '../utils/branchFormatter.js';
import { calculateStatistics } from '../utils/statisticsCalculator.js';
import type { BranchItem } from '../types.js';

export interface AppProps {
  onExit: (selectedBranch?: string) => void;
}

/**
 * App - Top-level component for Ink.js UI
 * Integrates ErrorBoundary, data fetching, screen navigation, and all screens
 */
export function App({ onExit }: AppProps) {
  const { branches, worktrees, loading, error, refresh } = useGitData();
  const { currentScreen, navigateTo, goBack, reset } = useScreenState();

  // Format branches to BranchItems
  const branchItems: BranchItem[] = formatBranchItems(branches);

  // Calculate statistics
  const stats = calculateStatistics(branches);

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

  // Handle branch selection
  const handleSelect = useCallback(
    (item: BranchItem) => {
      onExit(item.name);
    },
    [onExit]
  );

  // Handle navigation
  const handleNavigate = useCallback(
    (screen: string) => {
      navigateTo(screen as any);
    },
    [navigateTo]
  );

  // Handle quit
  const handleQuit = useCallback(() => {
    onExit();
  }, [onExit]);

  // Handle branch creation
  const handleCreate = useCallback(
    (branchName: string) => {
      // TODO: Implement branch creation logic (git.js integration)
      // For now, just go back to branch list
      goBack();
      refresh();
    },
    [goBack, refresh]
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
          />
        );

      case 'worktree-manager':
        return (
          <WorktreeManagerScreen
            worktrees={worktreeItems}
            onBack={goBack}
            onSelect={(worktree) => {
              // TODO: Implement worktree selection logic
              goBack();
            }}
          />
        );

      case 'branch-creator':
        return <BranchCreatorScreen onBack={goBack} onCreate={handleCreate} />;

      case 'pr-cleanup':
        // TODO: Implement PRCleanupScreen
        return <BranchListScreen branches={branchItems} stats={stats} onSelect={handleSelect} />;

      case 'ai-tool-selector':
        // TODO: Implement AIToolSelectorScreen
        return <BranchListScreen branches={branchItems} stats={stats} onSelect={handleSelect} />;

      case 'session-selector':
        // TODO: Implement SessionSelectorScreen
        return <BranchListScreen branches={branchItems} stats={stats} onSelect={handleSelect} />;

      case 'execution-mode-selector':
        // TODO: Implement ExecutionModeSelectorScreen
        return <BranchListScreen branches={branchItems} stats={stats} onSelect={handleSelect} />;

      default:
        return (
          <BranchListScreen
            branches={branchItems}
            stats={stats}
            onSelect={handleSelect}
            loading={loading}
            error={error}
          />
        );
    }
  };

  return <ErrorBoundary>{renderScreen()}</ErrorBoundary>;
}
