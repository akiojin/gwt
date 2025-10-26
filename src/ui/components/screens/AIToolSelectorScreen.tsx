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
  items?: AIToolItem[];
  loading?: boolean;
  skipPermissions?: boolean;
  onToggleSkip?: () => void;
  infoMessage?: string | null;
}

/**
 * AIToolSelectorScreen - Screen for selecting AI tool (Claude Code or Codex CLI)
 * Layout: Header + Tool Selection + Footer
 */
export function AIToolSelectorScreen({
  onBack,
  onSelect,
  items,
  loading = false,
  skipPermissions = false,
  onToggleSkip,
  infoMessage = null,
}: AIToolSelectorScreenProps) {
  const { rows } = useTerminalSize();

  // Handle keyboard input
  useInput((input, key) => {
    if (input === 'q') {
      onBack();
    } else if (input === 's' && onToggleSkip) {
      onToggleSkip();
    }
  });

  const toolItems: AIToolItem[] =
    items && items.length > 0
      ? items
      : [
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
    { key: 'q', description: 'Back' },
    ...(onToggleSkip
      ? [{ key: 's', description: skipPermissions ? 'Skip perms: on' : 'Skip perms: off' }]
      : []),
  ];

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header title="AI Tool Selection" titleColor="blue" />

      {/* Content */}
      <Box flexDirection="column" flexGrow={1} marginTop={1}>
        <Box marginBottom={1} flexDirection="column">
          <Text>Select AI tool to use:</Text>
          {loading && (
            <Text color="yellow">
              Checking tool availability...
            </Text>
          )}
          {!loading && infoMessage && (
            <Text color="yellow">{infoMessage}</Text>
          )}
          {!loading && onToggleSkip && (
            <Text color={skipPermissions ? 'yellow' : 'gray'}>
              Permission check: {skipPermissions ? 'Disabled (dangerous)' : 'Enabled'} — press "s" to toggle
            </Text>
          )}
        </Box>
        {loading ? (
          <Text dimColor>Loading tools…</Text>
        ) : toolItems.length === 0 ? (
          <Text color="red">No AI tools available in PATH.</Text>
        ) : (
          <Select items={toolItems} onSelect={handleSelect} />
        )}
      </Box>

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
