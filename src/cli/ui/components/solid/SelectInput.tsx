/** @jsxImportSource @opentui/solid */
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

export function SelectInput(props: SelectInputProps) {
  // Solid.js ではpropsを分割代入するとreactivityが失われるため、propsを直接参照
  const options = () =>
    props.items.map((item) => ({
      name: item.label,
      description: item.description ?? "",
      value: item.value,
    })) as SelectOption[];

  const handleSelect = (index: number, option: SelectOption | null) => {
    if (index < 0) {
      return;
    }
    const item = props.items[index];
    if (!item || !option) {
      return;
    }
    props.onSelect?.(item);
  };

  const handleChange = (index: number, option: SelectOption | null) => {
    if (index < 0 || !option) {
      props.onChange?.(null);
      return;
    }
    props.onChange?.(props.items[index] ?? null);
  };

  return (
    <box flexDirection="column" height={1}>
      <select
        options={options()}
        {...(props.selectedIndex !== undefined && {
          selectedIndex: props.selectedIndex,
        })}
        focused={props.focused ?? false}
        showDescription={props.showDescription ?? false}
        wrapSelection={props.wrapSelection ?? false}
        onSelect={handleSelect}
        onChange={handleChange}
      />
    </box>
  );
}
