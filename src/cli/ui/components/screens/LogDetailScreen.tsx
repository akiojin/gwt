import React, { useMemo } from "react";
import { Box, Text } from "ink";
import { Header } from "../parts/Header.js";
import { Footer } from "../parts/Footer.js";
import { useAppInput } from "../../hooks/useAppInput.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";
import type { FormattedLogEntry } from "../../../../logging/formatter.js";

export interface LogDetailScreenProps {
  entry: FormattedLogEntry | null;
  onBack: () => void;
  onCopy: (entry: FormattedLogEntry) => void;
  notification?: { message: string; tone: "success" | "error" } | null;
  version?: string | null;
}

export function LogDetailScreen({
  entry,
  onBack,
  onCopy,
  notification,
  version,
}: LogDetailScreenProps) {
  const { rows } = useTerminalSize();

  useAppInput((input, key) => {
    if (key.escape || input === "q") {
      onBack();
      return;
    }

    if (input === "c" && entry) {
      onCopy(entry);
    }
  });

  const jsonLines = useMemo<string[]>(() => {
    if (!entry) return ["ログがありません"]; // fallback
    return entry.json.split("\n");
  }, [entry]);

  const footerActions = [
    { key: "c", description: "Copy" },
    { key: "esc", description: "Back" },
  ];

  return (
    <Box flexDirection="column" height={rows}>
      <Header title="gwt - Log Detail" titleColor="cyan" version={version} />

      {notification ? (
        <Box marginTop={1}>
          <Text color={notification.tone === "error" ? "red" : "green"}>
            {notification.message}
          </Text>
        </Box>
      ) : null}

      <Box flexDirection="column" flexGrow={1} marginTop={1}>
        {jsonLines.map((line, index) => (
          <Text key={`${line}-${index}`}>{line}</Text>
        ))}
      </Box>

      <Footer actions={footerActions} />
    </Box>
  );
}
