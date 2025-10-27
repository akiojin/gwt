import React from "react";
import { Box, Text } from "ink";
import type { BatchMergeProgress } from "../../types.js";

export interface ProgressBarProps {
  progress: BatchMergeProgress;
  width?: number;
}

/**
 * Format seconds to human-readable time (e.g., "1m 23s", "45s")
 */
function formatTime(seconds: number): string {
  if (seconds < 60) {
    return `${Math.floor(seconds)}s`;
  }

  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = Math.floor(seconds % 60);
  return `${minutes}m ${remainingSeconds}s`;
}

/**
 * ProgressBar component - displays batch merge progress
 * Optimized with React.memo to prevent unnecessary re-renders
 * @see specs/SPEC-ee33ca26/spec.md - FR-009
 */
export const ProgressBar = React.memo(function ProgressBar({
  progress,
  width = 40,
}: ProgressBarProps) {
  const filledWidth = Math.floor((progress.percentage / 100) * width);
  const emptyWidth = width - filledWidth;

  const filledBar = "█".repeat(filledWidth);
  const emptyBar = "░".repeat(emptyWidth);

  return (
    <Box flexDirection="column">
      <Box>
        <Text dimColor>Current: </Text>
        <Text bold color="cyan">
          {progress.currentBranch}
        </Text>
        <Text dimColor>
          {" "}
          ({progress.currentIndex + 1}/{progress.totalBranches})
        </Text>
      </Box>

      <Box marginTop={1}>
        <Text color="green">{filledBar}</Text>
        <Text dimColor>{emptyBar}</Text>
        <Text dimColor> {progress.percentage}%</Text>
      </Box>

      <Box marginTop={1}>
        <Text dimColor>Phase: </Text>
        <Text color="yellow">{progress.currentPhase}</Text>
        <Text dimColor> | Elapsed: </Text>
        <Text color="magenta">{formatTime(progress.elapsedSeconds)}</Text>
        {progress.estimatedRemainingSeconds !== undefined && (
          <>
            <Text dimColor> | Remaining: </Text>
            <Text color="gray">
              ~{formatTime(progress.estimatedRemainingSeconds)}
            </Text>
          </>
        )}
      </Box>
    </Box>
  );
});
