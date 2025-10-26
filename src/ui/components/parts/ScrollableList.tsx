import React, { type ReactNode } from 'react';
import { Box } from 'ink';

export interface ScrollableListProps {
  children: ReactNode;
  maxHeight?: number;
}

/**
 * ScrollableList component - container for scrollable content
 * Note: Actual scrolling is handled by ink-select-input's limit prop
 * This component provides a consistent container for list content
 */
export function ScrollableList({ children, maxHeight }: ScrollableListProps) {
  return (
    <Box
      flexDirection="column"
      {...(maxHeight !== undefined && { height: maxHeight })}
      overflow="hidden"
    >
      {children}
    </Box>
  );
}
