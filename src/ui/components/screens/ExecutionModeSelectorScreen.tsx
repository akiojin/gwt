import React from 'react';
import { Box, Text, useInput } from 'ink';
import { Header } from '../parts/Header.js';
import { Footer } from '../parts/Footer.js';
import { Select } from '../common/Select.js';
import { useTerminalSize } from '../../hooks/useTerminalSize.js';

export type ExecutionMode = 'normal' | 'continue' | 'resume';

export interface ExecutionModeItem {
  label: string;
  value: ExecutionMode;
  description: string;
}

export interface ExecutionModeSelectorScreenProps {
  onBack: () => void;
  onSelect: (mode: ExecutionMode) => void;
}

/**
 * ExecutionModeSelectorScreen - Screen for selecting execution mode
 * Layout: Header + Mode Selection + Footer
 */
export function ExecutionModeSelectorScreen({
  onBack,
  onSelect,
}: ExecutionModeSelectorScreenProps) {
  const { rows } = useTerminalSize();

  // Handle keyboard input
  useInput((input, key) => {
    if (input === 'q') {
      onBack();
    }
  });

  // Execution mode options
  const modeItems: ExecutionModeItem[] = [
    {
      label: 'Normal',
      value: 'normal',
      description: 'Start fresh session',
    },
    {
      label: 'Continue',
      value: 'continue',
      description: 'Continue from last session',
    },
    {
      label: 'Resume',
      value: 'resume',
      description: 'Resume specific session',
    },
  ];

  // Handle mode selection
  const handleSelect = (item: ExecutionModeItem) => {
    onSelect(item.value);
  };

  // Footer actions
  const footerActions = [
    { key: 'enter', description: 'Select' },
    { key: 'q', description: 'Back' },
  ];

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header title="Execution Mode" titleColor="magenta" />

      {/* Content */}
      <Box flexDirection="column" flexGrow={1} marginTop={1}>
        <Box marginBottom={1}>
          <Text>Select execution mode:</Text>
        </Box>
        <Select items={modeItems} onSelect={handleSelect} />
      </Box>

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
