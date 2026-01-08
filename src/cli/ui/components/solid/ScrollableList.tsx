/** @jsxImportSource @opentui/solid */
import type { JSX } from "solid-js";

export interface ScrollableListProps {
  children: JSX.Element;
  maxHeight?: number;
}

/**
 * ScrollableList component - container for scrollable content
 * Note: actual scrolling is handled by list components using their own limits
 */
export function ScrollableList({ children, maxHeight }: ScrollableListProps) {
  return (
    <box
      flexDirection="column"
      overflow="hidden"
      {...(maxHeight !== undefined && { height: maxHeight })}
    >
      {children}
    </box>
  );
}
