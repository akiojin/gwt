import React, { useCallback } from 'react';
import { ErrorBoundary } from './common/ErrorBoundary.js';
import { BranchListScreen } from './screens/BranchListScreen.js';
import { useGitData } from '../hooks/useGitData.js';
import { formatBranchItems } from '../utils/branchFormatter.js';
import { calculateStatistics } from '../utils/statisticsCalculator.js';
import type { BranchItem } from '../types.js';

export interface AppProps {
  onExit: (selectedBranch?: string) => void;
}

/**
 * App - Top-level component for Ink.js UI
 * Integrates ErrorBoundary, data fetching, and main screen
 */
export function App({ onExit }: AppProps) {
  const { branches, loading, error, refresh } = useGitData();

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

  return (
    <ErrorBoundary>
      <BranchListScreen
        branches={branchItems}
        stats={stats}
        onSelect={handleSelect}
        loading={loading}
        error={error}
      />
    </ErrorBoundary>
  );
}
