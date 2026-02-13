import type { Tab, TerminalInfo } from "./types";
import { inferAgentId } from "./agentUtils";

/**
 * localStorage key used to persist agent tab state (per project path).
 */
export const PROJECT_AGENT_TABS_STORAGE_KEY = "gwt.projectAgentTabs.v1";

/**
 * Minimal persisted representation of an agent tab.
 */
export type StoredAgentTab = { paneId: string; label: string };

/**
 * Persisted agent tab state for a single project.
 */
export type StoredProjectAgentTabs = {
  tabs: StoredAgentTab[];
  activePaneId: string | null;
};

export const AGENT_TAB_RESTORE_MAX_RETRIES = 8;

export function shouldRetryAgentTabRestore(
  storedTabsCount: number,
  restoredTabsCount: number,
  attempt: number,
  maxRetries = AGENT_TAB_RESTORE_MAX_RETRIES,
): boolean {
  return (
    storedTabsCount > 0 &&
    restoredTabsCount === 0 &&
    attempt < maxRetries - 1
  );
}

type StoredProjectAgentTabsRoot = {
  version: 1;
  byProjectPath: Record<string, StoredProjectAgentTabs>;
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

/**
 * Load stored agent tab state for the given project path.
 *
 * `storage` is injectable for tests; defaults to `window.localStorage` when available.
 */
export function loadStoredProjectAgentTabs(
  projectPath: string,
  storage?: Storage | null,
): StoredProjectAgentTabs | null {
  const store = getStorageSafe(storage);
  if (!store) return null;

  const key = projectPath.trim();
  if (!key) return null;

  try {
    const raw = store.getItem(PROJECT_AGENT_TABS_STORAGE_KEY);
    if (!raw) return null;

    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;

    const root = parsed as Partial<StoredProjectAgentTabsRoot>;
    if (root.version !== 1) return null;
    if (!root.byProjectPath || typeof root.byProjectPath !== "object") return null;

    const entryRaw = (root.byProjectPath as Record<string, unknown>)[key];
    if (!entryRaw || typeof entryRaw !== "object") return null;

    const entry = entryRaw as Partial<StoredProjectAgentTabs>;
    const tabsRaw = Array.isArray(entry.tabs) ? entry.tabs : [];

    const seen = new Set<string>();
    const tabs: StoredAgentTab[] = [];
    for (const t of tabsRaw) {
      if (!t || typeof t !== "object") continue;
      const obj = t as Partial<StoredAgentTab>;
      const paneId = typeof obj.paneId === "string" ? obj.paneId.trim() : "";
      if (!paneId || seen.has(paneId)) continue;
      const label = typeof obj.label === "string" ? obj.label : "";
      tabs.push({ paneId, label });
      seen.add(paneId);
    }

    const active =
      typeof entry.activePaneId === "string" ? entry.activePaneId.trim() : "";
    const activePaneId = active ? active : null;

    return { tabs, activePaneId };
  } catch {
    return null;
  }
}

/**
 * Persist agent tab state for the given project path.
 *
 * `storage` is injectable for tests; defaults to `window.localStorage` when available.
 */
export function persistStoredProjectAgentTabs(
  projectPath: string,
  state: StoredProjectAgentTabs,
  storage?: Storage | null,
) {
  const store = getStorageSafe(storage);
  if (!store) return;

  const key = projectPath.trim();
  if (!key) return;

  try {
    const raw = store.getItem(PROJECT_AGENT_TABS_STORAGE_KEY);
    let root: StoredProjectAgentTabsRoot = { version: 1, byProjectPath: {} };

    if (raw) {
      const parsed: unknown = JSON.parse(raw);
      if (parsed && typeof parsed === "object") {
        const existing = parsed as Partial<StoredProjectAgentTabsRoot>;
        if (
          existing.version === 1 &&
          existing.byProjectPath &&
          typeof existing.byProjectPath === "object"
        ) {
          root = { version: 1, byProjectPath: existing.byProjectPath };
        }
      }
    }

    root.byProjectPath = { ...root.byProjectPath, [key]: state };
    store.setItem(PROJECT_AGENT_TABS_STORAGE_KEY, JSON.stringify(root));
  } catch {
    // Ignore storage failures.
  }
}

/**
 * Build the set of agent `Tab`s to restore by intersecting persisted pane ids with
 * the currently known terminal panes.
 */
export function buildRestoredAgentTabs(
  stored: StoredProjectAgentTabs,
  terminals: TerminalInfo[],
): { tabs: Tab[]; activeTabId: string | null } {
  const existingPaneIds = new Set(terminals.map((t) => t.pane_id));
  const terminalByPaneId = new Map(terminals.map((terminal) => [terminal.pane_id, terminal]));

  const restoredTabs: Tab[] = [];
  for (const t of stored.tabs) {
    if (!existingPaneIds.has(t.paneId)) continue;

    const terminal = terminalByPaneId.get(t.paneId);
    const agentId = inferAgentId(terminal?.agent_name);

    restoredTabs.push({
      id: `agent-${t.paneId}`,
      label: t.label,
      type: "agent",
      paneId: t.paneId,
      ...(agentId ? { agentId } : {}),
    });
  }

  const restoredActive =
    stored.activePaneId && existingPaneIds.has(stored.activePaneId)
      ? `agent-${stored.activePaneId}`
      : "";

  const activeTabId =
    restoredActive && restoredTabs.some((t) => t.id === restoredActive)
      ? restoredActive
      : null;

  return { tabs: restoredTabs, activeTabId };
}
