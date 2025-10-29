import React from 'react';
import { Box, Text, useInput } from 'ink';
import { Header } from '../parts/Header.js';
import { Footer } from '../parts/Footer.js';
import { Select } from '../common/Select.js';
import { useTerminalSize } from '../../hooks/useTerminalSize.js';

export type AITool = 'claude-code' | 'codex-cli';

export interface AIToolItem {
  label: string;
  value: AITool;
  description: string;
}

export interface AIToolSelectorScreenProps {
  onBack: () => void;
  onSelect: (tool: AITool) => void;
}

/**
 * AIToolSelectorScreen - Screen for selecting AI tool (Claude Code or Codex CLI)
 * Layout: Header + Tool Selection + Footer
 */
export function AIToolSelectorScreen({ onBack, onSelect }: AIToolSelectorScreenProps) {
  const { rows } = useTerminalSize();

  // Handle keyboard input
  // Note: Select component handles Enter and arrow keys
  useInput((input, key) => {
    if (key.escape) {
      onBack();
    }
  });

  // AI tool options
  const toolItems: AIToolItem[] = [
    {
      label: 'Claude Code',
      value: 'claude-code',
      description: 'Official Claude CLI tool',
    },
    {
      label: 'Codex CLI',
      value: 'codex-cli',
      description: 'Alternative AI coding assistant',
    },
  ];

  // Handle tool selection
  const handleSelect = (item: AIToolItem) => {
    onSelect(item.value);
  };

  // Footer actions
  const footerActions = [
    { key: 'enter', description: 'Select' },
    { key: 'esc', description: 'Back' },
  ];

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header title="AI Tool Selection" titleColor="blue" />

      {/* Content */}
      <Box flexDirection="column" flexGrow={1} marginTop={1}>
        <Box marginBottom={1}>
          <Text>Select AI tool to use:</Text>
        </Box>
        <Select items={toolItems} onSelect={handleSelect} />
      </Box>

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
