import { useState, useEffect } from "react";

export interface TerminalSize {
  rows: number;
  columns: number;
}

/**
 * Hook to get current terminal size and listen for resize events
 */
export function useTerminalSize(): TerminalSize {
  const [size, setSize] = useState<TerminalSize>(() => ({
    rows: process.stdout.rows || 24,
    columns: process.stdout.columns || 80,
  }));

  useEffect(() => {
    const handleResize = () => {
      setSize({
        rows: process.stdout.rows || 24,
        columns: process.stdout.columns || 80,
      });
    };

    process.stdout.on("resize", handleResize);

    return () => {
      process.stdout.removeListener("resize", handleResize);
    };
  }, []);

  return size;
}
