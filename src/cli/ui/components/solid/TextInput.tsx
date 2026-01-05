import { TextAttributes } from "@opentui/core";

export interface TextInputProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit?: (value: string) => void;
  placeholder?: string;
  label?: string;
  focused?: boolean;
  width?: number;
}

export function TextInput({
  value,
  onChange,
  onSubmit,
  placeholder = "",
  label,
  focused,
  width,
}: TextInputProps) {
  const inputWidth =
    width ?? Math.max(10, value.length, placeholder.length + 1);
  const isFocused = focused ?? true;

  return (
    <box flexDirection="column">
      {label && <text attributes={TextAttributes.DIM}>{label}</text>}
      <input
        value={value}
        onChange={onChange}
        onSubmit={onSubmit}
        placeholder={placeholder}
        focused={isFocused}
        width={inputWidth}
      />
    </box>
  );
}
