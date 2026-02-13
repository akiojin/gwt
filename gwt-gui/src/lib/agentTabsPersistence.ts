import type { Tab, TerminalInfo } from "./types";
import { inferAgentId } from "./agentUtils";

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
  agentId?: Tab["agentId"];
};
export type StoredStaticTab = {
  type: "agentMode" | "settings" | "versionHistory";
  id: string;
  label: string;
};
export type StoredProjectTab = StoredAgentTab | StoredStaticTab;

/**
 * Persisted tab state for a single project.
 */
export type StoredProjectTabs = {
  tabs: StoredProjectTab[];
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
    storedTabsCount > 0 &&
    restoredTabsCount === 0 &&
    attempt < maxRetries - 1
  );
}

type StoredProjectTabsRoot = {
  version: 2;
  byProjectPath: Record<string, StoredProjectTabs>;
};

type LegacyStoredAgentTab = { paneId: string; label: string };
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
  if (id === "claude" || id === "codex" || id === "gemini" || id === "opencode") {
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
    const agentId = normalizeAgentId(obj.agentId);
    return {
      type: "agent",
      paneId,
      label,
      ...(agentId ? { agentId } : {}),
    };
  }

  if (type === "agentMode" || type === "settings" || type === "versionHistory") {
    const fallbackId =
      type === "agentMode"
        ? "agentMode"
        : type === "settings"
          ? "settings"
          : "versionHistory";
    const fallbackLabel =
      type === "agentMode"
        ? "Agent Mode"
        : type === "settings"
          ? "Settings"
          : "Version History";
    const id = normalizeString(obj.id) || fallbackId;
    const label = typeof obj.label === "string" ? obj.label : fallbackLabel;
    return { type, id, label };
  }

  return null;
}

function tabStorageKey(tab: StoredProjectTab): string {
  if (tab.type === "agent") return `agent:${tab.paneId}`;
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
  return { tabs, activeTabId };
}

function sanitizeLegacyProjectTabsEntry(rawEntry: unknown): LegacyStoredProjectAgentTabs | null {
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
    tabs.push({ paneId, label });
    seenPaneIds.add(paneId);
  }

  const activePaneId = normalizeString(entry.activePaneId) || null;
  return { tabs, activePaneId };
}

function loadStoredProjectTabsV2(
  projectPath: string,
  store: Storage,
): StoredProjectTabs | null {
  try {
    const raw = store.getItem(PROJECT_TABS_STORAGE_KEY);
    if (!raw) return null;

    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;
    const root = parsed as Partial<StoredProjectTabsRoot>;

    if (root.version !== 2) return null;
    if (!root.byProjectPath || typeof root.byProjectPath !== "object") return null;

    const entryRaw = (root.byProjectPath as Record<string, unknown>)[projectPath];
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
    if (!root.byProjectPath || typeof root.byProjectPath !== "object") return null;

    const entryRaw = (root.byProjectPath as Record<string, unknown>)[projectPath];
    const legacy = sanitizeLegacyProjectTabsEntry(entryRaw);
    if (!legacy) return null;

    const tabs: StoredProjectTab[] = legacy.tabs.map((tab) => ({
      type: "agent",
      paneId: tab.paneId,
      label: tab.label,
    }));
    const activeTabId = legacy.activePaneId ? `agent-${legacy.activePaneId}` : null;

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

  return loadStoredProjectTabsV2(key, store) ?? loadStoredProjectTabsLegacy(key, store);
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
    let root: StoredProjectTabsRoot = { version: 2, byProjectPath: {} };

    if (raw) {
      const parsed: unknown = JSON.parse(raw);
      if (parsed && typeof parsed === "object") {
        const existing = parsed as Partial<StoredProjectTabsRoot>;
        if (
          existing.version === 2 &&
          existing.byProjectPath &&
          typeof existing.byProjectPath === "object"
        ) {
          root = { version: 2, byProjectPath: existing.byProjectPath };
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
 * Build the set of `Tab`s to restore by intersecting persisted pane ids with
 * currently known terminal panes.
 */
export function buildRestoredProjectTabs(
  stored: StoredProjectTabs,
  terminals: TerminalInfo[],
): { tabs: Tab[]; activeTabId: string | null } {
  const existingPaneIds = new Set(terminals.map((t) => t.pane_id));
  const terminalByPaneId = new Map(terminals.map((terminal) => [terminal.pane_id, terminal]));

  const restoredTabs: Tab[] = [];
  const seen = new Set<string>();
  for (const t of stored.tabs) {
    if (t.type === "agent") {
      if (!existingPaneIds.has(t.paneId)) continue;
      const key = `agent:${t.paneId}`;
      if (seen.has(key)) continue;
      seen.add(key);

      const terminal = terminalByPaneId.get(t.paneId);
      const agentId = inferAgentId(terminal?.agent_name) ?? t.agentId;

      restoredTabs.push({
        id: `agent-${t.paneId}`,
        label: t.label,
        type: "agent",
        paneId: t.paneId,
        ...(agentId ? { agentId } : {}),
      });
      continue;
    }

    const key = `id:${t.id}`;
    if (seen.has(key)) continue;
    seen.add(key);
    restoredTabs.push({
      id: t.id,
      label: t.label,
      type: t.type,
    });
  }

  if (!restoredTabs.some((tab) => tab.id === "agentMode")) {
    restoredTabs.unshift({ id: "agentMode", label: "Agent Mode", type: "agentMode" });
  }

  const restoredIds = new Set(restoredTabs.map((tab) => tab.id));
  const activeTabId =
    stored.activeTabId && restoredIds.has(stored.activeTabId)
      ? stored.activeTabId
      : null;

  return { tabs: restoredTabs, activeTabId };
}
