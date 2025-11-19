import React from "react";
import { Box, Text, useInput } from "ink";
import { Header } from "../parts/Header.js";
import { Footer } from "../parts/Footer.js";
import { Select } from "../common/Select.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";

export interface SessionItem {
  label: string;
  value: string;
}

export interface SessionSelectorScreenProps {
  sessions: string[];
  onBack: () => void;
  onSelect: (session: string) => void;
  version?: string | null;
}

/**
 * SessionSelectorScreen - Screen for selecting a session
 * Layout: Header + Stats + Session List + Footer
 */
export function SessionSelectorScreen({
  sessions,
  onBack,
  onSelect,
  version,
}: SessionSelectorScreenProps) {
  const { rows } = useTerminalSize();

  // Handle keyboard input
  // Note: Select component handles Enter and arrow keys
  useInput((input, key) => {
    if (key.escape) {
      onBack();
    }
  });

  // Format sessions for Select component
  const sessionItems: SessionItem[] = sessions.map((session) => ({
    label: session,
    value: session,
  }));

  // Handle session selection
  const handleSelect = (item: SessionItem) => {
    onSelect(item.value);
  };

  // Calculate available space for session list
  const headerLines = 2;
  const statsLines = 1;
  const emptyLine = 1;
  const footerLines = 1;
  const fixedLines = headerLines + statsLines + emptyLine + footerLines;
  const contentHeight = rows - fixedLines;
  const limit = Math.max(5, contentHeight);

  // Footer actions
  const footerActions = [
    { key: "enter", description: "Select" },
    { key: "esc", description: "Back" },
  ];

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header title="Session Selection" titleColor="cyan" version={version} />

      {/* Stats */}
      <Box marginTop={1}>
        <Box flexDirection="row">
          <Box marginRight={2}>
            <Text>
              Total: <Text bold>{sessions.length}</Text>
            </Text>
          </Box>
        </Box>
      </Box>

      {/* Empty line */}
      <Box height={1} />

      {/* Content */}
      <Box flexDirection="column" flexGrow={1}>
        {sessions.length === 0 ? (
          <Box>
            <Text dimColor>No sessions found</Text>
          </Box>
        ) : (
          <Select items={sessionItems} onSelect={handleSelect} limit={limit} />
        )}
      </Box>

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
