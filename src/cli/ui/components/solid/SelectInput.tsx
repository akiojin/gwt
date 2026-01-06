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
  /** 明示的に高さを指定。undefined の場合はアイテム数から計算 */
  height?: number;
}

export function SelectInput(props: SelectInputProps) {
  // Solid.js ではpropsを分割代入するとreactivityが失われるため、propsを直接参照
  const options = () =>
    props.items.map((item) => ({
      name: item.label,
      description: item.description ?? "",
      value: item.value,
    })) as SelectOption[];

  // 高さを計算: 明示的指定がなければアイテム数から計算
  // showDescription が有効の場合は各アイテムに2行使用
  const computedHeight = () => {
    if (props.height !== undefined) {
      return props.height;
    }
    const itemCount = props.items.length;
    const linesPerItem = props.showDescription ? 2 : 1;
    return itemCount * linesPerItem;
  };

  const handleSelect = (index: number, option: SelectOption | null) => {
    // T412: focused が false の間は選択を無視（キー伝播防止）
    if (props.focused === false) {
      return;
    }
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
    <box flexDirection="column">
      <select
        options={options()}
        height={computedHeight()}
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
