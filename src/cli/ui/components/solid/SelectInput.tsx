import type { SelectOption } from "@opentui/core";

export interface SelectInputItem {
  label: string;
  value: string;
  description?: string;
}

export interface SelectInputProps {
  items: SelectInputItem[];
  selectedIndex?: number;
  onSelect?: (item: SelectInputItem) => void;
  onChange?: (item: SelectInputItem | null) => void;
  focused?: boolean;
  showDescription?: boolean;
  wrapSelection?: boolean;
}

export function SelectInput({
  items,
  selectedIndex,
  onSelect,
  onChange,
  focused,
  showDescription = false,
  wrapSelection = false,
}: SelectInputProps) {
  const options: SelectOption[] = items.map((item) => ({
    name: item.label,
    description: item.description ?? "",
    value: item.value,
  }));

  const handleSelect = (index: number, option: SelectOption | null) => {
    if (index < 0) {
      return;
    }
    const item = items[index];
    if (!item || !option) {
      return;
    }
    onSelect?.(item);
  };

  const handleChange = (index: number, option: SelectOption | null) => {
    if (index < 0 || !option) {
      onChange?.(null);
      return;
    }
    onChange?.(items[index] ?? null);
  };

  return (
    <select
      options={options}
      {...(selectedIndex !== undefined && { selectedIndex })}
      focused={focused}
      showDescription={showDescription}
      wrapSelection={wrapSelection}
      onSelect={handleSelect}
      onChange={handleChange}
    />
  );
}
