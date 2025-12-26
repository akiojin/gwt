import React, { useCallback, useMemo, useState } from "react";
import { Box, Text } from "ink";
import stringWidth from "string-width";
import { Header } from "../parts/Header.js";
import { Footer } from "../parts/Footer.js";
import { Select } from "../common/Select.js";
import { useAppInput } from "../../hooks/useAppInput.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";
import type { FormattedLogEntry } from "../../../../logging/formatter.js";

interface LogListItem {
  label: string;
  value: string;
  entry: FormattedLogEntry;
}

export interface LogListScreenProps {
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

const truncateToWidth = (value: string, maxWidth: number): string => {
  if (maxWidth <= 0) return "";
  if (stringWidth(value) <= maxWidth) return value;
  const ellipsis = "…";
  const ellipsisWidth = stringWidth(ellipsis);
  if (ellipsisWidth >= maxWidth) return ellipsis;

  let result = "";
  for (const char of Array.from(value)) {
    if (stringWidth(result + char) + ellipsisWidth > maxWidth) {
      break;
    }
    result += char;
  }
  return result + ellipsis;
};

const padToWidth = (value: string, width: number): string => {
  if (width <= 0) return "";
  if (stringWidth(value) >= width) return value;
  return value + " ".repeat(width - stringWidth(value));
};

export function LogListScreen({
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
}: LogListScreenProps) {
  const { rows, columns } = useTerminalSize();
  const [selectedIndex, setSelectedIndex] = useState(0);

  const maxLabelWidth = Math.max(10, columns - 2);

  const items = useMemo<LogListItem[]>(
    () =>
      entries.map((entry) => ({
        label: truncateToWidth(entry.summary, maxLabelWidth),
        value: entry.id,
        entry,
      })),
    [entries, maxLabelWidth],
  );

  const handleSelect = useCallback(
    (item: LogListItem) => {
      onSelect(item.entry);
    },
    [onSelect],
  );

  const handleCopy = useCallback(() => {
    const entry = entries[selectedIndex];
    if (entry) {
      onCopy(entry);
    }
  }, [entries, selectedIndex, onCopy]);

  useAppInput((input, key) => {
    if (key.escape || input === "q") {
      onBack();
      return;
    }

    if (input === "c") {
      handleCopy();
      return;
    }

    if (input === "d" && onPickDate) {
      onPickDate();
    }
  });

  const renderItem = useCallback(
    (item: LogListItem, isSelected: boolean) => {
      if (!isSelected) {
        return <Text>{item.label}</Text>;
      }
      const padded = padToWidth(item.label, maxLabelWidth);
      const output = `\u001b[46m\u001b[30m${padded}\u001b[0m`;
      return <Text>{output}</Text>;
    },
    [maxLabelWidth],
  );

  const headerLines = 2;
  const statsLines = 1;
  const emptyLine = 1;
  const footerLines = 1;
  const fixedLines = headerLines + statsLines + emptyLine + footerLines;
  const contentHeight = rows - fixedLines;
  const limit = Math.max(5, contentHeight);

  const footerActions = [
    { key: "enter", description: "Detail" },
    { key: "c", description: "Copy" },
    { key: "d", description: "Date" },
    { key: "esc", description: "Back" },
  ];

  return (
    <Box flexDirection="column" height={rows}>
      <Header title="gwt - Log Viewer" titleColor="cyan" version={version} />

      {notification ? (
        <Box marginTop={1}>
          <Text color={notification.tone === "error" ? "red" : "green"}>
            {notification.message}
          </Text>
        </Box>
      ) : null}

      <Box marginTop={1}>
        <Box marginRight={2}>
          <Text>
            Date: <Text bold>{selectedDate ?? "---"}</Text>
          </Text>
        </Box>
        <Text>
          Total: <Text bold>{entries.length}</Text>
        </Text>
      </Box>

      <Box height={1} />

      <Box flexDirection="column" flexGrow={1}>
        {loading ? (
          <Box>
            <Text dimColor>Loading logs...</Text>
          </Box>
        ) : entries.length === 0 ? (
          <Box>
            <Text dimColor>ログがありません</Text>
          </Box>
        ) : (
          <Select
            items={items}
            onSelect={handleSelect}
            limit={limit}
            selectedIndex={selectedIndex}
            onSelectedIndexChange={setSelectedIndex}
            renderItem={renderItem}
          />
        )}

        {error ? (
          <Box marginTop={1}>
            <Text color="red">{error}</Text>
          </Box>
        ) : null}
      </Box>

      <Footer actions={footerActions} />
    </Box>
  );
}
