import React from "react";
import { Box, Text, useInput } from "ink";
import { Header } from "../parts/Header.js";
import { Footer } from "../parts/Footer.js";
import { MergeStatusList } from "../parts/MergeStatusList.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";
import type { BatchMergeResult } from "../../types.js";

export interface BatchMergeResultScreenProps {
  result: BatchMergeResult;
  onBack?: () => void;
  onQuit?: () => void;
}

/**
 * BatchMergeResultScreen - Final result summary for batch merge
 * Layout: Header + Summary + Status List + Footer
 * @see specs/SPEC-ee33ca26/spec.md - FR-010
 */
export function BatchMergeResultScreen({
  result,
  onBack,
  onQuit,
}: BatchMergeResultScreenProps) {
  const { rows } = useTerminalSize();

  // Handle keyboard input
  useInput((input, key) => {
    if (input === "q" && onQuit) {
      onQuit();
    } else if (key.escape && onBack) {
      onBack();
    }
  });

  const { summary, totalDurationSeconds, cancelled } = result;

  return (
    <Box flexDirection="column" height={rows}>
      <Header
        title={cancelled ? "Batch Merge Cancelled" : "Batch Merge Complete"}
      />

      <Box flexDirection="column" paddingX={2} paddingY={1}>
        {/* Summary Statistics */}
        <Box flexDirection="column" marginBottom={2}>
          <Text bold>Summary:</Text>
          <Box marginTop={1}>
            <Text color="green">✓ Success: {summary.successCount}</Text>
            <Text dimColor> | </Text>
            <Text color="yellow">⊘ Skipped: {summary.skippedCount}</Text>
            <Text dimColor> | </Text>
            <Text color="red">✗ Failed: {summary.failedCount}</Text>
            <Text dimColor> | </Text>
            <Text dimColor>Total: {totalDurationSeconds.toFixed(1)}s</Text>
          </Box>

          {summary.pushedCount > 0 && (
            <Box marginTop={1}>
              <Text color="cyan">↑ Pushed: {summary.pushedCount}</Text>
              {summary.pushFailedCount > 0 && (
                <>
                  <Text dimColor> | </Text>
                  <Text color="red">
                    Push Failed: {summary.pushFailedCount}
                  </Text>
                </>
              )}
            </Box>
          )}
        </Box>

        {/* Detailed Status List */}
        <Box flexDirection="column">
          <Text bold>Branch Details:</Text>
          <Box marginTop={1}>
            <MergeStatusList
              statuses={result.statuses}
              maxVisible={rows - 12}
            />
          </Box>
        </Box>

        {/* Messages for next actions */}
        {summary.skippedCount > 0 && (
          <Box marginTop={2}>
            <Text color="yellow">
              Note: {summary.skippedCount} branch
              {summary.skippedCount > 1 ? "es" : ""} skipped due to conflicts.
              Please resolve manually.
            </Text>
          </Box>
        )}
      </Box>

      <Footer
        actions={[
          { key: "q", description: "Quit" },
          ...(onBack ? [{ key: "ESC", description: "Back" }] : []),
        ]}
      />
    </Box>
  );
}
