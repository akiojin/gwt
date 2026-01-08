/** @jsxImportSource @opentui/solid */
import { TextAttributes } from "@opentui/core";

export interface TextInputProps {
  value: string;
  onChange: (value: string) => void;
  onInput?: (value: string) => void;
  onSubmit?: (value: string) => void;
  placeholder?: string;
  label?: string;
  focused?: boolean;
  width?: number;
}

export function TextInput({
  value,
  onChange,
  onInput,
  onSubmit,
  placeholder = "",
  label,
  focused,
  width,
}: TextInputProps) {
  const inputWidth =
    width ?? Math.max(10, value.length + 1, placeholder.length + 1);
  const isFocused = focused ?? true;

  return (
    <box flexDirection="column">
      {label && (
        <box height={1}>
          <text attributes={TextAttributes.DIM}>{label}</text>
        </box>
      )}
      <box height={1}>
        <input
          value={value}
          onChange={onChange}
          placeholder={placeholder}
          focused={isFocused}
          width={inputWidth}
          {...(onInput ? { onInput } : {})}
          {...(onSubmit ? { onSubmit } : {})}
        />
      </box>
    </box>
  );
}
