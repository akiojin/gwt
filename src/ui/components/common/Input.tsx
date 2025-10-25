import React from 'react';
import { Box, Text } from 'ink';
import TextInput from 'ink-text-input';

export interface InputProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit: (value: string) => void;
  placeholder?: string;
  label?: string;
  mask?: string;
}

/**
 * Input component - wrapper around ink-text-input with optional label
 */
export function Input({ value, onChange, onSubmit, placeholder, label, mask }: InputProps) {
  return (
    <Box flexDirection="column">
      {label && (
        <Box marginBottom={0}>
          <Text>{label}</Text>
        </Box>
      )}
      <Box>
        <TextInput
          value={value}
          onChange={onChange}
          onSubmit={onSubmit}
          {...(placeholder !== undefined && { placeholder })}
          {...(mask !== undefined && { mask })}
        />
      </Box>
    </Box>
  );
}
