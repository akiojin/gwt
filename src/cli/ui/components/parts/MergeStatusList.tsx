import React from "react";
import { Box, Text } from "ink";
import type { BranchMergeStatus } from "../../types.js";

export interface MergeStatusListProps {
  statuses: BranchMergeStatus[];
  maxVisible?: number;
}

/**
 * Get status icon and color based on merge status
 */
function getStatusDisplay(status: BranchMergeStatus["status"]): {
  icon: string;
  color: string;
} {
  switch (status) {
    case "success":
      return { icon: "✓", color: "green" };
    case "skipped":
      return { icon: "⊘", color: "yellow" };
    case "failed":
      return { icon: "✗", color: "red" };
  }
}

/**
 * MergeStatusList component - displays list of merge statuses
 * Optimized with React.memo to prevent unnecessary re-renders
 * @see specs/SPEC-ee33ca26/spec.md - FR-015
 */
export const MergeStatusList = React.memo(function MergeStatusList({
  statuses,
  maxVisible = 10,
}: MergeStatusListProps) {
  const visibleStatuses = statuses.slice(-maxVisible);
  const hiddenCount = statuses.length - visibleStatuses.length;

  return (
    <Box flexDirection="column">
      {hiddenCount > 0 && (
        <Box marginBottom={1}>
          <Text dimColor>... {hiddenCount} more branches above</Text>
        </Box>
      )}

      {visibleStatuses.map((status) => {
        const { icon, color } = getStatusDisplay(status.status);

        return (
          <Box key={status.branchName}>
            <Text color={color}>{icon} </Text>
            <Text>{status.branchName}</Text>
            <Text dimColor> ({status.durationSeconds.toFixed(1)}s)</Text>

            {status.status === "skipped" && status.conflictFiles && (
              <Text color="yellow">
                {" "}
                - Conflicts: {status.conflictFiles.length} files
              </Text>
            )}

            {status.status === "failed" && status.error && (
              <Text color="red"> - {status.error}</Text>
            )}

            {status.worktreeCreated && (
              <Text dimColor> [worktree created]</Text>
            )}
          </Box>
        );
      })}
    </Box>
  );
});
