/** @jsxImportSource @opentui/solid */
import { useKeyboard } from "@opentui/solid";
import { createSignal } from "solid-js";
import stringWidth from "string-width";
import { useTerminalSize } from "../../hooks/solid/useTerminalSize.js";
import { selectionStyle } from "../../core/theme.js";

export interface ConfirmScreenProps {
  message: string;
  onConfirm: (confirmed: boolean) => void;
  yesLabel?: string;
  noLabel?: string;
  defaultNo?: boolean;
  helpVisible?: boolean;
}

export function ConfirmScreen({
  message,
  onConfirm,
  yesLabel = "Yes",
  noLabel = "No",
  defaultNo = false,
  helpVisible = false,
}: ConfirmScreenProps) {
  const [selectedIndex, setSelectedIndex] = createSignal(defaultNo ? 1 : 0);
  const terminal = useTerminalSize();

  const padLine = (value: string, width: number) => {
    const padding = Math.max(0, width - stringWidth(value));
    return padding > 0 ? `${value}${" ".repeat(padding)}` : value;
  };

  const confirm = (confirmed: boolean) => {
    onConfirm(confirmed);
  };

  const toggleSelection = () => {
    setSelectedIndex((current) => (current === 0 ? 1 : 0));
  };

  useKeyboard((key) => {
    if (helpVisible) {
      return;
    }
    if (key.name === "up" || key.name === "down" || key.name === "tab") {
      toggleSelection();
      return;
    }

    if (key.name === "y") {
      confirm(true);
      return;
    }

    if (key.name === "n" || key.name === "escape") {
      confirm(false);
      return;
    }

    if (key.name === "return" || key.name === "linefeed") {
      confirm(selectedIndex() === 0);
    }
  });

  const renderOption = (label: string, isSelected: boolean) =>
    isSelected ? (
      <text fg={selectionStyle.fg} bg={selectionStyle.bg}>
        {padLine(label, terminal().columns)}
      </text>
    ) : (
      <text>{label}</text>
    );

  return (
    <box flexDirection="column">
      <text>{message}</text>
      <box flexDirection="column">
        {renderOption(yesLabel, selectedIndex() === 0)}
        {renderOption(noLabel, selectedIndex() === 1)}
      </box>
    </box>
  );
}
