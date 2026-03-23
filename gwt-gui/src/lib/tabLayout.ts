import type { Tab } from "./types";

export type TabDropPosition = "before" | "after";
export type TabSplitDirection = "left" | "right" | "up" | "down";
export type TabSplitAxis = "horizontal" | "vertical";

export interface TabGroupState {
  id: string;
  tabIds: string[];
  activeTabId: string | null;
}

export type TabLayoutNode =
  | {
      type: "group";
      groupId: string;
    }
  | {
      type: "split";
      id: string;
      axis: TabSplitAxis;
      sizes: [number, number];
      children: [TabLayoutNode, TabLayoutNode];
    };

export interface TabLayoutState {
  groups: Record<string, TabGroupState>;
  root: TabLayoutNode;
  activeGroupId: string;
}

export type TabLayoutDropTarget =
  | {
      kind: "tab";
      groupId: string;
      tabId: string;
      position: TabDropPosition;
    }
  | {
      kind: "group";
      groupId: string;
    }
  | {
      kind: "split";
      groupId: string;
      direction: TabSplitDirection;
    };

let syntheticIdCounter = 0;

function createSyntheticId(prefix: "group" | "split"): string {
  syntheticIdCounter += 1;
  return `${prefix}-${syntheticIdCounter}`;
}

function cloneGroups(
  groups: Record<string, TabGroupState>,
): Record<string, TabGroupState> {
  const next: Record<string, TabGroupState> = {};
  for (const [id, group] of Object.entries(groups)) {
    next[id] = {
      id: group.id,
      tabIds: [...group.tabIds],
      activeTabId: group.activeTabId,
    };
  }
  return next;
}

function axisForDirection(direction: TabSplitDirection): TabSplitAxis {
  return direction === "left" || direction === "right"
    ? "horizontal"
    : "vertical";
}

function splitChildrenForDirection(
  direction: TabSplitDirection,
  groupNode: TabLayoutNode,
  newGroupNode: TabLayoutNode,
): [TabLayoutNode, TabLayoutNode] {
  if (direction === "left" || direction === "up") {
    return [newGroupNode, groupNode];
  }
  return [groupNode, newGroupNode];
}

function replaceGroupNode(
  node: TabLayoutNode,
  groupId: string,
  replacement: TabLayoutNode,
): TabLayoutNode {
  if (node.type === "group") {
    return node.groupId === groupId ? replacement : node;
  }

  const [left, right] = node.children;
  const nextLeft = replaceGroupNode(left, groupId, replacement);
  const nextRight = replaceGroupNode(right, groupId, replacement);
  if (nextLeft === left && nextRight === right) {
    return node;
  }
  return {
    ...node,
    children: [nextLeft, nextRight],
  };
}

function removeGroupNode(node: TabLayoutNode, groupId: string): TabLayoutNode | null {
  if (node.type === "group") {
    return node.groupId === groupId ? null : node;
  }

  const [left, right] = node.children;
  const nextLeft = removeGroupNode(left, groupId);
  const nextRight = removeGroupNode(right, groupId);

  if (!nextLeft && !nextRight) return null;
  if (!nextLeft) return nextRight;
  if (!nextRight) return nextLeft;
  if (nextLeft === left && nextRight === right) return node;
  return {
    ...node,
    children: [nextLeft, nextRight],
  };
}

function collectGroupIds(node: TabLayoutNode): string[] {
  if (node.type === "group") return [node.groupId];
  return [...collectGroupIds(node.children[0]), ...collectGroupIds(node.children[1])];
}

function pickFallbackActiveTab(
  tabIds: string[],
  removedTabId: string,
): string | null {
  if (tabIds.length === 0) return null;
  const removedIndex = tabIds.indexOf(removedTabId);
  if (removedIndex >= 0) {
    return tabIds[Math.min(removedIndex, tabIds.length - 1)] ?? null;
  }
  return tabIds[0] ?? null;
}

function sanitizeActiveGroupId(
  root: TabLayoutNode,
  activeGroupId: string | null,
): string {
  const groupIds = collectGroupIds(root);
  if (activeGroupId && groupIds.includes(activeGroupId)) {
    return activeGroupId;
  }
  return groupIds[0] ?? "group-unknown";
}

