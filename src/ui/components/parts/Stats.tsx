import React from 'react';
import { Box, Text } from 'ink';
import type { Statistics } from '../../types.js';

export interface StatsProps {
  stats: Statistics;
  separator?: string;
}

/**
 * Stats component - displays statistics in one line
 */
export function Stats({ stats, separator = '  ' }: StatsProps) {
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
          {index < items.length - 1 && <Text dimColor>{separator}</Text>}
        </Box>
      ))}
    </Box>
  );
}
