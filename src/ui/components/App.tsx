import React, { useCallback } from 'react';
import { ErrorBoundary } from './common/ErrorBoundary.js';
import { BranchListScreen } from './screens/BranchListScreen.js';
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
  const { branches, loading, error, refresh } = useGitData();
  const { currentScreen, navigateTo, goBack, reset } = useScreenState();

  // Format branches to BranchItems
  const branchItems: BranchItem[] = formatBranchItems(branches);

  // Calculate statistics
  const stats = calculateStatistics(branches);

  // Handle branch selection
  const handleSelect = useCallback(
    (item: BranchItem) => {
      onExit(item.name);
    },
    [onExit]
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
            loading={loading}
            error={error}
          />
        );

      case 'worktree-manager':
        // TODO: Implement WorktreeManagerScreen
        return <BranchListScreen branches={branchItems} stats={stats} onSelect={handleSelect} />;

      case 'branch-creator':
        // TODO: Implement BranchCreatorScreen
        return <BranchListScreen branches={branchItems} stats={stats} onSelect={handleSelect} />;

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
