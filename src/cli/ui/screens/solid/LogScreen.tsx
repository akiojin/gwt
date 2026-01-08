/** @jsxImportSource @opentui/solid */
import { useKeyboard } from "@opentui/solid";
import { createMemo, mergeProps } from "solid-js";
import { TextAttributes } from "@opentui/core";
import { Header } from "../../components/solid/Header.js";
import { Footer } from "../../components/solid/Footer.js";
import { useTerminalSize } from "../../hooks/solid/useTerminalSize.js";
import type { FormattedLogEntry } from "../../../../logging/formatter.js";
import { useScrollableList } from "../../hooks/solid/useScrollableList.js";

export interface LogScreenProps {
  entries: FormattedLogEntry[];
  loading?: boolean;
  error?: string | null;
  onBack: () => void;
  onSelect: (entry: FormattedLogEntry) => void;
  onCopy: (entry: FormattedLogEntry) => void;
  onPickDate?: () => void;
  notification?: { message: string; tone: "success" | "error" } | null;
  version?: string | null;
  selectedDate?: string | null;
  helpVisible?: boolean;
}

export function LogScreen(props: LogScreenProps) {
  const merged = mergeProps(
    {
      loading: false,
      error: null,
      helpVisible: false,
    },
    props,
  );
  const terminal = useTerminalSize();
  const listHeight = createMemo(() => {
    const headerRows = 2;
    const infoRows = 1;
    const footerRows = 1;
    const notificationRows = merged.notification ? 1 : 0;
    const reserved = headerRows + infoRows + footerRows + notificationRows;
    return Math.max(1, terminal().rows - reserved);
  });

  const list = useScrollableList({
    items: () => merged.entries,
    visibleCount: listHeight,
  });

  const currentEntry = createMemo(() => merged.entries[list.selectedIndex()]);

  const updateSelectedIndex = (value: number | ((prev: number) => number)) => {
    list.setSelectedIndex(value);
  };

  useKeyboard((key) => {
    if (merged.helpVisible) {
      return;
    }
    if (key.name === "escape" || key.name === "q") {
      merged.onBack();
      return;
    }

    if (key.name === "c") {
      const entry = currentEntry();
      if (entry) {
        merged.onCopy(entry);
      }
      return;
    }

    if (key.name === "d") {
      merged.onPickDate?.();
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
      updateSelectedIndex(merged.entries.length - 1);
      return;
    }

    if (key.name === "return" || key.name === "linefeed") {
      const entry = currentEntry();
      if (entry) {
        merged.onSelect(entry);
      }
    }
  });

  const footerActions = createMemo(() => {
    const actions = [
      { key: "enter", description: "Detail" },
      { key: "c", description: "Copy" },
      { key: "d", description: "Date" },
      { key: "esc", description: "Back" },
    ];
    return actions;
  });

  return (
    <box flexDirection="column" height={terminal().rows}>
      <Header
        title="gwt - Log Viewer"
        titleColor="cyan"
        version={merged.version}
      />

      {merged.notification ? (
        <text fg={merged.notification.tone === "error" ? "red" : "green"}>
          {merged.notification.message}
        </text>
      ) : null}

      <box flexDirection="row">
        <text attributes={TextAttributes.DIM}>Date: </text>
        <text attributes={TextAttributes.BOLD}>
          {merged.selectedDate ?? "---"}
        </text>
        <text attributes={TextAttributes.DIM}> Total: </text>
        <text attributes={TextAttributes.BOLD}>{merged.entries.length}</text>
      </box>

      <box flexDirection="column" flexGrow={1}>
        {merged.loading ? (
          <text fg="gray">Loading logs...</text>
        ) : merged.entries.length === 0 ? (
          <text fg="gray">No logs available.</text>
        ) : (
          <box flexDirection="column">
            {list.visibleItems().map((entry, index) => {
              const absoluteIndex = list.scrollOffset() + index;
              const isSelected = absoluteIndex === list.selectedIndex();
              return (
                <text
                  {...(isSelected
                    ? { fg: "cyan", attributes: TextAttributes.BOLD }
                    : {})}
                >
                  {entry.summary}
                </text>
              );
            })}
          </box>
        )}

        {merged.error ? <text fg="red">{merged.error}</text> : null}
      </box>

      <Footer actions={footerActions()} />
    </box>
  );
}
