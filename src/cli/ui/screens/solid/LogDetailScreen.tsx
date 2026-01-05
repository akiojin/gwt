import { useKeyboard } from "@opentui/solid";
import { createMemo } from "solid-js";
import { Header } from "../../components/solid/Header.js";
import { Footer } from "../../components/solid/Footer.js";
import { useTerminalSize } from "../../hooks/solid/useTerminalSize.js";
import type { FormattedLogEntry } from "../../../logging/formatter.js";

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
  const terminal = useTerminalSize();

  useKeyboard((key) => {
    if (key.name === "escape" || key.name === "q") {
      onBack();
      return;
    }

    if (key.name === "c" && entry) {
      onCopy(entry);
    }
  });

  const jsonLines = createMemo(() => {
    if (!entry) {
      return ["No logs available."];
    }
    return entry.json.split("\n");
  });

  const footerActions = [
    { key: "c", description: "Copy" },
    { key: "esc", description: "Back" },
  ];

  return (
    <box flexDirection="column" height={terminal().rows}>
      <Header title="gwt - Log Detail" titleColor="cyan" version={version} />

      {notification ? (
        <box marginTop={1}>
          <text fg={notification.tone === "error" ? "red" : "green"}>
            {notification.message}
          </text>
        </box>
      ) : null}

      <box flexDirection="column" flexGrow={1} marginTop={1}>
        {jsonLines().map((line) => (
          <text>{line}</text>
        ))}
      </box>

      <Footer actions={footerActions} />
    </box>
  );
}
