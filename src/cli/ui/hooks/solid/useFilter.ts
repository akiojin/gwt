import { createMemo, createSignal, type Accessor } from "solid-js";

type MaybeAccessor<T> = T | (() => T);

type StringSetter = (value: string | ((prev: string) => string)) => void;

type BooleanSetter = (value: boolean | ((prev: boolean) => boolean)) => void;

export interface UseFilterOptions<T> {
  items: MaybeAccessor<T[]>;
  initialQuery?: string;
  initialActive?: boolean;
  normalizeQuery?: (query: string) => string;
  filter?: (item: T, normalizedQuery: string) => boolean;
}

export interface UseFilterResult<T> {
  query: Accessor<string>;
  setQuery: StringSetter;
  normalizedQuery: Accessor<string>;
  isActive: Accessor<boolean>;
  setActive: BooleanSetter;
  filteredItems: Accessor<T[]>;
  clear: () => void;
  activate: () => void;
  deactivate: () => void;
  toggle: () => void;
}

const resolveMaybe = <T>(value: MaybeAccessor<T>): T =>
  typeof value === "function" ? (value as () => T)() : value;

const defaultNormalizeQuery = (query: string): string =>
  query.trim().toLowerCase();

export function useFilter<T>(options: UseFilterOptions<T>): UseFilterResult<T> {
  const [query, setQueryInternal] = createSignal(options.initialQuery ?? "");
  const [isActive, setActiveInternal] = createSignal(
    options.initialActive ?? false,
  );

  const normalizedQuery = createMemo(() => {
    const normalize = options.normalizeQuery ?? defaultNormalizeQuery;
    return normalize(query());
  });

  const filteredItems = createMemo(() => {
    const items = resolveMaybe(options.items) ?? [];
    const currentQuery = normalizedQuery();
    const filter = options.filter;

    if (!filter || !currentQuery) {
      return items;
    }

    return items.filter((item) => filter(item, currentQuery));
  });

  const setQuery: StringSetter = (value) => {
    const next = typeof value === "function" ? value(query()) : value;
    setQueryInternal(next);
  };

  const setActive: BooleanSetter = (value) => {
    const next = typeof value === "function" ? value(isActive()) : value;
    setActiveInternal(next);
  };

  const clear = () => setQuery("");
  const activate = () => setActive(true);
  const deactivate = () => setActive(false);
  const toggle = () => setActive((prev) => !prev);

  return {
    query,
    setQuery,
    normalizedQuery,
    isActive,
    setActive,
    filteredItems,
    clear,
    activate,
    deactivate,
    toggle,
  };
}
