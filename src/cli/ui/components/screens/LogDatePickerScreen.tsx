import React from "react";
import { Box, Text } from "ink";
import { Header } from "../parts/Header.js";
import { Footer } from "../parts/Footer.js";
import { Select } from "../common/Select.js";
import { useAppInput } from "../../hooks/useAppInput.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";
import type { LogFileInfo } from "../../../../logging/reader.js";

interface DateItem {
  label: string;
  value: string;
}

export interface LogDatePickerScreenProps {
  dates: LogFileInfo[];
  onBack: () => void;
  onSelect: (date: string) => void;
  version?: string | null;
}

export function LogDatePickerScreen({
  dates,
  onBack,
  onSelect,
  version,
}: LogDatePickerScreenProps) {
  const { rows } = useTerminalSize();

  useAppInput((input, key) => {
    if (key.escape || input === "q") {
      onBack();
    }
  });

  const items: DateItem[] = dates.map((date) => ({
    label: date.date,
    value: date.date,
  }));

  const handleSelect = (item: DateItem) => {
    onSelect(item.value);
  };

  const headerLines = 2;
  const statsLines = 1;
  const emptyLine = 1;
  const footerLines = 1;
  const fixedLines = headerLines + statsLines + emptyLine + footerLines;
  const contentHeight = rows - fixedLines;
  const limit = Math.max(5, contentHeight);

  const footerActions = [
    { key: "enter", description: "Select" },
    { key: "esc", description: "Back" },
  ];

  return (
    <Box flexDirection="column" height={rows}>
      <Header title="gwt - Log Date" titleColor="cyan" version={version} />

      <Box marginTop={1}>
        <Text>
          Total: <Text bold>{dates.length}</Text>
        </Text>
      </Box>

      <Box height={1} />

      <Box flexDirection="column" flexGrow={1}>
        {dates.length === 0 ? (
          <Box>
            <Text dimColor>No logs available.</Text>
          </Box>
        ) : (
          <Select items={items} onSelect={handleSelect} limit={limit} />
        )}
      </Box>

      <Footer actions={footerActions} />
    </Box>
  );
}
