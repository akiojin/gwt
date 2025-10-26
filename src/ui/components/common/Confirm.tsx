import React from 'react';
import { Box, Text } from 'ink';
import { Select, type SelectItem } from './Select.js';

export interface ConfirmProps {
  message: string;
  onConfirm: (confirmed: boolean) => void;
  yesLabel?: string;
  noLabel?: string;
  defaultNo?: boolean;
}

/**
 * Confirm component - Yes/No confirmation dialog
 */
export function Confirm({
  message,
  onConfirm,
  yesLabel = 'Yes',
  noLabel = 'No',
  defaultNo = false,
}: ConfirmProps) {
  const items: SelectItem[] = [
    { label: yesLabel, value: 'yes' },
    { label: noLabel, value: 'no' },
  ];

  const handleSelect = (item: SelectItem) => {
    onConfirm(item.value === 'yes');
  };

  return (
    <Box flexDirection="column">
      <Box marginBottom={1}>
        <Text>{message}</Text>
      </Box>
      <Select items={items} onSelect={handleSelect} initialIndex={defaultNo ? 1 : 0} />
    </Box>
  );
}
