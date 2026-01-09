/** @jsxImportSource @opentui/solid */
import { TextAttributes } from "@opentui/core";
import { createEffect, createMemo } from "solid-js";
import type { ToolSessionEntry } from "../../../../config/index.js";
import { SelectInput, type SelectInputItem } from "./SelectInput.js";

/**
 * QuickStartStep - 前回履歴からのクイック選択ステップ
 *
 * FR-050: 前回履歴がある場合に表示
 * FR-051: 「Choose different settings...」で設定選択フローへ
 */

export interface QuickStartStepProps {
  history: ToolSessionEntry[];
  onResume: (entry: ToolSessionEntry) => void;
  onStartNew: (entry: ToolSessionEntry) => void;
  onChooseDifferent: () => void;
  onBack: () => void;
  focused?: boolean;
}

interface QuickStartItem extends SelectInputItem {
  action: "resume" | "start-new" | "choose-different";
  entry?: ToolSessionEntry;
}

export function QuickStartStep(props: QuickStartStepProps) {
  // T506: 履歴がない場合は自動的に onChooseDifferent を呼ぶ
  createEffect(() => {
    if (props.history.length === 0) {
      props.onChooseDifferent();
    }
  });

  const buildSettingsDescription = (entry: ToolSessionEntry): string => {
    const parts: string[] = [];
    if (entry.toolLabel) {
      parts.push(entry.toolLabel);
    } else if (entry.toolId) {
      parts.push(entry.toolId);
    }
    if (entry.model) {
      parts.push(entry.model);
    }
    if (entry.toolId === "codex-cli" && entry.reasoningLevel) {
      parts.push(entry.reasoningLevel);
    }
    return parts.join(", ");
  };

  const items = createMemo<QuickStartItem[]>(() => {
    const result: QuickStartItem[] = [];

    props.history.forEach((entry, index) => {
      if (!entry) return;
      const settingsDesc = buildSettingsDescription(entry);
      const sessionId =
        entry.sessionId && entry.sessionId.trim().length > 0
          ? entry.sessionId.trim()
          : null;
      const suffix = `${entry.toolId ?? "tool"}-${index}`;

      if (sessionId) {
        result.push({
          label: "Resume session (previous settings)",
          value: `resume-${suffix}`,
          description: `${settingsDesc} | Session: ${sessionId}`,
          action: "resume",
          entry,
        });
      }

      result.push({
        label: "Start new (previous settings)",
        value: `start-new-${suffix}`,
        description: settingsDesc,
        action: "start-new",
        entry,
      });
    });

    // Add "Choose different settings..." at the end
    result.push({
      label: "Choose different settings...",
      value: "choose-different",
      description: "Configure manually",
      action: "choose-different",
    });

    return result;
  });

  const handleSelect = (item: SelectInputItem) => {
    const quickItem = item as QuickStartItem;
    switch (quickItem.action) {
      case "resume":
        if (quickItem.entry) {
          props.onResume(quickItem.entry);
        }
        break;
      case "start-new":
        if (quickItem.entry) {
          props.onStartNew(quickItem.entry);
        }
        break;
      case "choose-different":
        props.onChooseDifferent();
        break;
    }
  };

  return (
    <box flexDirection="column" padding={1}>
      <text fg="cyan" attributes={TextAttributes.BOLD}>
        Quick Start
      </text>
      <text> </text>
      <SelectInput
        items={items()}
        onSelect={handleSelect}
        focused={props.focused ?? true}
        showDescription={true}
      />
      <text> </text>
      <text attributes={TextAttributes.DIM}>
        [Esc] Cancel [Enter] Select [Up/Down] Navigate
      </text>
    </box>
  );
}
