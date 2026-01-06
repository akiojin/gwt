/** @jsxImportSource @opentui/solid */
import { createEffect, For } from "solid-js";
import type { ToolSessionEntry } from "../../../../config/index.js";

/**
 * QuickStartStep - 前回履歴からのクイック選択ステップ
 *
 * FR-050: 前回履歴がある場合に表示
 * FR-051: 「Choose different settings...」で設定選択フローへ
 *
 * TODO: 実装予定（TDD RED状態）
 */

export interface QuickStartStepProps {
  history: ToolSessionEntry[];
  onResume: (entry: ToolSessionEntry) => void;
  onStartNew: (entry: ToolSessionEntry) => void;
  onChooseDifferent: () => void;
  onBack: () => void;
}

export function QuickStartStep(props: QuickStartStepProps) {
  // T506: 履歴がない場合は自動的に onChooseDifferent を呼ぶ
  createEffect(() => {
    if (props.history.length === 0) {
      props.onChooseDifferent();
    }
  });

  return (
    <box flexDirection="column" padding={1}>
      <text bold color="cyan">
        Quick Start
      </text>
      <text> </text>
      <For each={props.history}>
        {(entry) => (
          <box flexDirection="column" marginBottom={1}>
            <text bold>
              {entry.toolLabel} ({entry.model}
              {entry.reasoningLevel
                ? `, Reasoning: ${entry.reasoningLevel}`
                : ""}
              )
            </text>
            <text> Resume with previous settings</text>
            <text> Start new with previous settings</text>
          </box>
        )}
      </For>
      <text>---</text>
      <text> Choose different settings...</text>
      <text> </text>
      <text dimColor>[Esc] Cancel [Enter] Select [Up/Down] Navigate</text>
    </box>
  );
}
