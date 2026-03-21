import type { Tab, TerminalInfo } from "./types";
import { inferAgentId } from "./agentUtils";
import type { TabGroupState, TabLayoutNode } from "./tabLayout";
import {
  createInitialTabLayout,
  flattenTabIdsByLayout,
  normalizeTabLayoutState,
} from "./tabLayout";

/**
 * localStorage key used to persist tab state (per project path).
 */
export const PROJECT_TABS_STORAGE_KEY = "gwt.projectTabs.v2";
export const PROJECT_AGENT_TABS_STORAGE_KEY = "gwt.projectAgentTabs.v1";

/**
 * Minimal persisted representation of a tab.
 */
export type StoredAgentTab = {
  type: "agent";
  paneId: string;
  label: string;
  branchName?: string;
  agentId?: Tab["agentId"];
};

export type StoredTerminalTab = {
  type: "terminal";
  paneId: string;
  label: string;
  cwd?: string;
};

export type StoredStaticTab = {
  type:
    | "assistant"
    | "agentCanvas"
    | "branchBrowser"
    | "settings"
    | "versionHistory"
    | "issues"
    | "prs"
    | "projectIndex"
    | "issueSpec";
  id: string;
  label: string;
  issueNumber?: number;
};

export type StoredProjectTab =
  | StoredAgentTab
  | StoredTerminalTab
  | StoredStaticTab;

/**
 * Persisted tab state for a single project.
 */
export type StoredProjectTabs = {
  tabs: StoredProjectTab[];
  activeTabId: string | null;
  activeGroupId?: string | null;
  groups?: StoredTabGroup[];
  root?: StoredTabLayoutNode | null;
};

/**
 * Result of restoring persisted tabs against currently known panes.
 */
export type BuildRestoredProjectTabsResult = {
  tabs: Tab[];
  activeTabId: string | null;
  activeGroupId: string | null;
  groups: StoredTabGroup[];
  root: StoredTabLayoutNode;
  terminalTabsToRespawn: StoredTerminalTab[];
  activeTerminalPaneIdToRespawn: string | null;
};

// Backward-compatible shape consumed by legacy App.svelte restore/persist flow.
export type StoredProjectAgentTabs = {
  tabs: Array<{ paneId: string; label: string; branchName?: string }>;
  activePaneId: string | null;
};

export type BuildRestoredAgentTabsResult = {
  tabs: Tab[];
  activeTabId: string | null;
};

export const AGENT_TAB_RESTORE_MAX_RETRIES = 8;

export function shouldRetryAgentTabRestore(
  storedTabsCount: number,
  restoredTabsCount: number,
  attempt: number,
  maxRetries = AGENT_TAB_RESTORE_MAX_RETRIES,
): boolean {
  return (
    storedTabsCount > 0 && restoredTabsCount === 0 && attempt < maxRetries - 1
  );
}

type StoredProjectTabsRoot = {
  version: 2 | 3;
  byProjectPath: Record<string, StoredProjectTabs>;
};

export type StoredTabGroup = {
  id: string;
  tabIds: string[];
  activeTabId: string | null;
};

export type StoredTabLayoutNode =
  | {
      type: "group";
      groupId: string;
    }
  | {
      type: "split";
      id: string;
      axis: "horizontal" | "vertical";
      sizes: [number, number];
      children: [StoredTabLayoutNode, StoredTabLayoutNode];
    };

type LegacyStoredAgentTab = {
  paneId: string;
  label: string;
  type?: "terminal";
  cwd?: string;
};

type LegacyStoredProjectAgentTabs = {
  tabs: LegacyStoredAgentTab[];
  activePaneId: string | null;
};

type LegacyStoredProjectAgentTabsRoot = {
  version: 1;
  byProjectPath: Record<string, LegacyStoredProjectAgentTabs>;
};

function getStorageSafe(storage?: Storage | null): Storage | null {
  try {
    if (storage) return storage;
    if (typeof window === "undefined") return null;
    return window.localStorage;
  } catch {
    return null;
  }
}

