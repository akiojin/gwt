import React from 'react';
import { Box, Text, useInput } from 'ink';
import { Header } from '../parts/Header.js';
import { Footer } from '../parts/Footer.js';
import { Select } from '../common/Select.js';
import { useTerminalSize } from '../../hooks/useTerminalSize.js';
import type { CleanupTarget } from '../../types.js';

export interface PRItem {
  label: string;
  value: string;
  target: CleanupTarget;
}

export interface PRCleanupScreenProps {
  targets: CleanupTarget[];
  loading: boolean;
  error: Error | null;
  statusMessage?: string | null;
  onBack: () => void;
  onRefresh: () => void;
  onCleanup: (target: CleanupTarget) => void;
  version?: string | null;
}

/**
 * PRCleanupScreen - Screen for cleaning up merged pull requests
 * Layout: Header + Stats + PR List + Footer
 */
export function PRCleanupScreen({
  targets,
  loading,
  error,
  statusMessage,
  onBack,
  onRefresh,
  onCleanup,
  version,
}: PRCleanupScreenProps) {
  const { rows } = useTerminalSize();

  // Handle keyboard input
  // Note: Select component handles Enter and arrow keys
  useInput((input, key) => {
    if (key.escape) {
      onBack();
    } else if (input === 'r') {
      onRefresh();
    }
  });

  // Format pull requests for Select component
  const prItems: PRItem[] = targets.map((target) => {
    const pr = target.pullRequest;
    const flags: string[] = [];
    if (target.cleanupType === 'worktree-and-branch') {
      flags.push('worktree');
    } else {
      flags.push('branch');
    }
    if (target.reasons?.includes('merged-pr')) {
      flags.push('merged');
    }
    if (target.reasons?.includes('no-diff-with-base')) {
      flags.push('base');
    }
    if (target.hasUncommittedChanges) {
      flags.push('changes');
    }
    if (target.hasUnpushedCommits) {
      flags.push('unpushed');
    }
    if (target.isAccessible === false) {
      flags.push('inaccessible');
    }

    const flagText = flags.length > 0 ? ` [${flags.join(', ')}]` : '';

    const label = pr
      ? `${target.branch} - #${pr.number} ${pr.title}${flagText}`
      : `${target.branch}${flagText}`;

    return {
      label,
      value: target.branch,
      target,
    };
  });

  // Calculate available space for PR list
  const headerLines = 2;
  const statsLines = 1;
  const emptyLine = 1;
  const footerLines = 1;
  const fixedLines = headerLines + statsLines + emptyLine + footerLines;
  const contentHeight = rows - fixedLines;
  const limit = Math.max(5, contentHeight);

  // Footer actions
  const footerActions = [
    { key: 'enter', description: 'Cleanup' },
    { key: 'r', description: 'Refresh' },
    { key: 'esc', description: 'Back' },
  ];

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header title="Branch Cleanup" titleColor="yellow" version={version} />

      {/* Stats */}
      <Box marginTop={1}>
        <Box flexDirection="row">
          <Box marginRight={2}>
            <Text>
              Total: <Text bold>{targets.length}</Text>
            </Text>
          </Box>
          {loading && (
            <Box marginRight={2}>
              <Text color="cyan">Loading...</Text>
            </Box>
          )}
        </Box>
      </Box>

      {error && (
        <Box marginTop={1}>
          <Text color="red">
            Error: <Text bold>{error.message}</Text>
          </Text>
        </Box>
      )}

      {statusMessage && (
        <Box marginTop={1}>
          <Text color="green">{statusMessage}</Text>
        </Box>
      )}

      {/* Empty line */}
      <Box height={1} />

      {/* Content */}
      <Box flexDirection="column" flexGrow={1}>
        {loading ? (
          <Box>
            <Text dimColor>Loading cleanup targets...</Text>
          </Box>
        ) : targets.length === 0 ? (
          <Box>
            <Text dimColor>No cleanup targets found</Text>
          </Box>
        ) : (
          <Select<PRItem>
            items={prItems}
            onSelect={(item) => onCleanup(item.target)}
            limit={limit}
          />
        )}
      </Box>

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
