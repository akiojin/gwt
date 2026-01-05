/** @jsxImportSource @opentui/solid */
import { TextAttributes } from "@opentui/core";

export interface SearchInputProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit?: (value: string) => void;
  placeholder?: string;
  count?: { filtered: number; total: number };
  focused?: boolean;
}

export function SearchInput({
  value,
  onChange,
  onSubmit,
  placeholder = "Search...",
  count,
  focused,
}: SearchInputProps) {
  const inputWidth = Math.max(10, value.length, placeholder.length);
  const isFocused = focused ?? true;

  return (
    <box flexDirection="row">
      <text attributes={TextAttributes.DIM}>Search: </text>
      <input
        value={value}
        onChange={onChange}
        onSubmit={onSubmit}
        placeholder={placeholder}
        width={inputWidth}
        focused={isFocused}
      />
      {count && (
        <text attributes={TextAttributes.DIM}>
          {` ${count.filtered} / ${count.total}`}
        </text>
      )}
    </box>
  );
}
