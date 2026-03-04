import type { Tab } from "./types";

export type WindowMenuTabType = "agent" | "terminal";

export interface WindowMenuTabEntry {
  id: string;
  label: string;
  tab_type: WindowMenuTabType;
}

const SIGNATURE_ENTRY_SEPARATOR = "\u0001";
const SIGNATURE_FIELD_SEPARATOR = "\u0000";

export function buildWindowMenuVisibleTabs(tabs: Tab[]): WindowMenuTabEntry[] {
  return tabs
    .filter((tab): tab is Tab & { type: WindowMenuTabType } =>
      tab.type === "agent" || tab.type === "terminal")
    .map((tab) => ({
      id: tab.id,
      label: tab.label,
      tab_type: tab.type,
    }));
}

export function buildWindowMenuTabsSignature(tabs: WindowMenuTabEntry[]): string {
  return tabs
    .map((tab) =>
      [tab.id, tab.label, tab.tab_type].join(SIGNATURE_FIELD_SEPARATOR))
    .join(SIGNATURE_ENTRY_SEPARATOR);
}

export function resolveActiveWindowMenuTabId(
  visibleTabs: WindowMenuTabEntry[],
  activeTabId: string,
): string | null {
  return visibleTabs.some((tab) => tab.id === activeTabId) ? activeTabId : null;
}

