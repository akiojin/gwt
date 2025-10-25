import React from 'react';
import { Box, Text } from 'ink';
import { Header } from '../parts/Header.js';
import { Stats } from '../parts/Stats.js';
import { Footer } from '../parts/Footer.js';
import { Select } from '../common/Select.js';
import { useTerminalSize } from '../../hooks/useTerminalSize.js';
import type { BranchItem, Statistics } from '../../types.js';

export interface BranchListScreenProps {
  branches: BranchItem[];
  stats: Statistics;
  onSelect: (branch: BranchItem) => void;
  loading?: boolean;
  error?: Error | null;
}

/**
 * BranchListScreen - Main screen for branch selection
 * Layout: Header + Stats + Branch List + Footer
 */
export function BranchListScreen({
  branches,
  stats,
  onSelect,
  loading = false,
  error = null,
}: BranchListScreenProps) {
  const { rows } = useTerminalSize();

  // Calculate available space for branch list
  // Header: 2 lines (title + divider)
  // Stats: 1 line
  // Empty line: 1 line
  // Footer: 1 line
  // Total fixed: 5 lines
  const headerLines = 2;
  const statsLines = 1;
  const emptyLine = 1;
  const footerLines = 1;
  const fixedLines = headerLines + statsLines + emptyLine + footerLines;
  const contentHeight = rows - fixedLines;
  const limit = Math.max(5, contentHeight); // Minimum 5 items visible

  // Footer actions
  const footerActions = [
    { key: 'enter', description: 'Select' },
    { key: 'q', description: 'Quit' },
  ];

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header title="Claude Worktree - Branch Selection" titleColor="cyan" />

      {/* Stats */}
      <Box marginTop={1}>
        <Stats stats={stats} />
      </Box>

      {/* Empty line */}
      <Box height={1} />

      {/* Content */}
      <Box flexDirection="column" flexGrow={1}>
        {loading && (
          <Box>
            <Text color="yellow">Loading branches...</Text>
          </Box>
        )}

        {error && (
          <Box flexDirection="column">
            <Text color="red" bold>
              Error: {error.message}
            </Text>
          </Box>
        )}

        {!loading && !error && branches.length === 0 && (
          <Box>
            <Text dimColor>No branches found</Text>
          </Box>
        )}

        {!loading && !error && branches.length > 0 && (
          <Select items={branches} onSelect={onSelect} limit={limit} />
        )}
      </Box>

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
