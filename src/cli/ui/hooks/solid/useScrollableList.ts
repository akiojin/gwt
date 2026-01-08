import {
  createEffect,
  createMemo,
  createSignal,
  type Accessor,
} from "solid-js";

type MaybeAccessor<T> = T | (() => T);

type IndexSetter = (value: number | ((prev: number) => number)) => void;

export interface UseScrollableListOptions<T> {
  items: MaybeAccessor<T[]>;
  visibleCount: MaybeAccessor<number>;
  initialIndex?: number;
  initialOffset?: number;
  wrapSelection?: boolean;
}

export interface UseScrollableListResult<T> {
  selectedIndex: Accessor<number>;
  setSelectedIndex: IndexSetter;
  scrollOffset: Accessor<number>;
  setScrollOffset: IndexSetter;
  visibleItems: Accessor<T[]>;
  moveUp: () => void;
  moveDown: () => void;
  pageUp: () => void;
  pageDown: () => void;
  goToTop: () => void;
  goToBottom: () => void;
}

const resolveMaybe = <T>(value: MaybeAccessor<T>): T =>
  typeof value === "function" ? (value as () => T)() : value;

const clamp = (value: number, min: number, max: number): number =>
  Math.min(Math.max(value, min), max);

const wrapIndex = (value: number, total: number): number => {
  if (total <= 0) {
    return 0;
  }
  const wrapped = value % total;
  return wrapped < 0 ? wrapped + total : wrapped;
};

export function useScrollableList<T>(
  options: UseScrollableListOptions<T>,
): UseScrollableListResult<T> {
  const [selectedIndex, setSelectedIndexInternal] = createSignal(
    options.initialIndex ?? 0,
  );
  const [scrollOffset, setScrollOffsetInternal] = createSignal(
    options.initialOffset ?? 0,
  );

  const items = createMemo(() => resolveMaybe(options.items) ?? []);
  const total = createMemo(() => items().length);
  const limit = createMemo(() =>
    Math.max(1, resolveMaybe(options.visibleCount) ?? 1),
  );
  const maxOffset = createMemo(() => Math.max(0, total() - limit()));

  const setSelectedIndex: IndexSetter = (value) => {
    const next = typeof value === "function" ? value(selectedIndex()) : value;
    const count = total();

    if (count === 0) {
      setSelectedIndexInternal(0);
      return;
    }

    if (options.wrapSelection) {
      setSelectedIndexInternal(wrapIndex(next, count));
      return;
    }

    setSelectedIndexInternal(clamp(next, 0, count - 1));
  };

  const setScrollOffset: IndexSetter = (value) => {
    const next = typeof value === "function" ? value(scrollOffset()) : value;
    setScrollOffsetInternal(clamp(next, 0, maxOffset()));
  };

  createEffect(() => {
    const count = total();

    if (count === 0) {
      setSelectedIndexInternal(0);
      setScrollOffsetInternal(0);
      return;
    }

    setSelectedIndexInternal((prev) => clamp(prev, 0, count - 1));
    setScrollOffsetInternal((prev) => clamp(prev, 0, maxOffset()));
  });

  createEffect(() => {
    const count = total();
    const visible = limit();
    const index = selectedIndex();

    setScrollOffsetInternal((prev) => {
      if (count === 0) {
        return 0;
      }

      let next = prev;
      if (index < prev) {
        next = index;
      } else if (index >= prev + visible) {
        next = index - visible + 1;
      }

      return clamp(next, 0, Math.max(0, count - visible));
    });
  });

  const visibleItems = createMemo(() => {
    const start = scrollOffset();
    const end = start + limit();
    return items().slice(start, end);
  });

  const moveBy = (delta: number) => {
    setSelectedIndex((prev) => prev + delta);
  };

  const pageBy = (delta: number) => {
    const step = Math.max(1, limit());
    setSelectedIndex((prev) => prev + delta * step);
  };

  return {
    selectedIndex,
    setSelectedIndex,
    scrollOffset,
    setScrollOffset,
    visibleItems,
    moveUp: () => moveBy(-1),
    moveDown: () => moveBy(1),
    pageUp: () => pageBy(-1),
    pageDown: () => pageBy(1),
    goToTop: () => setSelectedIndex(0),
    goToBottom: () => setSelectedIndex(total() - 1),
  };
}
