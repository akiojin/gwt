import React from "react";
import { Box, Text } from "ink";
import { Select, type SelectItem } from "../components/common/Select.js";
import type { BranchAction } from "../types.js";

export interface BranchActionSelectorScreenProps {
  selectedBranch: string;
  onUseExisting: () => void;
  onCreateNew: () => void;
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
}: BranchActionSelectorScreenProps) {
  const items: SelectItem[] = [
    {
      label: "既存のブランチで続行",
      value: "use-existing",
      description: "選択したブランチでそのまま作業を開始",
    },
    {
      label: "新しいブランチを作成",
      value: "create-new",
      description: "選択したブランチをベースに新しいブランチを作成",
    },
  ];

  const handleSelect = (item: SelectItem) => {
    const action = item.value as BranchAction;

    if (action === "use-existing") {
      onUseExisting();
    } else if (action === "create-new") {
      onCreateNew();
    }
  };

  return (
    <Box flexDirection="column">
      <Box marginBottom={1}>
        <Text>
          ブランチ <Text bold color="cyan">{selectedBranch}</Text> を選択しました
        </Text>
      </Box>
      <Box marginBottom={1}>
        <Text color="gray">次のアクションを選択してください:</Text>
      </Box>
      <Select items={items} onSelect={handleSelect} />
    </Box>
  );
}
