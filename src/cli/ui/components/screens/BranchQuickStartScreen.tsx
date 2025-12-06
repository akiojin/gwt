import React from "react";
import { Box, Text, useInput } from "ink";
import { Header } from "../parts/Header.js";
import { Footer } from "../parts/Footer.js";
import { Select, type SelectItem } from "../common/Select.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";

export type QuickStartAction = "reuse-continue" | "reuse-new" | "manual";

export interface BranchQuickStartOption {
  toolLabel: string;
  model?: string | null;
  sessionId?: string | null;
  inferenceLevel?: string | null;
}

type QuickStartItem = SelectItem & {
  description: string;
  disabled?: boolean;
};

export interface BranchQuickStartScreenProps {
  previousOption: BranchQuickStartOption | null;
  loading?: boolean;
  onBack: () => void;
  onSelect: (action: QuickStartAction) => void;
  version?: string | null;
  branchName: string;
}

export function BranchQuickStartScreen({
  previousOption,
  loading = false,
  onBack,
  onSelect,
  version,
  branchName,
}: BranchQuickStartScreenProps) {
  const { rows } = useTerminalSize();

  const items: QuickStartItem[] = [
    {
      label: "Resume with previous settings",
      value: "reuse-continue",
      description: previousOption
        ? `${previousOption.toolLabel} / ${previousOption.model ?? "default"} / Reasoning: ${previousOption.inferenceLevel ?? "default"} / ${previousOption.sessionId ? `ID: ${previousOption.sessionId}` : "No ID"}`
        : "No previous settings (disabled)",
      disabled: !previousOption,
    },
    {
      label: "Start new with previous settings",
      value: "reuse-new",
      description: previousOption
        ? `${previousOption.toolLabel} / ${previousOption.model ?? "default"}`
        : "No previous settings (disabled)",
      disabled: !previousOption,
    },
    {
      label: "Choose manually",
      value: "manual",
      description: "Pick tool and model manually",
    },
  ];

  useInput((_, key) => {
    if (key.escape) {
      onBack();
    }
  });

  return (
    <Box flexDirection="column" height={rows}>
      <Header
        title="Quick Start"
        titleColor="cyan"
        version={version}
      />

      <Box flexDirection="column" flexGrow={1} marginTop={1}>
        <Box marginBottom={1}>
          <Text>
            {loading
              ? "Loading previous settings..."
              : "Resume with previous settings, start new, or choose manually."}
          </Text>
          <Text color="gray">{`Branch: ${branchName}`}</Text>
        </Box>
        <Select
          items={items}
          onSelect={(item: QuickStartItem) => {
            if (item.disabled) return;
            onSelect(item.value as QuickStartAction);
          }}
          renderItem={(item: QuickStartItem, isSelected) => (
            <Box flexDirection="column">
              <Text color={isSelected ? "cyan" : "white"}>
                {item.label}
                {item.disabled ? " (disabled)" : ""}
              </Text>
              <Text color="gray">{item.description}</Text>
            </Box>
          )}
        />
      </Box>

      <Footer
        actions={[
          { key: "enter", description: "Select" },
          { key: "esc", description: "Back" },
        ]}
      />
    </Box>
  );
}
