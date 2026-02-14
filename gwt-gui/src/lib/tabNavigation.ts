import type { Tab } from "./types";

/**
 * Returns the ID of the next tab in display order, or null if at the end.
 */
export function getNextTabId(tabs: Tab[], activeTabId: string): string | null {
  const index = tabs.findIndex((t) => t.id === activeTabId);
  if (index < 0 || index >= tabs.length - 1) return null;
  return tabs[index + 1].id;
}

/**
 * Returns the ID of the previous tab in display order, or null if at the start.
 */
export function getPreviousTabId(tabs: Tab[], activeTabId: string): string | null {
  const index = tabs.findIndex((t) => t.id === activeTabId);
  if (index <= 0) return null;
  return tabs[index - 1].id;
}
