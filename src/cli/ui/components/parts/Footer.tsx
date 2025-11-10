import React from "react";
import { Box, Text } from "ink";

export interface FooterAction {
  key: string;
  description: string;
}

export interface FooterProps {
  actions: FooterAction[];
  separator?: string;
}

/**
 * Footer component - displays keyboard actions
 * Optimized with React.memo to prevent unnecessary re-renders
 */
export const Footer = React.memo(function Footer({
  actions,
  separator = "  ",
}: FooterProps) {
  if (actions.length === 0) {
    return null;
  }

  return (
    <Box>
      {actions.map((action, index) => (
        <Box key={`${action.key}-${index}`}>
          <Text dimColor>[</Text>
          <Text bold color="cyan">
            {action.key}
          </Text>
          <Text dimColor>]</Text>
          <Text> {action.description}</Text>
          {index < actions.length - 1 && <Text dimColor>{separator}</Text>}
        </Box>
      ))}
    </Box>
  );
});
