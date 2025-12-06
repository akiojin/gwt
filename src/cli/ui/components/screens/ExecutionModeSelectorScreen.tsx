import React, { useState } from "react";
import { Box, Text, useInput } from "ink";
import { Header } from "../parts/Header.js";
import { Footer } from "../parts/Footer.js";
import { Select } from "../common/Select.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";

export type ExecutionMode = "normal" | "continue" | "resume";

export interface ExecutionModeItem {
  label: string;
  value: ExecutionMode;
  description: string;
}

export interface SkipPermissionsItem {
  label: string;
  value: string; // "yes" or "no"
  description: string;
}

export interface ExecutionModeResult {
  mode: ExecutionMode;
  skipPermissions: boolean;
}

export interface ExecutionModeSelectorScreenProps {
  onBack: () => void;
  onSelect: (result: ExecutionModeResult) => void;
  version?: string | null;
  continueSessionId?: string | null;
}

/**
 * ExecutionModeSelectorScreen - Screen for selecting execution mode (2-step)
 * Step 1: Select mode (New/Continue/Resume)
 * Step 2: Select skip permissions (Yes/No)
 * Layout: Header + Selection + Footer
 */
export function ExecutionModeSelectorScreen({
  onBack,
  onSelect,
  version,
  continueSessionId = null,
}: ExecutionModeSelectorScreenProps) {
  const { rows } = useTerminalSize();
  const [step, setStep] = useState<1 | 2>(1);
  const [selectedMode, setSelectedMode] = useState<ExecutionMode | null>(null);

  // Handle keyboard input
  useInput((input, key) => {
    if (key.escape) {
      if (step === 2) {
        // Go back to step 1
        setStep(1);
        setSelectedMode(null);
      } else {
        // Go back to previous screen
        onBack();
      }
    }
  });

  // Execution mode options (Step 1)
  const modeItems: ExecutionModeItem[] = [
    {
      label: "New",
      value: "normal",
      description: "Start fresh session",
    },
    {
      label: continueSessionId
        ? `Continue (ID: ${continueSessionId})`
        : "Continue",
      value: "continue",
      description: "Continue from last session",
    },
    {
      label: "Resume",
      value: "resume",
      description: "Resume specific session",
    },
  ];

  // Skip permissions options (Step 2)
  const skipPermissionsItems: SkipPermissionsItem[] = [
    {
      label: "No",
      value: "no",
      description: "Normal permission checks",
    },
    {
      label: "Yes",
      value: "yes",
      description:
        "Skip permission checks (--dangerously-skip-permissions / --yolo)",
    },
  ];

  // Handle mode selection (Step 1)
  const handleModeSelect = (item: ExecutionModeItem) => {
    setSelectedMode(item.value);
    setStep(2);
  };

  // Handle skip permissions selection (Step 2)
  const handleSkipPermissionsSelect = (item: SkipPermissionsItem) => {
    if (selectedMode) {
      onSelect({
        mode: selectedMode,
        skipPermissions: item.value === "yes",
      });
    }
  };

  // Footer actions
  const footerActions = [
    { key: "enter", description: "Select" },
    { key: "esc", description: step === 2 ? "Back to mode selection" : "Back" },
  ];

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header
        title={step === 1 ? "Execution Mode" : "Skip Permissions"}
        titleColor="magenta"
        version={version}
      />

      {/* Content */}
      <Box flexDirection="column" flexGrow={1} marginTop={1}>
        {step === 1 ? (
          <>
            <Box marginBottom={1}>
              <Text>Select execution mode:</Text>
            </Box>
            <Select items={modeItems} onSelect={handleModeSelect} />
          </>
        ) : (
          <>
            <Box marginBottom={1}>
              <Text>
                Skip permission checks? (--dangerously-skip-permissions /
                --yolo)
              </Text>
            </Box>
            <Select
              items={skipPermissionsItems}
              onSelect={handleSkipPermissionsSelect}
            />
          </>
        )}
      </Box>

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
