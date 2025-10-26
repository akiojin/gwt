import React from 'react';
import { Box, Text, useInput } from 'ink';
import { Header } from '../parts/Header.js';
import { Footer } from '../parts/Footer.js';
import { Select } from '../common/Select.js';
import { useTerminalSize } from '../../hooks/useTerminalSize.js';
import type { MergedPullRequest } from '../../types.js';

export interface PRItem extends MergedPullRequest {
  label: string;
  value: string;
}

export interface PRCleanupScreenProps {
  pullRequests: MergedPullRequest[];
  onBack: () => void;
  onCleanup: (pr: MergedPullRequest) => void;
}

/**
 * PRCleanupScreen - Screen for cleaning up merged pull requests
 * Layout: Header + Stats + PR List + Footer
 */
export function PRCleanupScreen({ pullRequests, onBack, onCleanup }: PRCleanupScreenProps) {
  const { rows } = useTerminalSize();

  // Handle keyboard input
  useInput((input, key) => {
    // Skip Enter and arrow keys - let SelectInput handle them
    if (key.return || key.upArrow || key.downArrow) {
      return;
    }

    if (input === 'q') {
      onBack();
    }
  });

  // Format pull requests for Select component
  const prItems: PRItem[] = pullRequests.map((pr) => ({
    ...pr,
    label: `#${pr.number} ${pr.title} (${pr.branch}) by ${pr.author}`,
    value: String(pr.number),
  }));

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
    { key: 'q', description: 'Back' },
  ];

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header title="PR Cleanup" titleColor="yellow" />

      {/* Stats */}
      <Box marginTop={1}>
        <Box flexDirection="row">
          <Box marginRight={2}>
            <Text>
              Total: <Text bold>{pullRequests.length}</Text>
            </Text>
          </Box>
        </Box>
      </Box>

      {/* Empty line */}
      <Box height={1} />

      {/* Content */}
      <Box flexDirection="column" flexGrow={1}>
        {pullRequests.length === 0 ? (
          <Box>
            <Text dimColor>No merged pull requests found</Text>
          </Box>
        ) : (
          <Select
            items={prItems}
            onSelect={(item) => onCleanup(item as PRItem)}
            limit={limit}
          />
        )}
      </Box>

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
