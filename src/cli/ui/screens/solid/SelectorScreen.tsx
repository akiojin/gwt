/** @jsxImportSource @opentui/solid */
import { useKeyboard } from "@opentui/solid";
import { createEffect, createMemo } from "solid-js";
import { TextAttributes } from "@opentui/core";
import { Header } from "../../components/solid/Header.js";
import { Footer } from "../../components/solid/Footer.js";
import { useTerminalSize } from "../../hooks/solid/useTerminalSize.js";
import { useScrollableList } from "../../hooks/solid/useScrollableList.js";

export interface SelectorItem {
  label: string;
  value: string;
  description?: string;
}

export interface SelectorScreenProps {
  title: string;
  description?: string;
  items: SelectorItem[];
  onSelect: (item: SelectorItem) => void;
  onBack?: () => void;
  version?: string | null;
  emptyMessage?: string;
  selectedIndex?: number;
  onSelectedIndexChange?: (index: number) => void;
  showDescription?: boolean;
  helpVisible?: boolean;
}

const clamp = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), max);

export function SelectorScreen({
  title,
  description,
  items,
  onSelect,
  onBack,
  version,
  emptyMessage = "No options available.",
  selectedIndex: controlledIndex,
  onSelectedIndexChange,
  showDescription = false,
  helpVisible = false,
}: SelectorScreenProps) {
  const terminal = useTerminalSize();
  const listHeight = createMemo(() => {
    const headerRows = 2;
    const descriptionRows = description ? 1 : 0;
    const footerRows = 1;
    const reserved = headerRows + descriptionRows + footerRows;
    return Math.max(1, terminal().rows - reserved);
  });

  const list = useScrollableList({
    items: () => items,
    visibleCount: listHeight,
    initialIndex: controlledIndex ?? 0,
  });

  createEffect(() => {
    if (controlledIndex === undefined) {
      return;
    }
    list.setSelectedIndex(clamp(controlledIndex, 0, items.length - 1));
  });

  const selectedIndex = list.selectedIndex;

  const updateSelectedIndex = (value: number | ((prev: number) => number)) => {
    list.setSelectedIndex(value);
    onSelectedIndexChange?.(selectedIndex());
  };

  useKeyboard((key) => {
    if (helpVisible) {
      return;
    }
    if (key.name === "escape" || key.name === "q") {
      onBack?.();
      return;
    }

    if (key.name === "down") {
      updateSelectedIndex((prev) => prev + 1);
      return;
    }

    if (key.name === "up") {
      updateSelectedIndex((prev) => prev - 1);
      return;
    }

    if (key.name === "pageup") {
      updateSelectedIndex((prev) => prev - listHeight());
      return;
    }

    if (key.name === "pagedown") {
      updateSelectedIndex((prev) => prev + listHeight());
      return;
    }

    if (key.name === "home") {
      updateSelectedIndex(0);
      return;
    }

    if (key.name === "end") {
      updateSelectedIndex(items.length - 1);
      return;
    }

    if (key.name === "return" || key.name === "linefeed") {
      const item = items[selectedIndex()];
      if (item) {
        onSelect(item);
      }
    }
  });

  const footerActions = createMemo(() => {
    const actions = [{ key: "enter", description: "Select" }];
    if (onBack) {
      actions.push({ key: "esc", description: "Back" });
    }
    return actions;
  });

  return (
    <box flexDirection="column" height={terminal().rows}>
      <Header title={title} titleColor="cyan" version={version} />
      {description && <text>{description}</text>}

      <box flexDirection="column" flexGrow={1}>
        {items.length === 0 ? (
          <text fg="gray">{emptyMessage}</text>
        ) : (
          <box flexDirection="column">
            {list.visibleItems().map((item, index) => {
              const absoluteIndex = list.scrollOffset() + index;
              const isSelected = absoluteIndex === selectedIndex();
              const indicator = isSelected ? ">" : " ";
              return (
                <box flexDirection="row">
                  <text
                    {...(isSelected
                      ? { fg: "cyan", attributes: TextAttributes.BOLD }
                      : {})}
                  >
                    {`${indicator} ${item.label}`}
                  </text>
                  {showDescription && item.description ? (
                    <text
                      attributes={TextAttributes.DIM}
                      {...(isSelected ? { fg: "cyan" } : {})}
                    >
                      {` - ${item.description}`}
                    </text>
                  ) : null}
                </box>
              );
            })}
          </box>
        )}
      </box>

      <Footer actions={footerActions()} />
    </box>
  );
}
