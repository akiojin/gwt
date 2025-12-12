import React from "react";
import { Box, Text, useInput } from "ink";
import { Select, type SelectItem } from "../components/common/Select.js";
import { Footer } from "../components/parts/Footer.js";
import type { BranchAction } from "../types.js";

export interface BranchActionSelectorScreenProps {
  selectedBranch: string;
  onUseExisting: () => void;
  onCreateNew: () => void;
  onBack: () => void;
  canCreateNew?: boolean;
  mode?: "default" | "protected";
  infoMessage?: string | null;
  primaryLabel?: string;
  secondaryLabel?: string;
}

/**
 * BranchActionSelectorScreen - Screen for selecting action after branch selection
 *
 * Allows user to choose between:
 * - Using existing branch (continue to AI tool selection)
 * - Creating new branch from selected branch (go to branch creator)
 */
export function BranchActionSelectorScreen({
  selectedBranch,
  onUseExisting,
  onCreateNew,
  onBack,
  canCreateNew = true,
  mode = "default",
  infoMessage,
  primaryLabel,
  secondaryLabel,
}: BranchActionSelectorScreenProps) {
  // Handle keyboard input for back navigation
  useInput((input, key) => {
    if (key.escape) {
      onBack();
    }
  });

  const primaryActionLabel =
    primaryLabel ??
    (mode === "protected" ? "Switch to root branch" : "Use existing branch");
  const secondaryActionLabel =
    secondaryLabel ??
    (mode === "protected"
      ? "Create new branch from this branch"
      : "Create new branch");

  const items: SelectItem[] = [
    {
      label: primaryActionLabel,
      value: "use-existing",
    },
  ];

  if (canCreateNew) {
    items.push({
      label: secondaryActionLabel,
      value: "create-new",
    });
  }

  const handleSelect = (item: SelectItem) => {
    const action = item.value as BranchAction;

    if (action === "use-existing") {
      onUseExisting();
    } else if (action === "create-new") {
      onCreateNew();
    }
  };

  // Footer actions
  const footerActions = [
    { key: "enter", description: "Select" },
    { key: "esc", description: "Back" },
  ];

  return (
    <Box flexDirection="column">
      <Box marginBottom={1}>
        <Text>
          Selected branch:{" "}
          <Text bold color="cyan">
            {selectedBranch}
          </Text>
        </Text>
      </Box>
      {infoMessage ? (
        <Box marginBottom={1}>
          <Text color="yellow">{infoMessage}</Text>
        </Box>
      ) : null}
      <Box marginBottom={1}>
        <Text color="gray">Choose an action:</Text>
      </Box>
      <Select items={items} onSelect={handleSelect} />

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
