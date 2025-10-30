import React, { useCallback, useMemo, useState } from 'react';
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
import type { BranchItem, MergedPullRequest } from '../types.js';
import { createBranch } from '../../git.js';

export interface SelectionResult {
  branch: string;
  tool: AITool;
  mode: ExecutionMode;
  skipPermissions: boolean;
}

export interface AppProps {
  onExit: (result?: SelectionResult) => void;
}

/**
 * App - Top-level component for Ink.js UI
 * Integrates ErrorBoundary, data fetching, screen navigation, and all screens
 */
export function App({ onExit }: AppProps) {
  const { exit } = useApp();
  const { branches, worktrees, loading, error, refresh, lastUpdated } = useGitData({
    enableAutoRefresh: true,
    refreshInterval: 5000, // 5 seconds
  });
  const { currentScreen, navigateTo, goBack, reset } = useScreenState();

  // Selection state (for branch → tool → mode flow)
  const [selectedBranch, setSelectedBranch] = useState<string | null>(null);
  const [selectedTool, setSelectedTool] = useState<AITool | null>(null);

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

  // Handle branch selection
  const handleSelect = useCallback(
    (item: BranchItem) => {
      setSelectedBranch(item.name);
      navigateTo('ai-tool-selector');
    },
    [navigateTo]
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
    exit();
  }, [onExit, exit]);

  // Handle branch creation
  const handleCreate = useCallback(
    async (branchName: string) => {
      try {
        // 1. Create branch using git.js
        await createBranch(branchName, 'main');

        // 2. Set the newly created branch as selected
        setSelectedBranch(branchName);

        // 3. Navigate to AI tool selector (same flow as branch selection)
        navigateTo('ai-tool-selector');
      } catch (error) {
        // On error, go back to branch list
        console.error('Failed to create branch:', error);
        goBack();
        refresh();
      }
    },
    [navigateTo, goBack, refresh]
  );

  // Handle PR cleanup
  const handleCleanup = useCallback(
    (pr: MergedPullRequest) => {
      // TODO: Implement PR cleanup logic (github.js integration)
      // For now, just go back to branch list
      goBack();
      refresh();
    },
    [goBack, refresh]
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
        // TODO: Implement merged PR data fetching
        return <PRCleanupScreen pullRequests={[]} onBack={goBack} onCleanup={handleCleanup} />;

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
          />
        );
    }
  };

  return <ErrorBoundary>{renderScreen()}</ErrorBoundary>;
}
