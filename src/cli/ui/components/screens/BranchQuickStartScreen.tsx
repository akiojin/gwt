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
  toolCategory?: "Codex" | "Claude" | "Gemini" | "Qwen" | "Other";
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
  const parts = [`Model: ${opt.model ?? "default"}`];
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
  groupStart?: boolean;
  category: string;
  categoryColor: "cyan" | "yellow" | "magenta" | "green" | "white";
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
  const containerHeight = rows && rows > 0 ? rows : undefined;
  const pendingEnterRef = React.useRef(false);

  const CATEGORY_META = {
    "codex-cli": { label: "Codex", color: "cyan" },
    "claude-code": { label: "Claude", color: "yellow" },
    "gemini-cli": { label: "Gemini", color: "magenta" },
    "qwen-cli": { label: "Qwen", color: "green" },
    other: { label: "Other", color: "white" },
  } as const;

  type CategoryMeta = (typeof CATEGORY_META)[keyof typeof CATEGORY_META];

  const resolveCategory = (toolId?: string | null): CategoryMeta => {
    switch (toolId) {
      case "codex-cli":
        return CATEGORY_META["codex-cli"];
      case "claude-code":
        return CATEGORY_META["claude-code"];
      case "gemini-cli":
        return CATEGORY_META["gemini-cli"];
      case "qwen-cli":
        return CATEGORY_META["qwen-cli"];
      default:
        return CATEGORY_META.other;
    }
  };

  const items: QuickStartItem[] = previousOptions.length
    ? (() => {
        const order = ["Claude", "Codex", "Gemini", "Qwen", "Other"];
      const sorted = [...previousOptions].sort((a, b) => {
        const ca = resolveCategory(a.toolId).label;
        const cb = resolveCategory(b.toolId).label;
        return order.indexOf(ca) - order.indexOf(cb);
      });

      const flat: QuickStartItem[] = [];
        sorted.forEach((opt, idx) => {
          const cat = resolveCategory(opt.toolId);
          const prevCat =
            idx > 0 ? resolveCategory(sorted[idx - 1]?.toolId).label : null;
          const isNewCategory = prevCat !== cat.label;

        flat.push(
          {
            label: "Resume",
            value: `reuse-continue:${opt.toolId ?? "unknown"}:${idx}`,
            action: "reuse-continue",
            toolId: opt.toolId ?? null,
            description: describe(opt, true),
            groupStart: isNewCategory && flat.length > 0,
            category: cat.label,
            categoryColor: cat.color,
          },
          {
            label: "New",
            value: `reuse-new:${opt.toolId ?? "unknown"}:${idx}`,
            action: "reuse-new",
            toolId: opt.toolId ?? null,
            description: describe(opt, false),
            groupStart: false,
            category: cat.label,
            categoryColor: cat.color,
          },
        );
      });

      return flat;
    })()
    : [
        {
          label: "Resume with previous settings",
          value: "reuse-continue",
          action: "reuse-continue",
          description: "No previous settings (disabled)",
          disabled: true,
          category: CATEGORY_META.other.label,
          categoryColor: CATEGORY_META.other.color,
        },
        {
          label: "Start new with previous settings",
          value: "reuse-new",
          action: "reuse-new",
          description: "No previous settings (disabled)",
          disabled: true,
          category: CATEGORY_META.other.label,
          categoryColor: CATEGORY_META.other.color,
        },
      ];

  items.push({
    label: "Manual selection",
    value: "manual",
    action: "manual",
    description: "Pick tool and model manually",
    category: CATEGORY_META.other.label,
    categoryColor: CATEGORY_META.other.color,
  });

  useInput((_, key) => {
    if (key.escape) {
      onBack();
      return;
    }
    if (key.return) {
      if (!loading && items.length > 0) {
        onSelect(
          (items[0] as QuickStartItem).action,
          (items[0] as QuickStartItem).toolId ?? null,
        );
      } else {
        pendingEnterRef.current = true;
      }
    }
  });

  React.useEffect(() => {
    if (pendingEnterRef.current && !loading && items.length > 0) {
      pendingEnterRef.current = false;
      onSelect(
        (items[0] as QuickStartItem).action,
        (items[0] as QuickStartItem).toolId ?? null,
      );
    }
  }, [loading, items, onSelect]);

  return (
    <Box flexDirection="column" height={containerHeight}>
      <Header
        title="Quick Start"
        titleColor="cyan"
        version={version}
      />

      <Box flexDirection="column" flexGrow={1} marginTop={1}>
        <Box marginBottom={1} flexDirection="column">
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
            <Box
              flexDirection="column"
              marginTop={item.groupStart ? 1 : item.category === "Other" ? 1 : 0}
            >
              <Text>
                <Text
                  color={item.categoryColor}
                  inverse={isSelected}
                >
                  {`[${item.category}] `}
                </Text>
                <Text inverse={isSelected}>
                  {item.label}
                  {item.disabled ? " (disabled)" : ""}
                </Text>
              </Text>
              {item.description && (
                <Text color="gray">  {item.description}</Text>
              )}
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
