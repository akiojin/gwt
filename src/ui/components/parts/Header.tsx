import React from 'react';
import { Box, Text } from 'ink';

export interface HeaderProps {
  title: string;
  titleColor?: string;
  dividerChar?: string;
  showDivider?: boolean;
  width?: number;
}

/**
 * Header component - displays title and optional divider
 * Optimized with React.memo to prevent unnecessary re-renders
 */
export const Header = React.memo(function Header({
  title,
  titleColor = 'cyan',
  dividerChar = '─',
  showDivider = true,
  width = 80,
}: HeaderProps) {
  const divider = dividerChar.repeat(width);

  return (
    <Box flexDirection="column">
      <Box>
        <Text bold color={titleColor}>
          {title}
        </Text>
      </Box>
      {showDivider && (
        <Box>
          <Text dimColor>{divider}</Text>
        </Box>
      )}
    </Box>
  );
});
