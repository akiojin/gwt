import React from 'react';
import { Box, Text, useInput } from 'ink';
import { Header } from '../parts/Header.js';
import { Footer } from '../parts/Footer.js';
import { Select } from '../common/Select.js';
import { useTerminalSize } from '../../hooks/useTerminalSize.js';

export interface SessionListEntry {
  id: string;
  branchName: string | null;
  worktreePath: string | null;
  formattedTimestamp: string;
}

export interface SessionSelectorScreenProps {
  sessions: SessionListEntry[];
  onBack: () => void;
  onSelect: (session: SessionListEntry) => void;
  infoMessage?: string | null;
}

/**
 * SessionSelectorScreen - Screen for selecting a session
 * Layout: Header + Stats + Session List + Footer
 */
export function SessionSelectorScreen({
  sessions,
  onBack,
  onSelect,
  infoMessage = null,
}: SessionSelectorScreenProps) {
  const { rows } = useTerminalSize();

  // Handle keyboard input
  useInput((input, key) => {
    if (input === 'q') {
      onBack();
    }
  });

  // Format sessions for Select component
  const sessionItems = sessions.map((session) => ({
    label: `${session.branchName ?? 'Unknown branch'} — ${session.formattedTimestamp}`,
    value: session.id,
    meta: session,
  }));

  // Handle session selection
  const handleSelect = (item: { value: string }) => {
    const selected = sessions.find((session) => session.id === item.value);
    if (selected) {
      onSelect(selected);
    }
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
    { key: 'enter', description: 'Select' },
    { key: 'q', description: 'Back' },
  ];

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header title="Session Selection" titleColor="cyan" />

      {/* Stats */}
      <Box marginTop={1}>
        <Box flexDirection="row">
          <Box marginRight={2}>
            <Text>
              Total: <Text bold>{sessions.length}</Text>
            </Text>
          </Box>
          {infoMessage && (
            <Box>
              <Text color="yellow">{infoMessage}</Text>
            </Box>
          )}
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
