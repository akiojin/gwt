import type { Tab } from "./types";

const DEFAULT_APP_TABS: Tab[] = [{ id: "agentMode", label: "Agent Mode", type: "agentMode" }];
export type TabDropPosition = "before" | "after";

export function defaultAppTabs(): Tab[] {
  return DEFAULT_APP_TABS.map((tab) => ({ ...tab }));
}

export function shouldAllowRestoredActiveTab(activeTabId: string): boolean {
  return activeTabId === "agentMode";
}

export function reorderTabsByDrop(
  tabs: Tab[],
  dragTabId: string,
  overTabId: string,
  position: TabDropPosition,
): Tab[] {
  if (dragTabId === overTabId) return tabs;

  const dragIndex = tabs.findIndex((tab) => tab.id === dragTabId);
  const overIndex = tabs.findIndex((tab) => tab.id === overTabId);
  if (dragIndex < 0 || overIndex < 0) return tabs;

  const nextTabs = [...tabs];
  const [dragged] = nextTabs.splice(dragIndex, 1);
  if (!dragged) return tabs;

  const targetIndex = nextTabs.findIndex((tab) => tab.id === overTabId);
  if (targetIndex < 0) return tabs;

  const insertIndex = position === "before" ? targetIndex : targetIndex + 1;
  nextTabs.splice(insertIndex, 0, dragged);

  const unchanged =
    nextTabs.length === tabs.length && nextTabs.every((tab, idx) => tab.id === tabs[idx]?.id);
  return unchanged ? tabs : nextTabs;
}