function sanitizeRuntimeRoot(
  node: TabLayoutNode,
  knownGroupIds: Set<string>,
): TabLayoutNode | null {
  if (node.type === "group") {
    return knownGroupIds.has(node.groupId) ? node : null;
  }

  const first = sanitizeRuntimeRoot(node.children[0], knownGroupIds);
  const second = sanitizeRuntimeRoot(node.children[1], knownGroupIds);

  if (!first && !second) return null;
  if (!first) return second;
  if (!second) return first;

  const [primary, secondary] = node.sizes;
  const normalizedPrimary =
    Number.isFinite(primary) && primary > 0 && primary < 1 ? primary : 0.5;

  return {
    ...node,
    sizes: [normalizedPrimary, 1 - normalizedPrimary],
    children: [first, second],
  };
}

function dedupeTabIds(tabIds: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const tabId of tabIds) {
    if (!tabId || seen.has(tabId)) continue;
    seen.add(tabId);
    out.push(tabId);
  }
  return out;
}

function collapseToSingleGroup(
  groups: Record<string, TabGroupState>,
  preferredGroupId: string | null,
  preferredActiveTabId: string | null,
  orderedTabIds: string[],
): TabLayoutState {
  const groupIds = Object.keys(groups);
  if (groupIds.length === 0) {
    return createInitialTabLayout([], null);
  }

  const targetGroupId =
    (preferredGroupId && groups[preferredGroupId] && preferredGroupId) ||
    groupIds[0] ||
    "group-unknown";
  const targetGroup = groups[targetGroupId];
  const fallbackOrder = dedupeTabIds(
    orderedTabIds.length > 0
      ? orderedTabIds
      : groupIds.flatMap((groupId) => groups[groupId]?.tabIds ?? []),
  );
  const activeTabId =
    preferredActiveTabId && fallbackOrder.includes(preferredActiveTabId)
      ? preferredActiveTabId
      : targetGroup?.activeTabId && fallbackOrder.includes(targetGroup.activeTabId)
        ? targetGroup.activeTabId
        : (fallbackOrder[0] ?? null);

  return {
    groups: {
      [targetGroupId]: {
        id: targetGroupId,
        tabIds: fallbackOrder,
        activeTabId,
      },
    },
    root: {
      type: "group",
      groupId: targetGroupId,
    },
    activeGroupId: targetGroupId,
  };
}

export function createInitialTabLayout(
  tabs: Pick<Tab, "id">[],
  activeTabId: string | null,
): TabLayoutState {
  const groupId = createSyntheticId("group");
  const tabIds = tabs.map((tab) => tab.id);
  return {
    groups: {
      [groupId]: {
        id: groupId,
        tabIds,
        activeTabId: activeTabId && tabIds.includes(activeTabId) ? activeTabId : (tabIds[0] ?? null),
      },
    },
    root: {
      type: "group",
      groupId,
    },
    activeGroupId: groupId,
  };
}

export function normalizeTabLayoutState(
  layout: TabLayoutState,
  preferredActiveTabIdOverride: string | null = null,
): TabLayoutState {
  const normalizedGroups: Record<string, TabGroupState> = {};

  for (const [groupId, group] of Object.entries(layout.groups)) {
    const tabIds = dedupeTabIds(group.tabIds);
    if (tabIds.length === 0) continue;
    normalizedGroups[groupId] = {
      id: group.id,
      tabIds,
      activeTabId:
        group.activeTabId && tabIds.includes(group.activeTabId)
          ? group.activeTabId
          : (tabIds[0] ?? null),
    };
  }

  const groupIds = Object.keys(normalizedGroups);
  if (groupIds.length === 0) {
    return createInitialTabLayout([], null);
  }

  const knownGroupIds = new Set(groupIds);
  const sanitizedRoot = sanitizeRuntimeRoot(layout.root, knownGroupIds);
  const preferredActiveTabId =
    preferredActiveTabIdOverride ??
    normalizedGroups[layout.activeGroupId]?.activeTabId ??
    normalizedGroups[groupIds[0]]?.activeTabId ??
    null;

  if (!sanitizedRoot) {
    return collapseToSingleGroup(
      normalizedGroups,
      layout.activeGroupId,
      preferredActiveTabId,
      groupIds.flatMap((groupId) => normalizedGroups[groupId]?.tabIds ?? []),
    );
  }

  const renderedGroupIds = collectGroupIds(sanitizedRoot);
  const rootOrderedTabIds = dedupeTabIds(
    renderedGroupIds.flatMap((groupId) => normalizedGroups[groupId]?.tabIds ?? []),
  );

  if (renderedGroupIds.length !== groupIds.length) {
    return collapseToSingleGroup(
      normalizedGroups,
      renderedGroupIds[0] ?? layout.activeGroupId,
      preferredActiveTabId,
      [
        ...rootOrderedTabIds,
        ...groupIds.flatMap((groupId) => normalizedGroups[groupId]?.tabIds ?? []),
      ],
    );
  }

  if (renderedGroupIds.length === 1) {
    const groupId = renderedGroupIds[0] ?? groupIds[0] ?? "group-unknown";
    return {
      groups: {
        [groupId]: normalizedGroups[groupId],
      },
      root: {
        type: "group",
        groupId,
      },
      activeGroupId: groupId,
    };
  }

  return {
    groups: normalizedGroups,
    root: sanitizedRoot,
    activeGroupId: sanitizeActiveGroupId(sanitizedRoot, layout.activeGroupId),
  };
}

