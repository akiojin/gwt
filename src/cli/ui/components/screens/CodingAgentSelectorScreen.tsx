import React, { useState, useEffect } from "react";
import { Box, Text } from "ink";
import { Header } from "../parts/Header.js";
import { Footer } from "../parts/Footer.js";
import { Select } from "../common/Select.js";
import { useAppInput } from "../../hooks/useAppInput.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";
import { getAllCodingAgents } from "../../../../config/tools.js";
import type { CodingAgentConfig } from "../../../../types/tools.js";
import type { CodingAgentId } from "../../types.js";

/**
 * Renderable item for the coding agent selector list.
 */
export interface CodingAgentItem {
  label: string;
  value: CodingAgentId;
  description: string;
}

/**
 * Props for `CodingAgentSelectorScreen`.
 */
export interface CodingAgentSelectorScreenProps {
  onBack: () => void;
  onSelect: (agentId: CodingAgentId) => void;
  version?: string | null;
  initialAgentId?: CodingAgentId | null;
}

/**
 * CodingAgentSelectorScreen - Screen for selecting coding agent (Claude Code, Codex CLI, or custom agents)
 * Layout: Header + Agent Selection + Footer
 *
 * This screen dynamically loads available agents from the configuration (builtin + custom).
 */
export function CodingAgentSelectorScreen({
  onBack,
  onSelect,
  version,
  initialAgentId,
}: CodingAgentSelectorScreenProps) {
  const { rows } = useTerminalSize();
  const [agentItems, setAgentItems] = useState<CodingAgentItem[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [selectedIndex, setSelectedIndex] = useState<number>(0);

  // Load agents from getAllCodingAgents()
  useEffect(() => {
    const loadAgents = async () => {
      try {
        const agents = await getAllCodingAgents();

        // Convert CodingAgentConfig[] to CodingAgentItem[]
        const items: CodingAgentItem[] = agents.map(
          (agent: CodingAgentConfig) => {
            // Generate description based on whether it's builtin or custom
            const description = agent.isBuiltin
              ? `Official ${agent.displayName} agent`
              : `Custom coding agent`;

            // Add icon to label if present
            const label = agent.icon
              ? `${agent.icon} ${agent.displayName}`
              : agent.displayName;

            return {
              label,
              value: agent.id,
              description,
            };
          },
        );

        setAgentItems(items);

        // Decide initial cursor position based on last used agent
        const idx =
          initialAgentId && items.length > 0
            ? items.findIndex((item) => item.value === initialAgentId)
            : 0;
        setSelectedIndex(idx >= 0 ? idx : 0);
      } catch (error) {
        // If loading fails, show error in console but don't crash
        console.error("Failed to load coding agents:", error);
        // Fall back to empty array
        setAgentItems([]);
      } finally {
        setIsLoading(false);
      }
    };

    loadAgents();
  }, [initialAgentId]);

  // Update selection when props or items change
  useEffect(() => {
    if (isLoading || agentItems.length === 0) return;
    const idx =
      initialAgentId && agentItems.length > 0
        ? agentItems.findIndex((item) => item.value === initialAgentId)
        : 0;
    setSelectedIndex(idx >= 0 ? idx : 0);
  }, [initialAgentId, agentItems, isLoading]);

  // Handle keyboard input
  // Note: Select component handles Enter and arrow keys
  useAppInput((input, key) => {
    if (key.escape) {
      onBack();
    }
  });

  // Handle agent selection
  const handleSelect = (item: CodingAgentItem) => {
    onSelect(item.value);
  };

  // Footer actions
  const footerActions = [
    { key: "enter", description: "Select" },
    { key: "esc", description: "Back" },
  ];

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header
        title="Coding Agent Selection"
        titleColor="blue"
        version={version}
      />

      {/* Content */}
      <Box flexDirection="column" flexGrow={1} marginTop={1}>
        <Box marginBottom={1}>
          <Text>Select coding agent to use:</Text>
        </Box>
        {isLoading ? (
          <Text>Loading coding agents...</Text>
        ) : agentItems.length === 0 ? (
          <Text color="yellow">
            No coding agents available. Please check your configuration.
          </Text>
        ) : (
          <Select
            items={agentItems}
            onSelect={handleSelect}
            selectedIndex={selectedIndex}
            onSelectedIndexChange={setSelectedIndex}
          />
        )}
      </Box>

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
