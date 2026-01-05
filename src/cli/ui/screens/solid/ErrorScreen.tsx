/** @jsxImportSource @opentui/solid */
import { TextAttributes } from "@opentui/core";
import { useKeyboard } from "@opentui/solid";

export interface ErrorScreenProps {
  error: Error | string;
  hint?: string;
  onBack?: () => void;
}

const resolveMessage = (error: Error | string): string =>
  typeof error === "string" ? error : error.message;

export function ErrorScreen({ error, hint, onBack }: ErrorScreenProps) {
  useKeyboard((key) => {
    if (key.name === "escape" || key.name === "return") {
      onBack?.();
    }
  });

  const message = resolveMessage(error);

  return (
    <box flexDirection="column">
      <text
        fg="red"
        attributes={TextAttributes.BOLD}
      >{`Error: ${message}`}</text>
      {hint && <text>{hint}</text>}
      {onBack && <text fg="gray">Press Enter or Esc to go back</text>}
    </box>
  );
}
