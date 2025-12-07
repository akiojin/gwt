import React from "react";
import { Box, Text, useInput } from "ink";
import { Header } from "../parts/Header.js";
import { Footer } from "../parts/Footer.js";
import { Select, type SelectItem } from "../common/Select.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";

export type QuickStartAction = "reuse-continue" | "reuse-new" | "manual";

export interface BranchQuickStartOption {
  toolId?: string | null;
  toolLabel: string;
  model?: string | null;
  sessionId?: string | null;
  inferenceLevel?: string | null;
  skipPermissions?: boolean | null;
}

const REASONING_LABELS: Record<string, string> = {
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra high",
};

const formatReasoning = (level?: string | null) =>
  level ? REASONING_LABELS[level] ?? level : "Default";

const formatSkip = (skip?: boolean | null) =>
  skip === true ? "Yes" : skip === false ? "No" : "No";

const supportsReasoning = (toolId?: string | null) =>
  toolId === "codex-cli";

const describe = (opt: BranchQuickStartOption, includeSessionId = true) => {
  const parts = [opt.toolLabel, opt.model ?? "default"];
  if (supportsReasoning(opt.toolId)) {
    parts.push(`Reasoning: ${formatReasoning(opt.inferenceLevel)}`);
  }
  parts.push(`Skip: ${formatSkip(opt.skipPermissions)}`);
  if (includeSessionId) {
    parts.push(opt.sessionId ? `ID: ${opt.sessionId}` : "No ID");
  }
  return parts.join(" / ");
};

type QuickStartItem = SelectItem & {
  description: string;
  disabled?: boolean;
  toolId?: string | null;
  action: QuickStartAction;
};

export interface BranchQuickStartScreenProps {
  previousOptions: BranchQuickStartOption[];
  loading?: boolean;
  onBack: () => void;
  onSelect: (action: QuickStartAction, toolId?: string | null) => void;
  version?: string | null;
  branchName: string;
}

export function BranchQuickStartScreen({
  previousOptions,
  loading = false,
  onBack,
  onSelect,
  version,
  branchName,
}: BranchQuickStartScreenProps) {
  const { rows } = useTerminalSize();

  const items: QuickStartItem[] = previousOptions.length
    ? previousOptions.flatMap((opt) => [
        {
          label: `Resume with previous settings (${opt.toolLabel})`,
          value: `reuse-continue:${opt.toolId ?? "unknown"}`,
          action: "reuse-continue",
          toolId: opt.toolId ?? null,
          description: describe(opt, true),
        },
        {
          label: `Start new with previous settings (${opt.toolLabel})`,
          value: `reuse-new:${opt.toolId ?? "unknown"}`,
          action: "reuse-new",
          toolId: opt.toolId ?? null,
          description: describe(opt, false),
        },
      ])
    : [
        {
          label: "Resume with previous settings",
          value: "reuse-continue",
          action: "reuse-continue",
          description: "No previous settings (disabled)",
          disabled: true,
        },
        {
          label: "Start new with previous settings",
          value: "reuse-new",
          action: "reuse-new",
          description: "No previous settings (disabled)",
          disabled: true,
        },
      ];

  items.push({
    label: "Choose manually",
    value: "manual",
    action: "manual",
    description: "Pick tool and model manually",
  });

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
            onSelect(item.action, item.toolId ?? null);
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
