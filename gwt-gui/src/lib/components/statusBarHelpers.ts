/**
 * Render an ASCII progress bar with 8-char width.
 * Example: renderBar(50) → "[||||    ]"
 */
export function renderBar(pct: number): string {
  const filled = Math.round((pct / 100) * 8);
  const bar = "|".repeat(filled) + " ".repeat(8 - filled);
  return `[${bar}]`;
}

/**
 * Return a CSS color class based on usage percentage.
 * <70% → "ok", 70-89% → "warn", >=90% → "bad"
 */
export function usageColorClass(pct: number): string {
  if (pct >= 90) return "bad";
  if (pct >= 70) return "warn";
  return "ok";
}

/**
 * Format bytes to GB with 1 decimal place.
 * Example: formatMemory(8589934592) → "8.0"
 */
export function formatMemory(bytes: number): string {
  return (bytes / 1073741824).toFixed(1);
}
