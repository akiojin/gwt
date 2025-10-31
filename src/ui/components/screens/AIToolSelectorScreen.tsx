import React, { useState, useEffect } from 'react';
import { Box, Text, useInput } from 'ink';
import { Header } from '../parts/Header.js';
import { Footer } from '../parts/Footer.js';
import { Select } from '../common/Select.js';
import { useTerminalSize } from '../../hooks/useTerminalSize.js';
import { getAllTools } from '../../../config/tools.js';
import type { AIToolConfig } from '../../../types/tools.js';

export type AITool = string;

export interface AIToolItem {
  label: string;
  value: AITool;
  description: string;
}

export interface AIToolSelectorScreenProps {
  onBack: () => void;
  onSelect: (tool: AITool) => void;
  version?: string | null;
}

/**
 * AIToolSelectorScreen - Screen for selecting AI tool (Claude Code, Codex CLI, or custom tools)
 * Layout: Header + Tool Selection + Footer
 *
 * This screen dynamically loads available tools from the configuration (builtin + custom).
 */
export function AIToolSelectorScreen({ onBack, onSelect, version }: AIToolSelectorScreenProps) {
  const { rows } = useTerminalSize();
  const [toolItems, setToolItems] = useState<AIToolItem[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  // Load tools from getAllTools()
  useEffect(() => {
    const loadTools = async () => {
      try {
        const tools = await getAllTools();

        // Convert AIToolConfig[] to AIToolItem[]
        const items: AIToolItem[] = tools.map((tool: AIToolConfig) => {
          // Generate description based on whether it's builtin or custom
          const description = tool.isBuiltin
            ? `Official ${tool.displayName} tool`
            : `Custom AI tool`;

          // Add icon to label if present
          const label = tool.icon
            ? `${tool.icon} ${tool.displayName}`
            : tool.displayName;

          return {
            label,
            value: tool.id,
            description,
          };
        });

        setToolItems(items);
      } catch (error) {
        // If loading fails, show error in console but don't crash
        console.error('Failed to load tools:', error);
        // Fall back to empty array
        setToolItems([]);
      } finally {
        setIsLoading(false);
      }
    };

    loadTools();
  }, []);

  // Handle keyboard input
  // Note: Select component handles Enter and arrow keys
  useInput((input, key) => {
    if (key.escape) {
      onBack();
    }
  });

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
      <Header title="AI Tool Selection" titleColor="blue" version={version} />

      {/* Content */}
      <Box flexDirection="column" flexGrow={1} marginTop={1}>
        <Box marginBottom={1}>
          <Text>Select AI tool to use:</Text>
        </Box>
        {isLoading ? (
          <Text>Loading tools...</Text>
        ) : toolItems.length === 0 ? (
          <Text color="yellow">No tools available. Please check your configuration.</Text>
        ) : (
          <Select items={toolItems} onSelect={handleSelect} />
        )}
      </Box>

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
