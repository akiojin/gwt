import React from 'react';
import SelectInput from 'ink-select-input';

export interface SelectItem {
  label: string;
  value: string;
}

export interface SelectProps<T extends SelectItem = SelectItem> {
  items: T[];
  onSelect: (item: T) => void;
  limit?: number;
  initialIndex?: number;
}

/**
 * Select component - wrapper around ink-select-input
 */
export function Select<T extends SelectItem = SelectItem>({
  items,
  onSelect,
  limit,
  initialIndex,
}: SelectProps<T>) {
  const handleSelect = (item: { label: string; value: string }) => {
    // ink-select-input returns an item, but we know it's one of our items
    const selectedItem = items.find((i) => i.value === item.value);
    if (selectedItem) {
      onSelect(selectedItem);
    }
  };

  return (
    <SelectInput
      items={items}
      onSelect={handleSelect}
      {...(limit !== undefined && { limit })}
      {...(initialIndex !== undefined && { initialIndex })}
    />
  );
}
