import { createMemo, type Accessor } from "solid-js";
import { useTerminalDimensions } from "@opentui/solid";

export interface TerminalSize {
  rows: number;
  columns: number;
}

/**
 * Solid hook to access terminal size (rows/columns) via OpenTUI.
 */
export function useTerminalSize(): Accessor<TerminalSize> {
  const terminal = useTerminalDimensions();

  return createMemo(() => {
    const { width, height } = terminal();
    return {
      rows: height || 24,
      columns: width || 80,
    };
  });
}
