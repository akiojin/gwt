import type { SelectOption } from "@opentui/core";

export interface SelectInputOption extends SelectOption {
  name: string;
  description: string;
  value?: string;
}

export interface SelectInputProps {
  options: SelectInputOption[];
  selectedIndex?: number;
  showDescription?: boolean;
  wrapSelection?: boolean;
  showScrollIndicator?: boolean;
  focused?: boolean;
  onChange?: (index: number, option: SelectInputOption | null) => void;
  onSelect?: (option: SelectInputOption | null) => void;
}

export function SelectInput({
  options,
  selectedIndex,
  showDescription,
  wrapSelection,
  showScrollIndicator,
  focused,
  onChange,
  onSelect,
}: SelectInputProps) {
  return (
    <select
      options={options}
      selectedIndex={selectedIndex}
      showDescription={showDescription}
      wrapSelection={wrapSelection}
      showScrollIndicator={showScrollIndicator}
      focused={focused}
      onChange={(index, option) => {
        onChange?.(index, option as SelectInputOption | null);
      }}
      onSelect={(index, option) => {
        onSelect?.(option as SelectInputOption | null);
      }}
    />
  );
}
