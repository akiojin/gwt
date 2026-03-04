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

export type StoredTerminalTab = {
  type: "terminal";
  paneId: string;
  label: string;
  cwd?: string;
};

export type StoredStaticTab = {
  type: "projectMode" | "settings" | "versionHistory" | "issues";
  id: string;
  label: string;
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
};

/**
 * Result of restoring persisted tabs against currently known panes.
 */
export type BuildRestoredProjectTabsResult = {
  tabs: Tab[];
  activeTabId: string | null;
  terminalTabsToRespawn: StoredTerminalTab[];
  activeTerminalPaneIdToRespawn: string | null;
};

// Backward-compatible shape consumed by legacy App.svelte restore/persist flow.
export type StoredProjectAgentTabs = {
  tabs: Array<{ paneId: string; label: string }>;
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
  version: 2;
  byProjectPath: Record<string, StoredProjectTabs>;
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
    const agentId = normalizeAgentId(obj.agentId);
    return {
      type: "agent",
      paneId,
      label,
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
    type === "projectMode" ||
    type === "settings" ||
    type === "versionHistory" ||
    type === "issues"
  ) {
    const canonicalType = type === "projectMode" ? "projectMode" : type;
    const fallbackId =
      canonicalType === "projectMode"
        ? "projectMode"
        : canonicalType === "settings"
          ? "settings"
          : canonicalType === "issues"
            ? "issues"
            : "versionHistory";
    const fallbackLabel =
      canonicalType === "projectMode"
        ? "Project Mode"
        : canonicalType === "settings"
          ? "Settings"
          : canonicalType === "issues"
            ? "Issues"
            : "Version History";
    const idRaw = normalizeString(obj.id);
    const id =
      canonicalType === "projectMode"
        ? "projectMode"
        : idRaw || fallbackId;
    const labelRaw = typeof obj.label === "string" ? obj.label.trim() : "";
    const label =
      canonicalType === "projectMode" && (type === "projectMode" || !labelRaw)
        ? "Project Mode"
        : labelRaw || fallbackLabel;
    return { type: canonicalType, id, label };
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
  return { tabs, activeTabId };
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
    loadStoredProjectTabsV2(key, store) ??
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
      const agentId = inferAgentId(terminal?.agent_name) ?? tab.agentId;
      restoredTabs.push({
        id: `agent-${tab.paneId}`,
        label: tab.label,
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

  if (!restoredTabs.some((tab) => tab.id === "projectMode")) {
    restoredTabs.unshift({
      id: "projectMode",
      label: "Project Mode",
      type: "projectMode",
    });
  }

  const restoredIds = new Set(restoredTabs.map((tab) => tab.id));
  const normalizedActiveTabId =
    stored.activeTabId === "projectMode" ? "projectMode" : stored.activeTabId;
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

  return {
    tabs: restoredTabs,
    activeTabId,
    terminalTabsToRespawn,
    activeTerminalPaneIdToRespawn,
  };
}

export function loadStoredProjectAgentTabs(
  projectPath: string,
  storage?: Storage | null,
): StoredProjectAgentTabs | null {
  const stored = loadStoredProjectTabs(projectPath, storage);
  if (!stored) return null;

  const tabs = stored.tabs
    .filter((tab): tab is StoredAgentTab => tab.type === "agent")
    .map((tab) => ({ paneId: tab.paneId, label: tab.label }));

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
