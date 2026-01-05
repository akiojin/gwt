/** @jsxImportSource @opentui/solid */
import { TextAttributes } from "@opentui/core";
import { useKeyboard } from "@opentui/solid";
import { createSignal } from "solid-js";

export interface ConfirmScreenProps {
  message: string;
  onConfirm: (confirmed: boolean) => void;
  yesLabel?: string;
  noLabel?: string;
  defaultNo?: boolean;
}

export function ConfirmScreen({
  message,
  onConfirm,
  yesLabel = "Yes",
  noLabel = "No",
  defaultNo = false,
}: ConfirmScreenProps) {
  const [selectedIndex, setSelectedIndex] = createSignal(defaultNo ? 1 : 0);

  const confirm = (confirmed: boolean) => {
    onConfirm(confirmed);
  };

  const toggleSelection = () => {
    setSelectedIndex((current) => (current === 0 ? 1 : 0));
  };

  useKeyboard((key) => {
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

  const renderOption = (label: string, isSelected: boolean) => (
    <text
      {...(isSelected ? { fg: "cyan", attributes: TextAttributes.BOLD } : {})}
    >
      {`${isSelected ? ">" : " "} ${label}`}
    </text>
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
