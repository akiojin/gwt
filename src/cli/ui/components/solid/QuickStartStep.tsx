/** @jsxImportSource @opentui/solid */
import { TextAttributes } from "@opentui/core";
import type { SelectRenderable } from "@opentui/core";
import { useKeyboard } from "@opentui/solid";
import { createEffect, createMemo, createSignal } from "solid-js";
import type { ToolSessionEntry } from "../../../../config/index.js";
import { SelectInput, type SelectInputItem } from "./SelectInput.js";
import { useWizardScroll } from "./WizardPopup.js";

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
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  const [selectRef, setSelectRef] = createSignal<SelectRenderable | undefined>(
    undefined,
  );
  const scroll = useWizardScroll();

  // T506: 履歴がない場合は自動的に onChooseDifferent を呼ぶ
  createEffect(() => {
    if (props.history.length === 0) {
      props.onChooseDifferent();
    }
  });

  // Build selection items from history
  // 最新の履歴エントリのみを使用（重複排除済み）
  const latestEntry = createMemo(() => props.history[0] ?? null);

  const items = createMemo<QuickStartItem[]>(() => {
    const result: QuickStartItem[] = [];
    const entry = latestEntry();

    if (entry) {
      const reasoningInfo = entry.reasoningLevel
        ? `, ${entry.reasoningLevel}`
        : "";
      const settingsDesc = `${entry.toolLabel}, ${entry.model}${reasoningInfo}`;

      result.push({
        label: "Resume session (previous settings)",
        value: `resume-${entry.toolId}`,
        description: settingsDesc,
        action: "resume",
        entry,
      });

      result.push({
        label: "Start new (previous settings)",
        value: `start-new-${entry.toolId}`,
        description: settingsDesc,
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

  const ensureIndexVisible = (index: number) => {
    if (!scroll) {
      return;
    }
    const select = selectRef();
    if (!select) {
      return;
    }
    const linesPerItem = 2;
    const startLine = select.y + Math.max(0, index) * linesPerItem;
    const endLine = startLine + Math.max(0, linesPerItem - 1);
    scroll.ensureLineVisible(startLine);
    if (endLine !== startLine) {
      scroll.ensureLineVisible(endLine);
    }
  };

  createEffect(() => {
    if (props.focused === false) {
      return;
    }
    const count = items().length;
    if (count <= 0) {
      return;
    }
    const safeIndex = Math.min(Math.max(selectedIndex(), 0), count - 1);
    ensureIndexVisible(safeIndex);
  });

  useKeyboard((key) => {
    if (props.focused === false) {
      return;
    }
    if (!scroll) {
      return;
    }
    if (key.name !== "up" && key.name !== "down") {
      return;
    }
    const count = items().length;
    if (count <= 0) {
      return;
    }
    const currentIndex = Math.min(Math.max(selectedIndex(), 0), count - 1);
    const nextIndex =
      key.name === "up"
        ? Math.max(0, currentIndex - 1)
        : Math.min(count - 1, currentIndex + 1);
    if (nextIndex === currentIndex) {
      return;
    }
    ensureIndexVisible(nextIndex);
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

  const handleChange = (item: SelectInputItem | null) => {
    if (!item) {
      setSelectedIndex(0);
      return;
    }
    const nextIndex = items().findIndex((candidate) => {
      return candidate.value === item.value;
    });
    if (nextIndex >= 0) {
      setSelectedIndex(nextIndex);
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
        onChange={handleChange}
        focused={props.focused ?? true}
        showDescription={true}
        selectRef={setSelectRef}
      />
      <text> </text>
      <text attributes={TextAttributes.DIM}>
        [Esc] Cancel [Enter] Select [Up/Down] Navigate
      </text>
    </box>
  );
}
