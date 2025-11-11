import React, { useCallback } from 'react';
import { Box, Text, useInput } from 'ink';
import { Header } from '../parts/Header.js';
import { Stats } from '../parts/Stats.js';
import { Footer } from '../parts/Footer.js';
import { Select } from '../common/Select.js';
import { LoadingIndicator } from '../common/LoadingIndicator.js';
import { useTerminalSize } from '../../hooks/useTerminalSize.js';
import type { BranchItem, Statistics } from '../../types.js';
import stringWidth from 'string-width';
import chalk from 'chalk';
import { isProtectedBranchName } from '../../../worktree.js';

const WIDTH_OVERRIDES: Record<string, number> = {
  '⬆': 1,
  '☁': 1,
};

const EMPTY_SELECTION = new Set<string>();

const measureDisplayWidth = (value: string): number => {
  let width = 0;
  for (const char of Array.from(value)) {
    const override = WIDTH_OVERRIDES[char];
    if (override !== undefined) {
      width += override;
      continue;
    }
    width += stringWidth(char);
  }
  return width;
};

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
  workingDirectory?: string;
  selectedBranches?: Set<string>;
  onToggleSelection?: (branchName: string) => void;
  onClearSelection?: () => void;
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
  workingDirectory,
  selectedBranches,
  onToggleSelection,
  onClearSelection,
}: BranchListScreenProps) {
  const { rows } = useTerminalSize();
  const selectionSet = selectedBranches ?? EMPTY_SELECTION;

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

  const formatLatestCommit = useCallback((timestamp?: number) => {
    if (!timestamp || Number.isNaN(timestamp)) {
      return '---';
    }

    const date = new Date(timestamp * 1000);
    const year = date.getFullYear();
    const month = String(date.getMonth() + 1).padStart(2, '0');
    const day = String(date.getDate()).padStart(2, '0');
    const hours = String(date.getHours()).padStart(2, '0');
    const minutes = String(date.getMinutes()).padStart(2, '0');

    return `${year}-${month}-${day} ${hours}:${minutes}`;
  }, []);

  const truncateToWidth = useCallback((value: string, maxWidth: number) => {
    if (maxWidth <= 0) {
      return '';
    }

    if (stringWidth(value) <= maxWidth) {
      return value;
    }

    const ellipsis = '…';
    const ellipsisWidth = stringWidth(ellipsis);
    if (ellipsisWidth >= maxWidth) {
      return ellipsis;
    }

    let currentWidth = 0;
    let result = '';

    for (const char of value) {
      const charWidth = stringWidth(char);
      if (currentWidth + charWidth + ellipsisWidth > maxWidth) {
        break;
      }
      result += char;
      currentWidth += charWidth;
    }

    return result + ellipsis;
  }, []);

  const renderBranchRow = useCallback(
    (item: BranchItem, isSelected: boolean, context: { columns: number }) => {
      const columns = Math.max(20, context.columns);
      const arrow = isSelected ? '>' : ' ';
      const timestampText = formatLatestCommit(item.latestCommitTimestamp);
      const timestampWidth = stringWidth(timestampText);
      const isManuallySelected = selectionSet.has(item.name);
      const shouldWarnSelection = Boolean(item.hasUnpushedCommits) || !item.mergedPR;
      let selectionMarker = isManuallySelected ? '*' : ' ';
      if (isManuallySelected && shouldWarnSelection) {
        selectionMarker = chalk.red('*');
      } else if (isManuallySelected) {
        selectionMarker = chalk.white('*');
      }

      const indicatorInfo = cleanupUI?.indicators?.[item.name];
      let indicatorIcon = indicatorInfo?.icon ?? '';
      if (indicatorIcon && indicatorInfo?.color && !isSelected) {
        switch (indicatorInfo.color) {
          case 'cyan':
            indicatorIcon = chalk.cyan(indicatorIcon);
            break;
          case 'green':
            indicatorIcon = chalk.green(indicatorIcon);
            break;
          case 'yellow':
            indicatorIcon = chalk.yellow(indicatorIcon);
            break;
          case 'red':
            indicatorIcon = chalk.red(indicatorIcon);
            break;
          default:
            break;
        }
      }
      const indicatorPrefix = indicatorIcon ? `${indicatorIcon} ` : '';
      const staticPrefix = `${arrow} ${selectionMarker} ${indicatorPrefix}`;
      const staticPrefixWidth = stringWidth(staticPrefix);

      const availableLeftWidth = Math.max(staticPrefixWidth, columns - timestampWidth - 1);
      const maxLabelWidth = Math.max(0, availableLeftWidth - staticPrefixWidth);
      const truncatedLabel = truncateToWidth(item.label, maxLabelWidth);
      const leftText = `${staticPrefix}${truncatedLabel}`;

      const leftMeasuredWidth = stringWidth(leftText);
      const leftDisplayWidth = measureDisplayWidth(leftText);
      const baseGapWidth = Math.max(1, columns - leftMeasuredWidth - timestampWidth);
      const displayGapWidth = Math.max(1, columns - leftDisplayWidth - timestampWidth);
      const cursorShift = Math.max(0, displayGapWidth - baseGapWidth);

      const gap = ' '.repeat(baseGapWidth);
      const cursorAdjust = cursorShift > 0 ? `\u001b[${cursorShift}C` : '';

      let line = `${leftText}${gap}${cursorAdjust}${timestampText}`;
      const paddingWidth = Math.max(0, columns - stringWidth(line));
      if (paddingWidth > 0) {
        line += ' '.repeat(paddingWidth);
      }

      const output = isSelected
        ? `[46m[30m${line}[0m`
        : line;
      return <Text>{output}</Text>;
    },
    [cleanupUI, formatLatestCommit, selectionSet, truncateToWidth]
  );

  const handleSpace = useCallback(
    (branch: BranchItem) => {
      if (!onToggleSelection) {
        return;
      }
      if (branch.type !== 'local') {
        return;
      }
      if (isProtectedBranchName(branch.name)) {
        return;
      }
      onToggleSelection(branch.name);
    },
    [onToggleSelection]
  );

  const handleEscape = useCallback(() => {
    onClearSelection?.();
  }, [onClearSelection]);

  const effectiveFooterMessage = cleanupUI?.footerMessage
    ?? (selectionSet.size > 0
      ? { text: `選択中: ${selectionSet.size}個のブランチ` }
      : null);

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header
        title="Claude Worktree - Branch Selection"
        titleColor="cyan"
        version={version}
        {...(workingDirectory !== undefined && { workingDirectory })}
      />

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
            renderIndicator={() => null}
            renderItem={renderBranchRow}
            onSpace={handleSpace}
            onEscape={handleEscape}
          />
        )}
      </Box>

      {effectiveFooterMessage && (
        <Box marginBottom={1}>
          {effectiveFooterMessage.color ? (
            <Text color={effectiveFooterMessage.color}>
              {effectiveFooterMessage.text}
            </Text>
          ) : (
            <Text>{effectiveFooterMessage.text}</Text>
          )}
        </Box>
      )}

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
