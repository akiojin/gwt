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
}

export function InputScreen({
  message,
  value,
  onChange,
  onSubmit,
  onCancel,
  placeholder,
  label,
}: InputScreenProps) {
  useKeyboard((key) => {
    if (key.name === "escape") {
      onCancel?.();
    }
  });

  return (
    <box flexDirection="column">
      <text>{message}</text>
      <TextInput
        value={value}
        onChange={onChange}
        onSubmit={onSubmit}
        placeholder={placeholder}
        label={label}
        focused
      />
    </box>
  );
}
