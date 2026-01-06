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

  // Build selection items from history
  const items = createMemo<QuickStartItem[]>(() => {
    const result: QuickStartItem[] = [];

    for (const entry of props.history) {
      const reasoningInfo = entry.reasoningLevel
        ? `, Reasoning: ${entry.reasoningLevel}`
        : "";
      const header = `${entry.toolLabel} (${entry.model}${reasoningInfo})`;

      result.push({
        label: `${header} - Resume`,
        value: `resume-${entry.toolId}`,
        description: "Resume with previous settings",
        action: "resume",
        entry,
      });

      result.push({
        label: `${header} - Start new`,
        value: `start-new-${entry.toolId}`,
        description: "Start new with previous settings",
        action: "start-new",
        entry,
      });
    }

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
        focused={true}
        showDescription={true}
      />
      <text> </text>
      <text attributes={TextAttributes.DIM}>
        [Esc] Cancel [Enter] Select [Up/Down] Navigate
      </text>
    </box>
  );
}