export function getGroupForTab(
  layout: TabLayoutState,
  tabId: string,
): TabGroupState | null {
  return (
    Object.values(layout.groups).find((group) => group.tabIds.includes(tabId)) ??
    null
  );
}

export function canSplitTab(
  layout: TabLayoutState,
  tabId: string,
): boolean {
  const group = getGroupForTab(layout, tabId);
  return (group?.tabIds.length ?? 0) > 1;
}

export function setActiveGroup(
  layout: TabLayoutState,
  groupId: string,
): TabLayoutState {
  if (!layout.groups[groupId] || layout.activeGroupId === groupId) {
    return layout;
  }
  return {
    ...layout,
    activeGroupId: groupId,
  };
}

export function setActiveTabInGroup(
  layout: TabLayoutState,
  groupId: string,
  tabId: string,
): TabLayoutState {
  const group = layout.groups[groupId];
  if (!group || !group.tabIds.includes(tabId) || group.activeTabId === tabId) {
    return layout;
  }
  return {
    ...layout,
    activeGroupId: groupId,
    groups: {
      ...layout.groups,
      [groupId]: {
        ...group,
        activeTabId: tabId,
      },
    },
  };
}

export function addTabToActiveGroup(
  layout: TabLayoutState,
  tabId: string,
): TabLayoutState {
  const group = layout.groups[layout.activeGroupId];
  if (!group || group.tabIds.includes(tabId)) return layout;
  return {
    ...layout,
    groups: {
      ...layout.groups,
      [group.id]: {
        ...group,
        tabIds: [...group.tabIds, tabId],
        activeTabId: tabId,
      },
    },
  };
}

export function reorderTabsInGroup(
  layout: TabLayoutState,
  groupId: string,
  dragTabId: string,
  overTabId: string,
  position: TabDropPosition,
): TabLayoutState {
  const group = layout.groups[groupId];
  if (!group) return layout;

  const dragIndex = group.tabIds.indexOf(dragTabId);
  const overIndex = group.tabIds.indexOf(overTabId);
  if (dragIndex < 0 || overIndex < 0 || dragIndex === overIndex) {
    return layout;
  }

  const nextTabIds = [...group.tabIds];
  const [dragged] = nextTabIds.splice(dragIndex, 1);
  if (!dragged) return layout;
  const targetIndex = nextTabIds.indexOf(overTabId);
  if (targetIndex < 0) return layout;
  const insertIndex = position === "before" ? targetIndex : targetIndex + 1;
  nextTabIds.splice(insertIndex, 0, dragged);

  return {
    ...layout,
    groups: {
      ...layout.groups,
      [groupId]: {
        ...group,
        tabIds: nextTabIds,
      },
    },
  };
}

function removeTabFromGroupState(
  layout: TabLayoutState,
  groupId: string,
  tabId: string,
): TabLayoutState {
  const group = layout.groups[groupId];
  if (!group || !group.tabIds.includes(tabId)) return layout;

  const nextGroups = cloneGroups(layout.groups);
  const nextGroup = nextGroups[groupId];
  if (!nextGroup) return layout;
  nextGroup.tabIds = nextGroup.tabIds.filter((candidate) => candidate !== tabId);
  if (nextGroup.activeTabId === tabId) {
    nextGroup.activeTabId = pickFallbackActiveTab(nextGroup.tabIds, tabId);
  }

  let nextRoot = layout.root;
  if (nextGroup.tabIds.length === 0) {
    delete nextGroups[groupId];
    const removed = removeGroupNode(layout.root, groupId);
    if (!removed) {
      return layout;
    }
    nextRoot = removed;
  }

  return {
    groups: nextGroups,
    root: nextRoot,
    activeGroupId: sanitizeActiveGroupId(nextRoot, layout.activeGroupId),
  };
}

