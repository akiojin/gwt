/** @jsxImportSource @opentui/solid */
import { useKeyboard } from "@opentui/solid";
import { TextInput } from "../../components/solid/TextInput.js";

export interface InputScreenProps {
  message: string;
  value: string;
  onChange: (value: string) => void;
  onSubmit: (value: string) => void;
  onCancel?: () => void;
  placeholder?: string;
  label?: string;
  helpVisible?: boolean;
}

export function InputScreen({
  message,
  value,
  onChange,
  onSubmit,
  onCancel,
  placeholder,
  label,
  helpVisible = false,
}: InputScreenProps) {
  const inputHeight = label ? 2 : 1;

  useKeyboard((key) => {
    if (helpVisible) {
      return;
    }
    if (key.name === "escape") {
      onCancel?.();
    }
  });

  return (
    <box flexDirection="column">
      <text>{message}</text>
      <box height={inputHeight}>
        <TextInput
          value={value}
          onChange={onChange}
          onSubmit={onSubmit}
          placeholder={placeholder}
          label={label}
          focused
        />
      </box>
    </box>
  );
}
