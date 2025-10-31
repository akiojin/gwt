import React, { useEffect, useState } from 'react';
import { Box, Text, useInput } from 'ink';

export interface SelectItem {
  label: string;
  value: string;
}

export interface SelectProps<T extends SelectItem = SelectItem> {
  items: T[];
  onSelect: (item: T) => void;
  limit?: number;
  initialIndex?: number;
  disabled?: boolean;
  renderIndicator?: (item: T, isSelected: boolean) => React.ReactNode;
  // Optional controlled component props for cursor position
  selectedIndex?: number;
  onSelectedIndexChange?: (index: number) => void;
}

/**
 * Custom comparison function for React.memo
 * Compares items array by content (value and label) instead of reference
 */
function arePropsEqual<T extends SelectItem = SelectItem>(
  prevProps: SelectProps<T>,
  nextProps: SelectProps<T>
): boolean {
  // Check if non-array props are the same
  if (
    prevProps.limit !== nextProps.limit ||
    prevProps.disabled !== nextProps.disabled ||
    prevProps.initialIndex !== nextProps.initialIndex ||
    prevProps.selectedIndex !== nextProps.selectedIndex ||
    prevProps.onSelect !== nextProps.onSelect ||
    prevProps.onSelectedIndexChange !== nextProps.onSelectedIndexChange ||
    prevProps.renderIndicator !== nextProps.renderIndicator
  ) {
    return false;
  }

  // Check if items arrays have the same length
  if (prevProps.items.length !== nextProps.items.length) {
    return false;
  }

  // Compare items by content (value and label)
  for (let i = 0; i < prevProps.items.length; i++) {
    const prevItem = prevProps.items[i];
    const nextItem = nextProps.items[i];

    if (!prevItem || !nextItem) {
      return false;
    }

    if (prevItem.value !== nextItem.value || prevItem.label !== nextItem.label) {
      return false;
    }
  }

  // All props are equal
  return true;
}

/**
 * Select component - custom implementation with no looping
 * Cursor stops at top and bottom instead of wrapping around
 * Wrapped with React.memo for performance optimization
 */
const SelectComponent = <T extends SelectItem = SelectItem,>({
  items,
  onSelect,
  limit,
  initialIndex = 0,
  disabled = false,
  renderIndicator,
  selectedIndex: externalSelectedIndex,
  onSelectedIndexChange,
}: SelectProps<T>) => {
  // Support both controlled and uncontrolled modes
  const [internalSelectedIndex, setInternalSelectedIndex] = useState(initialIndex);
  const [offset, setOffset] = useState(0);

  // Use external selectedIndex if provided (controlled mode), otherwise use internal state
  const isControlled = externalSelectedIndex !== undefined;
  const selectedIndex = isControlled ? externalSelectedIndex : internalSelectedIndex;

  const updateSelectedIndex = (value: number | ((prev: number) => number)) => {
    const newIndex = typeof value === 'function' ? value(selectedIndex) : value;

    if (!isControlled) {
      setInternalSelectedIndex(newIndex);
    }

    if (onSelectedIndexChange) {
      onSelectedIndexChange(newIndex);
    }
  };

  useEffect(() => {
    if (items.length === 0) {
      updateSelectedIndex(0);
      setOffset(0);
      return;
    }

    updateSelectedIndex((current) => {
      const clamped = Math.min(current, items.length - 1);
      return clamped < 0 ? 0 : clamped;
    });

    if (limit) {
      setOffset((current) => {
        if (current <= items.length - limit) {
          return current < 0 ? 0 : current;
        }
        const newOffset = Math.max(0, items.length - limit);
        return newOffset;
      });
    }
  }, [items, limit]);

  useInput((input, key) => {
    if (disabled) {
      return;
    }

    // Only handle navigation and selection keys
    // Let other keys (q, m, n, c, etc.) propagate to parent components
    if (key.upArrow || input === 'k') {
      // Move up but don't loop - stop at 0
      updateSelectedIndex((current) => {
        const newIndex = Math.max(0, current - 1);

        // Adjust offset if needed for scrolling
        if (limit && newIndex < offset) {
          setOffset(newIndex);
        }

        return newIndex;
      });
    } else if (key.downArrow || input === 'j') {
      // Move down but don't loop - stop at last item
      updateSelectedIndex((current) => {
        const newIndex = Math.min(items.length - 1, current + 1);

        // Adjust offset if needed for scrolling
        if (limit && newIndex >= offset + limit) {
          setOffset(newIndex - limit + 1);
        }

        return newIndex;
      });
    } else if (key.return) {
      // Select current item
      const selectedItem = items[selectedIndex];
      if (selectedItem && !disabled) {
        onSelect(selectedItem);
      }
    }
    // All other keys are ignored and will propagate to parent components
  });

  // Determine visible items based on limit
  const visibleItems = limit
    ? items.slice(offset, offset + limit)
    : items;
  const visibleStartIndex = limit ? offset : 0;

  return (
    <Box flexDirection="column">
      {visibleItems.map((item, index) => {
        const actualIndex = visibleStartIndex + index;
        const isSelected = actualIndex === selectedIndex;

        const indicatorElement = renderIndicator
          ? renderIndicator(item, isSelected)
          : isSelected
            ? <Text color="cyan">›</Text>
            : <Text> </Text>;

        return (
          <Box key={item.value}>
            <Box marginRight={1}>
              {indicatorElement ?? <Text> </Text>}
            </Box>
            {isSelected ? (
              <Text color="cyan">{item.label}</Text>
            ) : (
              <Text>{item.label}</Text>
            )}
          </Box>
        );
      })}
    </Box>
  );
};

/**
 * Export memoized Select component
 */
export const Select = React.memo(SelectComponent, arePropsEqual) as typeof SelectComponent;
