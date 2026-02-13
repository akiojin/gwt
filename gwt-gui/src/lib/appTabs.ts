import type { Tab } from "./types";

const DEFAULT_APP_TABS: Tab[] = [{ id: "agentMode", label: "Agent Mode", type: "agentMode" }];

export function defaultAppTabs(): Tab[] {
  return DEFAULT_APP_TABS.map((tab) => ({ ...tab }));
}

export function shouldAllowRestoredActiveTab(activeTabId: string): boolean {
  return activeTabId === "agentMode";
}
