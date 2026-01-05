/** @jsxImportSource @opentui/solid */
import { useKeyboard } from "@opentui/solid";
import { createMemo } from "solid-js";
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
}

export function LogScreen({
  entries,
  loading = false,
  error = null,
  onBack,
  onSelect,
  onCopy,
  onPickDate,
  notification,
  version,
  selectedDate,
}: LogScreenProps) {
  const terminal = useTerminalSize();
  const listHeight = createMemo(() => {
    const headerRows = 2;
    const infoRows = 1;
    const footerRows = 1;
    const notificationRows = notification ? 1 : 0;
    const reserved = headerRows + infoRows + footerRows + notificationRows;
    return Math.max(1, terminal().rows - reserved);
  });

  const list = useScrollableList({
    items: () => entries,
    visibleCount: listHeight,
  });

  const currentEntry = createMemo(() => entries[list.selectedIndex()]);

  const updateSelectedIndex = (value: number | ((prev: number) => number)) => {
    list.setSelectedIndex(value);
  };

  useKeyboard((key) => {
    if (key.name === "escape" || key.name === "q") {
      onBack();
      return;
    }

    if (key.name === "c") {
      const entry = currentEntry();
      if (entry) {
        onCopy(entry);
      }
      return;
    }

    if (key.name === "d") {
      onPickDate?.();
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
      updateSelectedIndex(entries.length - 1);
      return;
    }

    if (key.name === "return" || key.name === "linefeed") {
      const entry = currentEntry();
      if (entry) {
        onSelect(entry);
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
      <Header title="gwt - Log Viewer" titleColor="cyan" version={version} />

      {notification ? (
        <text fg={notification.tone === "error" ? "red" : "green"}>
          {notification.message}
        </text>
      ) : null}

      <box flexDirection="row">
        <text attributes={TextAttributes.DIM}>Date: </text>
        <text attributes={TextAttributes.BOLD}>{selectedDate ?? "---"}</text>
        <text attributes={TextAttributes.DIM}> Total: </text>
        <text attributes={TextAttributes.BOLD}>{entries.length}</text>
      </box>

      <box flexDirection="column" flexGrow={1}>
        {loading ? (
          <text fg="gray">Loading logs...</text>
        ) : entries.length === 0 ? (
          <text fg="gray">No logs available.</text>
        ) : (
          <box flexDirection="column">
            {list.visibleItems().map((entry, index) => {
              const absoluteIndex = list.scrollOffset() + index;
              const isSelected = absoluteIndex === list.selectedIndex();
              const indicator = isSelected ? ">" : " ";
              return (
                <text
                  {...(isSelected
                    ? { fg: "cyan", attributes: TextAttributes.BOLD }
                    : {})}
                >
                  {`${indicator} ${entry.summary}`}
                </text>
              );
            })}
          </box>
        )}

        {error ? <text fg="red">{error}</text> : null}
      </box>

      <Footer actions={footerActions()} />
    </box>
  );
}
