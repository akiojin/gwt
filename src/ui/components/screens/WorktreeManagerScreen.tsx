import React from 'react';
import { Box, Text } from 'ink';
import { Header } from '../parts/Header.js';
import { Footer } from '../parts/Footer.js';
import { Select } from '../common/Select.js';
import { useTerminalSize } from '../../hooks/useTerminalSize.js';

export interface WorktreeItem {
  branch: string;
  path: string;
  isAccessible: boolean;
  label?: string;
  value?: string;
}

export interface WorktreeManagerScreenProps {
  worktrees: WorktreeItem[];
  onBack: () => void;
  onSelect: (worktree: WorktreeItem) => void;
}

/**
 * WorktreeManagerScreen - Screen for managing worktrees
 * Layout: Header + Stats + Worktree List + Footer
 */
export function WorktreeManagerScreen({
  worktrees,
  onBack,
  onSelect,
}: WorktreeManagerScreenProps) {
  const { rows } = useTerminalSize();

  // Calculate accessible and inaccessible counts
  const accessibleCount = worktrees.filter((w) => w.isAccessible).length;
  const inaccessibleCount = worktrees.filter((w) => !w.isAccessible).length;

  // Format worktrees for Select component
  const worktreeItems = worktrees.map((wt) => ({
    ...wt,
    label: wt.isAccessible
      ? `${wt.branch} (${wt.path})`
      : `${wt.branch} (${wt.path}) [Inaccessible]`,
    value: wt.branch,
  }));

  // Calculate available space for worktree list
  const headerLines = 2;
  const statsLines = 1;
  const emptyLine = 1;
  const footerLines = 1;
  const fixedLines = headerLines + statsLines + emptyLine + footerLines;
  const contentHeight = rows - fixedLines;
  const limit = Math.max(5, contentHeight);

  // Footer actions
  const footerActions = [
    { key: 'enter', description: 'Select' },
    { key: 'q', description: 'Back' },
  ];

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header title="Worktree Manager" titleColor="magenta" />

      {/* Stats */}
      <Box marginTop={1}>
        <Box flexDirection="row">
          <Box marginRight={2}>
            <Text>
              Total: <Text bold>{worktrees.length}</Text>
            </Text>
          </Box>
          <Box marginRight={2}>
            <Text color="green">
              Accessible: <Text bold>{accessibleCount}</Text>
            </Text>
          </Box>
          {inaccessibleCount > 0 && (
            <Box>
              <Text color="red">
                Inaccessible: <Text bold>{inaccessibleCount}</Text>
              </Text>
            </Box>
          )}
        </Box>
      </Box>

      {/* Empty line */}
      <Box height={1} />

      {/* Content */}
      <Box flexDirection="column" flexGrow={1}>
        {worktrees.length === 0 ? (
          <Box>
            <Text dimColor>No worktrees found</Text>
          </Box>
        ) : (
          <Select items={worktreeItems} onSelect={onSelect} limit={limit} />
        )}
      </Box>

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
