import React from "react";
import { Box, Text, useInput } from "ink";
import { Header } from "../parts/Header.js";
import { Footer } from "../parts/Footer.js";
import { ProgressBar } from "../parts/ProgressBar.js";
import { MergeStatusList } from "../parts/MergeStatusList.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";
import type { BatchMergeProgress, BranchMergeStatus } from "../../types.js";

export interface BatchMergeProgressScreenProps {
  progress: BatchMergeProgress;
  statuses: BranchMergeStatus[];
  onCancel?: () => void;
}

/**
 * BatchMergeProgressScreen - Real-time progress display for batch merge
 * Layout: Header + Progress Bar + Status List + Footer
 * @see specs/SPEC-ee33ca26/spec.md - User Story 4
 */
export function BatchMergeProgressScreen({
  progress,
  statuses,
  onCancel,
}: BatchMergeProgressScreenProps) {
  const { rows } = useTerminalSize();

  // Handle keyboard input
  useInput((input, key) => {
    if ((input === "q" || key.escape) && onCancel) {
      onCancel();
    }
  });

  // Calculate available space for status list
  const headerLines = 1;
  const progressLines = 5; // Progress bar takes ~5 lines
  const footerLines = 1;
  const maxStatusLines = Math.max(
    5,
    rows - headerLines - progressLines - footerLines - 2,
  );

  return (
    <Box flexDirection="column" height={rows}>
      <Header title="Batch Merge in Progress" />

      <Box flexDirection="column" paddingX={2} paddingY={1}>
        <ProgressBar progress={progress} />

        <Box marginTop={2}>
          <Text bold>Recent Branches:</Text>
        </Box>

        <Box marginTop={1}>
          <MergeStatusList statuses={statuses} maxVisible={maxStatusLines} />
        </Box>
      </Box>

      <Footer
        actions={[
          {
            key: "q",
            description: `Cancel | Processing ${progress.currentIndex + 1}/${progress.totalBranches}`,
          },
        ]}
      />
    </Box>
  );
}
