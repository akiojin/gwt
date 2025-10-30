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
}

/**
 * Select component - custom implementation with no looping
 * Cursor stops at top and bottom instead of wrapping around
 */
export function Select<T extends SelectItem = SelectItem>({
  items,
  onSelect,
  limit,
  initialIndex = 0,
  disabled = false,
  renderIndicator,
}: SelectProps<T>) {
  const [selectedIndex, setSelectedIndex] = useState(initialIndex);
  const [offset, setOffset] = useState(0);

  useEffect(() => {
    if (items.length === 0) {
      setSelectedIndex(0);
      setOffset(0);
      return;
    }

    setSelectedIndex((current) => {
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
      setSelectedIndex((current) => {
        const newIndex = Math.max(0, current - 1);

        // Adjust offset if needed for scrolling
        if (limit && newIndex < offset) {
          setOffset(newIndex);
        }

        return newIndex;
      });
    } else if (key.downArrow || input === 'j') {
      // Move down but don't loop - stop at last item
      setSelectedIndex((current) => {
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
}