function normalizeString(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function normalizeAgentId(value: unknown): Tab["agentId"] | undefined {
  const id = normalizeString(value);
  if (
    id === "claude" ||
    id === "codex" ||
    id === "gemini" ||
    id === "opencode" ||
    id === "copilot"
  ) {
    return id;
  }
  return undefined;
}

function parseStoredProjectTab(raw: unknown): StoredProjectTab | null {
  if (!raw || typeof raw !== "object") return null;
  const obj = raw as Record<string, unknown>;
  const type = normalizeString(obj.type);

  if (type === "agent") {
    const paneId = normalizeString(obj.paneId);
    if (!paneId) return null;
    const label = typeof obj.label === "string" ? obj.label : "";
    const branchName = normalizeString(obj.branchName);
    const agentId = normalizeAgentId(obj.agentId);
    return {
      type: "agent",
      paneId,
      label,
      ...(branchName ? { branchName } : {}),
      ...(agentId ? { agentId } : {}),
    };
  }

  if (type === "terminal") {
    const paneId = normalizeString(obj.paneId);
    if (!paneId) return null;
    const label = typeof obj.label === "string" ? obj.label : "";
    const cwd = typeof obj.cwd === "string" ? obj.cwd : undefined;
    return {
      type: "terminal",
      paneId,
      label,
      ...(cwd ? { cwd } : {}),
    };
  }

  if (
    type === "assistant" ||
    type === "agentCanvas" ||
    type === "branchBrowser" ||
    type === "settings" ||
    type === "versionHistory" ||
    type === "issues" ||
    type === "prs" ||
    type === "projectIndex" ||
    type === "issueSpec"
  ) {
    const canonicalType =
      type === "assistant" ? "agentCanvas" : type;
    const fallbackId =
      canonicalType === "agentCanvas"
        ? "agentCanvas"
        : canonicalType === "branchBrowser"
          ? "branchBrowser"
        : canonicalType === "settings"
          ? "settings"
          : canonicalType === "issues"
            ? "issues"
            : canonicalType === "prs"
              ? "prs"
              : canonicalType === "projectIndex"
                ? "projectIndex"
                : canonicalType === "issueSpec"
                  ? "issueSpec"
                  : "versionHistory";
    const fallbackLabel =
      canonicalType === "agentCanvas"
        ? "Agent Canvas"
        : canonicalType === "branchBrowser"
          ? "Branch Browser"
        : canonicalType === "settings"
          ? "Settings"
          : canonicalType === "issues"
            ? "Issues"
            : canonicalType === "prs"
              ? "Pull Requests"
              : canonicalType === "projectIndex"
                ? "Project Index"
                : canonicalType === "issueSpec"
                  ? "Issue"
                  : "Version History";
    const idRaw = normalizeString(obj.id);
    const id =
      canonicalType === "agentCanvas"
        ? "agentCanvas"
        : idRaw || fallbackId;
    const labelRaw = typeof obj.label === "string" ? obj.label.trim() : "";
    const label =
      canonicalType === "agentCanvas" && (type === "assistant" || !labelRaw)
        ? "Agent Canvas"
        : labelRaw || fallbackLabel;
    const issueNumber =
      canonicalType === "issueSpec" && Number.isFinite(Number(obj.issueNumber))
        ? Number(obj.issueNumber)
        : undefined;
    return {
      type: canonicalType,
      id,
      label,
      ...(issueNumber && issueNumber > 0 ? { issueNumber } : {}),
    };
  }

  return null;
}

function tabStorageKey(tab: StoredProjectTab): string {
  if (tab.type === "agent") return `agent:${tab.paneId}`;
  if (tab.type === "terminal") return `terminal:${tab.paneId}`;
  return `id:${tab.id}`;
}

function sanitizeProjectTabsEntry(rawEntry: unknown): StoredProjectTabs | null {
  if (!rawEntry || typeof rawEntry !== "object") return null;
  const entry = rawEntry as Record<string, unknown>;
  const tabsRaw = Array.isArray(entry.tabs) ? entry.tabs : [];

  const tabs: StoredProjectTab[] = [];
  const seen = new Set<string>();
  for (const rawTab of tabsRaw) {
    const tab = parseStoredProjectTab(rawTab);
    if (!tab) continue;
    const key = tabStorageKey(tab);
    if (seen.has(key)) continue;
    seen.add(key);
    tabs.push(tab);
  }

  const activeTabId = normalizeString(entry.activeTabId) || null;
  const groups = sanitizeStoredGroups(entry.groups, tabs.map(resolveStoredTabId));
  const root = sanitizeStoredRoot(entry.root, groups.map((group) => group.id));
  const activeGroupId = normalizeString(entry.activeGroupId) || null;

  return {
    tabs,
    activeTabId,
    ...(groups.length > 0 ? { groups } : {}),
    ...(root ? { root } : {}),
    ...(activeGroupId ? { activeGroupId } : {}),
  };
}

function resolveStoredTabId(tab: StoredProjectTab): string {
  if (tab.type === "agent") return `agent-${tab.paneId}`;
  if (tab.type === "terminal") return `terminal-${tab.paneId}`;
  return tab.id;
}

function sanitizeStoredGroups(
  rawGroups: unknown,
  knownTabIds: string[],
): StoredTabGroup[] {
  if (!Array.isArray(rawGroups) || knownTabIds.length === 0) {
    return [];
  }

  const known = new Set(knownTabIds);
  const groups: StoredTabGroup[] = [];
  const seenGroups = new Set<string>();

  for (const rawGroup of rawGroups) {
    if (!rawGroup || typeof rawGroup !== "object") continue;
    const obj = rawGroup as Record<string, unknown>;
    const id = normalizeString(obj.id);
    if (!id || seenGroups.has(id)) continue;
    const tabIds = Array.isArray(obj.tabIds)
      ? obj.tabIds
          .map((value) => normalizeString(value))
          .filter((value) => value && known.has(value))
      : [];
    if (tabIds.length === 0) continue;
    const activeTabIdRaw = normalizeString(obj.activeTabId);
    groups.push({
      id,
      tabIds,
      activeTabId: activeTabIdRaw && tabIds.includes(activeTabIdRaw) ? activeTabIdRaw : (tabIds[0] ?? null),
    });
    seenGroups.add(id);
  }

  return groups;
}

function sanitizeStoredRoot(
  rawRoot: unknown,
  knownGroupIds: string[],
): StoredTabLayoutNode | null {
  if (!rawRoot || typeof rawRoot !== "object" || knownGroupIds.length === 0) {
    return null;
  }
  const known = new Set(knownGroupIds);

  function visit(rawNode: unknown): StoredTabLayoutNode | null {
    if (!rawNode || typeof rawNode !== "object") return null;
    const obj = rawNode as Record<string, unknown>;
    const type = normalizeString(obj.type);
    if (type === "group") {
      const groupId = normalizeString(obj.groupId);
      return groupId && known.has(groupId) ? { type: "group", groupId } : null;
    }
    if (type !== "split") return null;
    const id = normalizeString(obj.id);
    const axis =
      normalizeString(obj.axis) === "vertical" ? "vertical" : "horizontal";
    const childrenRaw = Array.isArray(obj.children) ? obj.children : [];
    if (childrenRaw.length !== 2) return null;
    const first = visit(childrenRaw[0]);
    const second = visit(childrenRaw[1]);
    if (!first && !second) return null;
    if (!first) return second;
    if (!second) return first;
    const sizesRaw = Array.isArray(obj.sizes) ? obj.sizes : [];
    const firstSize = Number(sizesRaw[0]);
    const primary =
      Number.isFinite(firstSize) && firstSize > 0 && firstSize < 1
        ? firstSize
        : 0.5;
    return {
      type: "split",
      id: id || `split-restored-${knownGroupIds.length}`,
      axis,
      sizes: [primary, 1 - primary],
      children: [first, second],
    };
  }

  return visit(rawRoot);
}

function sanitizeLegacyProjectTabsEntry(
  rawEntry: unknown,
): LegacyStoredProjectAgentTabs | null {
  if (!rawEntry || typeof rawEntry !== "object") return null;
  const entry = rawEntry as Record<string, unknown>;
  const tabsRaw = Array.isArray(entry.tabs) ? entry.tabs : [];

  const tabs: LegacyStoredAgentTab[] = [];
  const seenPaneIds = new Set<string>();
  for (const rawTab of tabsRaw) {
    if (!rawTab || typeof rawTab !== "object") continue;
    const obj = rawTab as Record<string, unknown>;
    const paneId = normalizeString(obj.paneId);
    if (!paneId || seenPaneIds.has(paneId)) continue;
    const label = typeof obj.label === "string" ? obj.label : "";
    const type =
      normalizeString(obj.type) === "terminal" ? "terminal" : undefined;
    const cwd = typeof obj.cwd === "string" ? obj.cwd : undefined;
    tabs.push({
      paneId,
      label,
      ...(type ? { type } : {}),
      ...(cwd ? { cwd } : {}),
    });
    seenPaneIds.add(paneId);
  }

  const activePaneId = normalizeString(entry.activePaneId) || null;
  return { tabs, activePaneId };
}

function loadStoredProjectTabsCurrent(
  projectPath: string,
  store: Storage,
): StoredProjectTabs | null {
  try {
    const raw = store.getItem(PROJECT_TABS_STORAGE_KEY);
    if (!raw) return null;

    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;
    const root = parsed as Partial<StoredProjectTabsRoot>;

    if (root.version !== 2 && root.version !== 3) return null;
    if (!root.byProjectPath || typeof root.byProjectPath !== "object")
      return null;

    const entryRaw = (root.byProjectPath as Record<string, unknown>)[
      projectPath
    ];
    return sanitizeProjectTabsEntry(entryRaw);
  } catch {
    return null;
  }
}

function loadStoredProjectTabsLegacy(
  projectPath: string,
  store: Storage,
): StoredProjectTabs | null {
  try {
    const raw = store.getItem(PROJECT_AGENT_TABS_STORAGE_KEY);
    if (!raw) return null;

    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;
    const root = parsed as Partial<LegacyStoredProjectAgentTabsRoot>;

    if (root.version !== 1) return null;
    if (!root.byProjectPath || typeof root.byProjectPath !== "object")
      return null;

    const entryRaw = (root.byProjectPath as Record<string, unknown>)[
      projectPath
    ];
    const legacy = sanitizeLegacyProjectTabsEntry(entryRaw);
    if (!legacy) return null;

    const tabs: StoredProjectTab[] = legacy.tabs.map((tab) => {
      if (tab.type === "terminal") {
        return {
          type: "terminal",
          paneId: tab.paneId,
          label: tab.label,
          ...(tab.cwd ? { cwd: tab.cwd } : {}),
        };
      }

      return {
        type: "agent",
        paneId: tab.paneId,
        label: tab.label,
        ...(tab.label ? { branchName: tab.label } : {}),
      };
    });

    const activeEntry = legacy.tabs.find(
      (tab) => tab.paneId === legacy.activePaneId,
    );
    const activeTabId = legacy.activePaneId
      ? `${activeEntry?.type === "terminal" ? "terminal" : "agent"}-${legacy.activePaneId}`
      : null;

    return { tabs, activeTabId };
  } catch {
    return null;
  }
}

/**
 * Load stored tab state for the given project path.
 *
 * `storage` is injectable for tests; defaults to `window.localStorage` when available.
 */
export function loadStoredProjectTabs(
  projectPath: string,
  storage?: Storage | null,
): StoredProjectTabs | null {
  const store = getStorageSafe(storage);
  if (!store) return null;

  const key = projectPath.trim();
  if (!key) return null;

  return (
    loadStoredProjectTabsCurrent(key, store) ??
    loadStoredProjectTabsLegacy(key, store)
  );
}

/**
 * Persist tab state for the given project path.
 *
 * `storage` is injectable for tests; defaults to `window.localStorage` when available.
 */
export function persistStoredProjectTabs(
  projectPath: string,
  state: StoredProjectTabs,
  storage?: Storage | null,
) {
  const store = getStorageSafe(storage);
  if (!store) return;

  const key = projectPath.trim();
  if (!key) return;

  try {
    const raw = store.getItem(PROJECT_TABS_STORAGE_KEY);
    let root: StoredProjectTabsRoot = { version: 3, byProjectPath: {} };

    if (raw) {
      const parsed: unknown = JSON.parse(raw);
      if (parsed && typeof parsed === "object") {
        const existing = parsed as Partial<StoredProjectTabsRoot>;
        if (
          (existing.version === 2 || existing.version === 3) &&
          existing.byProjectPath &&
          typeof existing.byProjectPath === "object"
        ) {
          root = { version: existing.version, byProjectPath: existing.byProjectPath };
        }
      }
    }

    const sanitized = sanitizeProjectTabsEntry(state);
    if (!sanitized) return;
    root.byProjectPath = { ...root.byProjectPath, [key]: sanitized };
    store.setItem(PROJECT_TABS_STORAGE_KEY, JSON.stringify(root));
  } catch {
    // Ignore storage failures.
  }
}

/**
 * Build the set of tabs that can be restored immediately by intersecting persisted pane ids
 * with currently known terminal panes, and list persisted terminal tabs that need respawn.
 */
export function buildRestoredProjectTabs(
  stored: StoredProjectTabs,
  terminals: TerminalInfo[],
): BuildRestoredProjectTabsResult {
  const existingPaneIds = new Set(terminals.map((t) => t.pane_id));
  const terminalByPaneId = new Map(
    terminals.map((terminal) => [terminal.pane_id, terminal]),
  );

  const restoredTabs: Tab[] = [];
  const terminalTabsToRespawn: StoredTerminalTab[] = [];
  const seen = new Set<string>();

  for (const tab of stored.tabs) {
    if (tab.type === "agent") {
      if (!existingPaneIds.has(tab.paneId)) continue;
      const key = `agent:${tab.paneId}`;
      if (seen.has(key)) continue;
      seen.add(key);

      const terminal = terminalByPaneId.get(tab.paneId);
      const branchName =
        normalizeString(tab.branchName) ||
        normalizeString(terminal?.branch_name) ||
        normalizeString(tab.label);
      const agentId = inferAgentId(terminal?.agent_name) ?? tab.agentId;
      restoredTabs.push({
        id: `agent-${tab.paneId}`,
        label: tab.label,
        ...(branchName ? { branchName } : {}),
        type: "agent",
        paneId: tab.paneId,
        ...(agentId ? { agentId } : {}),
      });
      continue;
    }

    if (tab.type === "terminal") {
      const key = `terminal:${tab.paneId}`;
      if (seen.has(key)) continue;
      seen.add(key);

      if (existingPaneIds.has(tab.paneId)) {
        restoredTabs.push({
          id: `terminal-${tab.paneId}`,
          label: tab.label,
          type: "terminal",
          paneId: tab.paneId,
          ...(tab.cwd ? { cwd: tab.cwd } : {}),
        });
      } else {
        terminalTabsToRespawn.push(tab);
      }
      continue;
    }

    const key = `id:${tab.id}`;
    if (seen.has(key)) continue;
    seen.add(key);
    restoredTabs.push({ id: tab.id, label: tab.label, type: tab.type });
  }

  if (!restoredTabs.some((tab) => tab.id === "assistant")) {
    restoredTabs.unshift({
      id: "assistant",
      label: "Assistant",
      type: "assistant",
    });
  }

  const restoredIds = new Set(restoredTabs.map((tab) => tab.id));
  const normalizedActiveTabId =
    stored.activeTabId === "assistant" ? "assistant" : stored.activeTabId;
  const activeTabId =
    normalizedActiveTabId && restoredIds.has(normalizedActiveTabId)
      ? normalizedActiveTabId
      : null;

  const activeTerminalPaneId =
    stored.activeTabId && stored.activeTabId.startsWith("terminal-")
      ? stored.activeTabId.slice("terminal-".length)
      : "";

  const activeTerminalPaneIdToRespawn =
    activeTerminalPaneId &&
    terminalTabsToRespawn.some((tab) => tab.paneId === activeTerminalPaneId)
      ? activeTerminalPaneId
      : null;

  const restoredLayout = buildRestoredLayoutState(
    stored,
    restoredTabs,
    activeTabId,
  );

  return {
    tabs: restoredTabs,
    activeTabId,
    activeGroupId: restoredLayout.activeGroupId,
    groups: Object.values(restoredLayout.groups),
    root: restoredLayout.root as StoredTabLayoutNode,
    terminalTabsToRespawn,
    activeTerminalPaneIdToRespawn,
  };
}

function buildRestoredLayoutState(
  stored: StoredProjectTabs,
  restoredTabs: Tab[],
  restoredActiveTabId: string | null,
): { groups: Record<string, TabGroupState>; root: TabLayoutNode; activeGroupId: string } {
  if (restoredTabs.length === 0) {
    return createInitialTabLayout([], null);
  }

  const restoredTabIds = restoredTabs.map((tab) => tab.id);
  const knownTabs = new Set(restoredTabIds);
  const storedGroups = Array.isArray(stored.groups) ? stored.groups : [];
  const nextGroups: Record<string, TabGroupState> = {};

  for (const group of storedGroups) {
    const tabIds = group.tabIds.filter((tabId) => knownTabs.has(tabId));
    if (tabIds.length === 0) continue;
    nextGroups[group.id] = {
      id: group.id,
      tabIds,
      activeTabId:
        group.activeTabId && tabIds.includes(group.activeTabId)
          ? group.activeTabId
          : (tabIds[0] ?? null),
    };
  }

  let root =
    stored.root && Object.keys(nextGroups).length > 0
      ? sanitizeStoredRoot(stored.root, Object.keys(nextGroups))
      : null;

  if (!root || Object.keys(nextGroups).length === 0) {
    return createInitialTabLayout(restoredTabs, restoredActiveTabId);
  }

  const assigned = new Set<string>();
  for (const group of Object.values(nextGroups)) {
    for (const tabId of group.tabIds) {
      assigned.add(tabId);
    }
  }

  const missingTabIds = restoredTabIds.filter((tabId) => !assigned.has(tabId));
  if (missingTabIds.length > 0) {
    const firstGroupId = Object.keys(nextGroups)[0];
    if (firstGroupId) {
      const target = nextGroups[firstGroupId];
      target.tabIds = [...target.tabIds, ...missingTabIds];
      if (!target.activeTabId) {
        target.activeTabId = target.tabIds[0] ?? null;
      }
    }
  }

  const activeGroupIdRaw =
    normalizeString(stored.activeGroupId) ||
    Object.values(nextGroups).find(
      (group) => group.activeTabId && group.activeTabId === restoredActiveTabId,
    )?.id ||
    Object.keys(nextGroups)[0] ||
    "";

  return normalizeTabLayoutState(
    {
      groups: nextGroups,
      root,
      activeGroupId: activeGroupIdRaw,
    },
    restoredActiveTabId,
  );
}

export function loadStoredProjectAgentTabs(
  projectPath: string,
  storage?: Storage | null,
): StoredProjectAgentTabs | null {
  const stored = loadStoredProjectTabs(projectPath, storage);
  if (!stored) return null;

  const tabs = stored.tabs
    .filter((tab): tab is StoredAgentTab => tab.type === "agent")
    .map((tab) => ({
      paneId: tab.paneId,
      label: tab.label,
      ...(tab.branchName ? { branchName: tab.branchName } : {}),
    }));

  const activePaneId =
    stored.activeTabId && stored.activeTabId.startsWith("agent-")
      ? stored.activeTabId.slice("agent-".length)
      : null;

  return { tabs, activePaneId };
}

export function persistStoredProjectAgentTabs(
  projectPath: string,
  state: StoredProjectAgentTabs,
  storage?: Storage | null,
) {
  const agentTabs: StoredProjectTab[] = state.tabs
    .map((tab) => {
      const paneId = normalizeString(tab.paneId);
      if (!paneId) return null;
      return {
        type: "agent" as const,
        paneId,
        label: tab.label ?? "",
        ...(normalizeString(tab.branchName) ? { branchName: normalizeString(tab.branchName) } : {}),
      };
    })
    .filter((tab): tab is StoredAgentTab => tab !== null);

  const existing = loadStoredProjectTabs(projectPath, storage);
  const preservedTabs = (existing?.tabs ?? []).filter((tab) => tab.type !== "agent");

  const activePaneId = normalizeString(state.activePaneId ?? "");
  const existingActiveTabId = existing?.activeTabId ?? null;
  const activeTabId = activePaneId
    ? `agent-${activePaneId}`
    : existingActiveTabId?.startsWith("agent-")
      ? null
      : existingActiveTabId;

  persistStoredProjectTabs(
    projectPath,
    {
      tabs: [...preservedTabs, ...agentTabs],
      activeTabId,
    },
    storage,
  );
}

export function buildRestoredAgentTabs(
  stored: StoredProjectAgentTabs,
  terminals: TerminalInfo[],
): BuildRestoredAgentTabsResult {
  const restored = buildRestoredProjectTabs(
    {
      tabs: stored.tabs.map((tab) => ({
        type: "agent" as const,
        paneId: tab.paneId,
        label: tab.label,
        ...(tab.branchName ? { branchName: tab.branchName } : {}),
      })),
      activeTabId: stored.activePaneId ? `agent-${stored.activePaneId}` : null,
    },
    terminals,
  );

  const tabs = restored.tabs.filter((tab) => tab.type === "agent");
  const activeTabId =
    restored.activeTabId && restored.activeTabId.startsWith("agent-")
      ? restored.activeTabId
      : null;

  return { tabs, activeTabId };
}
