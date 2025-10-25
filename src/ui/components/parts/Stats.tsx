import React from 'react';
import { Box, Text } from 'ink';
import type { Statistics } from '../../types.js';

export interface StatsProps {
  stats: Statistics;
  separator?: string;
  lastUpdated?: Date | null;
}

/**
 * Format relative time (e.g., "5s ago", "2m ago", "1h ago")
 */
function formatRelativeTime(date: Date): string {
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffSec = Math.floor(diffMs / 1000);

  if (diffSec < 60) {
    return `${diffSec}s ago`;
  }

  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) {
    return `${diffMin}m ago`;
  }

  const diffHour = Math.floor(diffMin / 60);
  return `${diffHour}h ago`;
}

/**
 * Stats component - displays statistics in one line
 */
export function Stats({ stats, separator = '  ', lastUpdated = null }: StatsProps) {
  const items = [
    { label: 'Local', value: stats.localCount, color: 'cyan' },
    { label: 'Remote', value: stats.remoteCount, color: 'green' },
    { label: 'Worktrees', value: stats.worktreeCount, color: 'yellow' },
    { label: 'Changes', value: stats.changesCount, color: 'magenta' },
  ];

  return (
    <Box>
      {items.map((item, index) => (
        <Box key={item.label}>
          <Text dimColor>{item.label}: </Text>
          <Text bold color={item.color}>
            {item.value}
          </Text>
          <Text dimColor>{separator}</Text>
        </Box>
      ))}
      {lastUpdated && (
        <Box>
          <Text dimColor>Updated: </Text>
          <Text color="gray">{formatRelativeTime(lastUpdated)}</Text>
        </Box>
      )}
    </Box>
  );
}