export function removeTabFromLayout(
  layout: TabLayoutState,
  tabId: string,
): TabLayoutState {
  const group = getGroupForTab(layout, tabId);
  if (!group) return layout;
  return removeTabFromGroupState(layout, group.id, tabId);
}

export function moveTabToGroup(
  layout: TabLayoutState,
  tabId: string,
  targetGroupId: string,
  overTabId?: string | null,
  position: TabDropPosition = "after",
): TabLayoutState {
  const sourceGroup = getGroupForTab(layout, tabId);
  const targetGroup = layout.groups[targetGroupId];
  if (!sourceGroup || !targetGroup) return layout;

  if (sourceGroup.id === targetGroupId) {
    if (!overTabId) return layout;
    return reorderTabsInGroup(layout, targetGroupId, tabId, overTabId, position);
  }

  const withoutSource = removeTabFromGroupState(layout, sourceGroup.id, tabId);
  const nextGroups = cloneGroups(withoutSource.groups);
  const nextTarget = nextGroups[targetGroupId];
  if (!nextTarget || nextTarget.tabIds.includes(tabId)) {
    return withoutSource;
  }

  const insertIndex =
    overTabId && nextTarget.tabIds.includes(overTabId)
      ? nextTarget.tabIds.indexOf(overTabId) + (position === "after" ? 1 : 0)
      : nextTarget.tabIds.length;

  nextTarget.tabIds.splice(insertIndex, 0, tabId);
  nextTarget.activeTabId = tabId;

  return {
    ...withoutSource,
    groups: nextGroups,
    activeGroupId: targetGroupId,
  };
}

export function splitTabToGroupEdge(
  layout: TabLayoutState,
  tabId: string,
  targetGroupId: string,
  direction: TabSplitDirection,
): TabLayoutState {
  const sourceGroup = getGroupForTab(layout, tabId);
  const targetGroup = layout.groups[targetGroupId];
  if (!sourceGroup || !targetGroup) return layout;
  if (sourceGroup.id === targetGroupId && sourceGroup.tabIds.length <= 1) {
    return layout;
  }

  const withoutSource = removeTabFromGroupState(layout, sourceGroup.id, tabId);
  const nextGroups = cloneGroups(withoutSource.groups);
  const newGroupId = createSyntheticId("group");
  nextGroups[newGroupId] = {
    id: newGroupId,
    tabIds: [tabId],
    activeTabId: tabId,
  };

  const targetNode: TabLayoutNode = { type: "group", groupId: targetGroupId };
  const newGroupNode: TabLayoutNode = { type: "group", groupId: newGroupId };
  const replacement: TabLayoutNode = {
    type: "split",
    id: createSyntheticId("split"),
    axis: axisForDirection(direction),
    sizes: [0.5, 0.5],
    children: splitChildrenForDirection(direction, targetNode, newGroupNode),
  };

  const nextRoot = replaceGroupNode(withoutSource.root, targetGroupId, replacement);
  return {
    groups: nextGroups,
    root: nextRoot,
    activeGroupId: newGroupId,
  };
}

export function resizeSplitNode(
  layout: TabLayoutState,
  splitId: string,
  primaryFraction: number,
): TabLayoutState {
  const clamped = Math.max(0.1, Math.min(0.9, primaryFraction));

  function visit(node: TabLayoutNode): TabLayoutNode {
    if (node.type === "group") return node;
    const [left, right] = node.children;
    const nextLeft = visit(left);
    const nextRight = visit(right);
    if (node.id !== splitId && nextLeft === left && nextRight === right) {
      return node;
    }
    return {
      ...node,
      sizes: node.id === splitId ? [clamped, 1 - clamped] : node.sizes,
      children: [nextLeft, nextRight],
    };
  }

  const nextRoot = visit(layout.root);
  if (nextRoot === layout.root) return layout;
  return {
    ...layout,
    root: nextRoot,
  };
}

export function flattenTabIdsByLayout(
  layout: TabLayoutState,
): string[] {
  const orderedGroupIds = collectGroupIds(layout.root);
  const out: string[] = [];
  for (const groupId of orderedGroupIds) {
    const group = layout.groups[groupId];
    if (!group) continue;
    out.push(...group.tabIds);
  }
  return out;
}
