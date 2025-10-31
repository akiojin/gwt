import React from 'react';
import { Box, Text, useInput } from 'ink';
import { Header } from '../parts/Header.js';
import { Stats } from '../parts/Stats.js';
import { Footer } from '../parts/Footer.js';
import { Select } from '../common/Select.js';
import { LoadingIndicator } from '../common/LoadingIndicator.js';
import { useTerminalSize } from '../../hooks/useTerminalSize.js';
import type { BranchItem, Statistics } from '../../types.js';

type IndicatorColor = 'cyan' | 'green' | 'yellow' | 'red';

interface CleanupIndicator {
  icon: string;
  color?: IndicatorColor;
}

interface CleanupFooterMessage {
  text: string;
  color?: IndicatorColor;
}

interface CleanupUIState {
  indicators: Record<string, CleanupIndicator>;
  footerMessage: CleanupFooterMessage | null;
  inputLocked: boolean;
}

export interface BranchListScreenProps {
  branches: BranchItem[];
  stats: Statistics;
  onSelect: (branch: BranchItem) => void;
  onNavigate?: (screen: string) => void;
  onQuit?: () => void;
  onCleanupCommand?: () => void;
  onRefresh?: () => void;
  loading?: boolean;
  error?: Error | null;
  lastUpdated?: Date | null;
  loadingIndicatorDelay?: number;
  cleanupUI?: CleanupUIState;
  version?: string | null;
}

/**
 * BranchListScreen - Main screen for branch selection
 * Layout: Header + Stats + Branch List + Footer
 */
export function BranchListScreen({
  branches,
  stats,
  onSelect,
  onNavigate,
  onQuit,
  onCleanupCommand,
  onRefresh,
  loading = false,
  error = null,
  lastUpdated = null,
  loadingIndicatorDelay = 300,
  cleanupUI,
  version,
}: BranchListScreenProps) {
  const { rows } = useTerminalSize();

  // Handle keyboard input
  // Note: Select component handles Enter and arrow keys
  useInput((input, key) => {
    if (cleanupUI?.inputLocked) {
      return;
    }

    if (input === 'm' && onNavigate) {
      onNavigate('worktree-manager');
    } else if (input === 'c') {
      onCleanupCommand?.();
    } else if (input === 'r' && onRefresh) {
      onRefresh();
    }
  });

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
    { key: 'r', description: 'Refresh' },
    { key: 'm', description: 'Manage worktrees' },
    { key: 'c', description: 'Cleanup branches' },
  ];

  const renderIndicator = (item: BranchItem, isSelected: boolean) => {
    const indicator = cleanupUI?.indicators?.[item.name];

    if (indicator) {
      const color = indicator.color ?? (isSelected ? 'cyan' : undefined);
      if (color) {
        return <Text color={color}>{indicator.icon}</Text>;
      }
      return <Text>{indicator.icon}</Text>;
    }

    return isSelected ? <Text color="cyan">›</Text> : <Text> </Text>;
  };

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header title="Claude Worktree - Branch Selection" titleColor="cyan" version={version} />

      {/* Stats */}
      <Box marginTop={1}>
        <Stats stats={stats} lastUpdated={lastUpdated} />
      </Box>

      {/* Content */}
      <Box flexDirection="column" flexGrow={1}>
        <LoadingIndicator
          isLoading={Boolean(loading)}
          delay={loadingIndicatorDelay}
          message="Git情報を読み込んでいます..."
        />

        {error && (
          <Box flexDirection="column">
            <Text color="red" bold>
              Error: {error.message}
            </Text>
            {process.env.DEBUG && error.stack && (
              <Box marginTop={1}>
                <Text color="gray">{error.stack}</Text>
              </Box>
            )}
          </Box>
        )}

        {!loading && !error && branches.length === 0 && (
          <Box>
            <Text dimColor>No branches found</Text>
          </Box>
        )}

        {!loading && !error && branches.length > 0 && (
          <Select
            items={branches}
            onSelect={onSelect}
            limit={limit}
            disabled={Boolean(cleanupUI?.inputLocked)}
            renderIndicator={renderIndicator}
          />
        )}
      </Box>

      {cleanupUI?.footerMessage && (
        <Box marginBottom={1}>
          {cleanupUI.footerMessage.color ? (
            <Text color={cleanupUI.footerMessage.color}>
              {cleanupUI.footerMessage.text}
            </Text>
          ) : (
            <Text>{cleanupUI.footerMessage.text}</Text>
          )}
        </Box>
      )}

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
