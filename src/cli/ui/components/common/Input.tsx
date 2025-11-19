import React from "react";
import { Box, Text, useInput } from "ink";
import TextInput from "ink-text-input";

export interface InputProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit: (value: string) => void;
  placeholder?: string;
  label?: string;
  mask?: string;
  /**
   * Block specific key bindings to prevent parent handlers from processing them
   * Useful for blocking shortcuts like 'c', 'r', 'm' while typing
   */
  blockKeys?: string[];
}

/**
 * Input component - wrapper around ink-text-input with optional label
 */
export function Input({
  value,
  onChange,
  onSubmit,
  placeholder,
  label,
  mask,
  blockKeys,
}: InputProps) {
  // Block specific keys from being processed by parent useInput handlers
  // This prevents shortcuts (c/r/m) from triggering while typing in the input
  useInput((input) => {
    if (blockKeys && blockKeys.includes(input)) {
      // Consume the key - don't let it propagate to parent handlers
      return;
    }
  });

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
