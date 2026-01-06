import { createMemo, createSignal, type Accessor } from "solid-js";

type SelectionKey = string;

type SelectionSetter = (
  value: SelectionKey[] | ((prev: SelectionKey[]) => SelectionKey[]),
) => void;

export interface UseSelectionOptions {
  initialSelected?: SelectionKey[];
}

export interface UseSelectionResult {
  selected: Accessor<SelectionKey[]>;
  selectedSet: Accessor<Set<SelectionKey>>;
  setSelected: SelectionSetter;
  isSelected: (key: SelectionKey) => boolean;
  add: (key: SelectionKey) => void;
  remove: (key: SelectionKey) => void;
  toggle: (key: SelectionKey) => void;
  selectOnly: (key: SelectionKey) => void;
  clear: () => void;
}

const uniq = (values: SelectionKey[]): SelectionKey[] =>
  Array.from(new Set(values));

export function useSelection(
  options: UseSelectionOptions = {},
): UseSelectionResult {
  const [selected, setSelectedInternal] = createSignal(
    uniq(options.initialSelected ?? []),
  );

  const setSelected: SelectionSetter = (value) => {
    const next = typeof value === "function" ? value(selected()) : value;
    setSelectedInternal(uniq(next));
  };

  const selectedSet = createMemo(() => new Set(selected()));

  const isSelected = (key: SelectionKey) => selectedSet().has(key);

  const add = (key: SelectionKey) => {
    setSelected((prev) => (prev.includes(key) ? prev : [...prev, key]));
  };

  const remove = (key: SelectionKey) => {
    setSelected((prev) => prev.filter((item) => item !== key));
  };

  const toggle = (key: SelectionKey) => {
    setSelected((prev) =>
      prev.includes(key) ? prev.filter((item) => item !== key) : [...prev, key],
    );
  };

  const selectOnly = (key: SelectionKey) => {
    setSelected([key]);
  };

  const clear = () => {
    setSelected([]);
  };

  return {
    selected,
    selectedSet,
    setSelected,
    isSelected,
    add,
    remove,
    toggle,
    selectOnly,
    clear,
  };
}
