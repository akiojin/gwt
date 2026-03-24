import { createDefaultAgentCanvasViewport } from "./agentCanvas";
import {
  buildRestoredProjectTabs,
  loadStoredProjectTabs,
  shouldRetryAgentTabRestore,
  type StoredBranchBrowserState,
  type StoredProjectTab,
  type StoredProjectTabs,
  type StoredTerminalTab,
} from "./agentTabsPersistence";
import { shouldAllowRestoredActiveTab } from "./appTabs";
import type {
  StartupFrontendMetric,
  StartupProfilePhase,
} from "./startupProfiling";
import type { AgentCanvasCardLayout, AgentCanvasViewport } from "./agentCanvas";
import type { Tab, TerminalInfo } from "./types";

export function getAgentTabRestoreDelayMs(
  attempt: number,
  baseDelayMs: number,
  maxDelayMs: number,
): number {
  return Math.min(maxDelayMs, baseDelayMs * 2 ** Math.min(attempt, 8));
}

export function mergeRestoredTabs(existingTabs: Tab[], restoredTabs: Tab[]): Tab[] {
  const normalizePrimaryTab = (tab: Tab): Tab =>
    tab.type === "assistant" || tab.id === "assistant"
      ? {
          ...tab,
          id: "agentCanvas",
          label: "Agent Canvas",
          type: "agentCanvas",
        }
      : tab;

  const tabMergeKey = (tab: Tab): string => {
    if (
      (tab.type === "agent" || tab.type === "terminal") &&
      typeof tab.paneId === "string" &&
      tab.paneId.length > 0
    ) {
      return `pane:${tab.paneId}`;
    }
    return `id:${tab.id}`;
  };

  const merged = restoredTabs.map((tab) => normalizePrimaryTab(tab));
  const seen = new Set(merged.map(tabMergeKey));

  for (const tab of existingTabs) {
    const normalized = normalizePrimaryTab(tab);
    const key = tabMergeKey(normalized);
    if (seen.has(key)) continue;
    seen.add(key);
    merged.push(normalized);
  }

  if (!merged.some((tab) => tab.id === "agentCanvas")) {
    merged.unshift({
      id: "agentCanvas",
      label: "Agent Canvas",
      type: "agentCanvas",
    });
  }

  if (!merged.some((tab) => tab.id === "branchBrowser")) {
    merged.splice(1, 0, {
      id: "branchBrowser",
      label: "Branch Browser",
      type: "branchBrowser",
    });
  }

  return merged;
}

function isPersistedShellTab(tab: Tab): boolean {
  return (
    tab.type === "agentCanvas" ||
    tab.type === "branchBrowser" ||
    tab.type === "settings" ||
    tab.type === "versionHistory" ||
    tab.type === "issues" ||
    tab.type === "prs" ||
    tab.type === "projectIndex" ||
    tab.type === "issueSpec"
  );
}

export function buildStoredProjectTabsSnapshot(args: {
  tabs: Tab[];
  activeTabId: string;
  selectedCanvasSessionTabId: string | null;
  canvasViewport: AgentCanvasViewport;
  canvasCardLayouts: Record<string, AgentCanvasCardLayout>;
  selectedCanvasCardId: string | null;
  branchBrowserState: StoredBranchBrowserState;
}): StoredProjectTabs {
  const orderedShellTabs = args.tabs.filter((tab) => isPersistedShellTab(tab));
  const seenShellIds = new Set(orderedShellTabs.map((tab) => tab.id));
  const fallbackOrderedTabs =
    orderedShellTabs.length > 0
      ? [...orderedShellTabs, ...args.tabs.filter((tab) => !seenShellIds.has(tab.id))]
      : args.tabs;

  const storedTabs: StoredProjectTab[] = [];
  for (const tab of fallbackOrderedTabs) {
    if (tab.type === "agent") {
      if (typeof tab.paneId !== "string" || tab.paneId.length === 0) continue;
      storedTabs.push({
        type: "agent",
        paneId: tab.paneId,
        label: tab.label,
        ...(tab.branchName ? { branchName: tab.branchName } : {}),
        ...(tab.worktreePath ? { worktreePath: tab.worktreePath } : {}),
        ...(tab.agentId ? { agentId: tab.agentId } : {}),
      });
      continue;
    }
    if (tab.type === "terminal") {
      if (typeof tab.paneId !== "string" || tab.paneId.length === 0) continue;
      storedTabs.push({
        type: "terminal",
        paneId: tab.paneId,
        label: tab.label,
        ...(tab.cwd ? { cwd: tab.cwd } : {}),
        ...(tab.branchName ? { branchName: tab.branchName } : {}),
        ...(tab.worktreePath ? { worktreePath: tab.worktreePath } : {}),
      });
      continue;
    }
    if (isPersistedShellTab(tab)) {
      storedTabs.push({
        type: tab.type as
          | "agentCanvas"
          | "branchBrowser"
          | "settings"
          | "versionHistory"
          | "issues"
          | "prs"
          | "projectIndex"
          | "issueSpec",
        id: tab.id,
        label: tab.label,
        ...(tab.type === "issueSpec" && tab.issueNumber
          ? { issueNumber: tab.issueNumber }
          : {}),
      });
    }
  }

  const storedActiveTabId = storedTabs.some((tab) => {
    if (tab.type === "agent") return `agent-${tab.paneId}` === args.activeTabId;
    if (tab.type === "terminal") return `terminal-${tab.paneId}` === args.activeTabId;
    return tab.id === args.activeTabId;
  })
    ? args.activeTabId
    : fallbackOrderedTabs.find((tab) => isPersistedShellTab(tab))?.id ?? "agentCanvas";

  return {
    tabs: storedTabs,
    activeTabId: storedActiveTabId,
    activeCanvasSessionTabId: args.selectedCanvasSessionTabId,
    agentCanvas: {
      viewport: args.canvasViewport,
      cardLayouts: args.canvasCardLayouts,
      selectedCardId: args.selectedCanvasCardId,
    },
    branchBrowser: args.branchBrowserState,
  };
}

export async function respawnStoredTerminalTabs(args: {
  storedTabs: StoredTerminalTab[];
  targetProjectPath: string;
  token: number;
  currentProjectPath: string | null;
  currentRestoreToken: number;
  terminalTabLabel: (pathLike: string | null | undefined, fallback?: string) => string;
  onError: (message: string, err: unknown) => void;
}): Promise<{ tabs: Tab[]; paneIdMap: Map<string, string> }> {
  if (args.storedTabs.length === 0) {
    return { tabs: [], paneIdMap: new Map() };
  }

  const restoredTabs: Tab[] = [];
  const paneIdMap = new Map<string, string>();

  try {
    const { invoke } = await import("$lib/tauriInvoke");
    for (const storedTab of args.storedTabs) {
      if (
        args.currentProjectPath !== args.targetProjectPath ||
        args.currentRestoreToken !== args.token
      ) {
        break;
      }

      const cwdCandidate = (storedTab.cwd ?? "").trim();
      const workingDir = cwdCandidate || args.targetProjectPath;
      try {
        const paneId = await invoke<string>("spawn_shell", { workingDir });
        paneIdMap.set(storedTab.paneId, paneId);
        restoredTabs.push({
          id: `terminal-${paneId}`,
          label: args.terminalTabLabel(workingDir, storedTab.label || "Terminal"),
          type: "terminal",
          paneId,
          ...(storedTab.branchName ? { branchName: storedTab.branchName } : {}),
          ...(storedTab.worktreePath ? { worktreePath: storedTab.worktreePath } : {}),
          cwd: workingDir,
        });
      } catch (err) {
        args.onError("Failed to respawn stored terminal tab:", err);
      }
    }
  } catch (err) {
    args.onError("Failed to load Tauri API for terminal restore:", err);
  }

  return { tabs: restoredTabs, paneIdMap };
}

export async function restoreProjectTabsRuntime(args: {
  targetProjectPath: string;
  token: number;
  attempt: number;
  startupToken: string | null;
  startupDiagnostics: { disableTabRestore?: boolean } | null;
  currentProjectPath: string | null;
  currentRestoreToken: number;
  activeTabId: string;
  existingTabs: Tab[];
  defaultBranchBrowserState: StoredBranchBrowserState;
  resolveCurrentWindowLabel: () => Promise<string | null>;
  terminalTabLabel: (pathLike: string | null | undefined, fallback?: string) => string;
  resolveCanvasWorktreePath: (branchName?: string | null) => string | null;
  refreshAgentTabLabelsForProject: (projectPath: string) => Promise<void>;
  beginPhase: (startupToken: string | null, phase: StartupProfilePhase) => void;
  finishPhase: (
    startupToken: string | null,
    phase: StartupProfilePhase,
    success: boolean,
  ) => StartupFrontendMetric[];
  flushStartupMetrics: (metrics: StartupFrontendMetric[]) => void;
  onRetry: (nextAttempt: number, delayMs: number) => void;
  baseDelayMs: number;
  maxDelayMs: number;
  maxRetries: number;
  onHydrated: (projectPath: string) => void;
  setTabs: (tabs: Tab[]) => void;
  setCanvasViewport: (viewport: ReturnType<typeof createDefaultAgentCanvasViewport>) => void;
  setCanvasCardLayouts: (layouts: Record<string, unknown>) => void;
  setSelectedCanvasCardId: (cardId: string | null) => void;
  setBranchBrowserState: (state: StoredBranchBrowserState) => void;
  setSelectedCanvasSessionTabId: (tabId: string | null) => void;
  setSelectedCanvasWorktreeBranch: (branchName: string | null) => void;
  setSelectedCanvasWorktreePath: (path: string | null) => void;
  setActiveTabId: (tabId: string) => void;
  onError: (message: string, err: unknown) => void;
}): Promise<void> {
  args.beginPhase(args.startupToken, "restore_project_agent_tabs");
  let success = false;
  if (args.startupDiagnostics?.disableTabRestore) {
    if (
      args.currentProjectPath === args.targetProjectPath &&
      args.currentRestoreToken === args.token
    ) {
      args.onHydrated(args.targetProjectPath);
      args.setCanvasViewport(createDefaultAgentCanvasViewport());
      args.setCanvasCardLayouts({});
      args.setSelectedCanvasCardId(null);
      args.setBranchBrowserState(args.defaultBranchBrowserState);
    }
    success = true;
    args.flushStartupMetrics(
      args.finishPhase(args.startupToken, "restore_project_agent_tabs", success),
    );
    return;
  }

  const windowLabel = await args.resolveCurrentWindowLabel();
  const stored = loadStoredProjectTabs(args.targetProjectPath, undefined, windowLabel);

  if (!stored) {
    if (
      args.currentProjectPath === args.targetProjectPath &&
      args.currentRestoreToken === args.token
    ) {
      args.onHydrated(args.targetProjectPath);
    }
    success = true;
    args.flushStartupMetrics(
      args.finishPhase(args.startupToken, "restore_project_agent_tabs", success),
    );
    return;
  }

  let terminals: TerminalInfo[] = [];
  try {
    const { invoke } = await import("$lib/tauriInvoke");
    terminals = await invoke<TerminalInfo[]>("list_terminals");
  } catch {
    // Ignore: not available outside Tauri runtime.
  }

  if (
    args.currentProjectPath !== args.targetProjectPath ||
    args.currentRestoreToken !== args.token
  ) {
    return;
  }

  const restored = buildRestoredProjectTabs(stored, terminals);
  const storedAgentTabsCount = stored.tabs.filter((tab) => tab.type === "agent").length;
  const restoredAgentTabsCount = restored.tabs.filter(
    (tab: Tab) => tab.type === "agent",
  ).length;
  const shouldRetry = shouldRetryAgentTabRestore(
    storedAgentTabsCount,
    restoredAgentTabsCount,
    args.attempt,
    args.maxRetries,
  );

  if (
    shouldRetry &&
    args.currentProjectPath === args.targetProjectPath &&
    args.currentRestoreToken === args.token
  ) {
    args.onRetry(
      args.attempt + 1,
      getAgentTabRestoreDelayMs(args.attempt, args.baseDelayMs, args.maxDelayMs),
    );
    return;
  }

  const respawnedTerminalResult = await respawnStoredTerminalTabs({
    storedTabs: restored.terminalTabsToRespawn,
    targetProjectPath: args.targetProjectPath,
    token: args.token,
    currentProjectPath: args.currentProjectPath,
    currentRestoreToken: args.currentRestoreToken,
    terminalTabLabel: args.terminalTabLabel,
    onError: args.onError,
  });

  if (
    args.currentProjectPath !== args.targetProjectPath ||
    args.currentRestoreToken !== args.token
  ) {
    return;
  }

  const mergedTabs = mergeRestoredTabs(args.existingTabs, [
    ...restored.tabs,
    ...respawnedTerminalResult.tabs,
  ]);
  args.setTabs(mergedTabs);
  await args.refreshAgentTabLabelsForProject(args.targetProjectPath);

  const allowOverrideActive = shouldAllowRestoredActiveTab(args.activeTabId);
  args.setCanvasViewport(
    restored.agentCanvas?.viewport ?? createDefaultAgentCanvasViewport(),
  );
  args.setCanvasCardLayouts(restored.agentCanvas?.cardLayouts ?? {});
  args.setSelectedCanvasCardId(restored.agentCanvas?.selectedCardId ?? null);
  args.setBranchBrowserState(
    restored.branchBrowser ?? args.defaultBranchBrowserState,
  );
  args.setSelectedCanvasSessionTabId(restored.activeCanvasSessionTabId);
  const restoredCanvasSession = restored.activeCanvasSessionTabId
    ? mergedTabs.find((tab) => tab.id === restored.activeCanvasSessionTabId)
    : null;
  if (restoredCanvasSession?.branchName) {
    args.setSelectedCanvasWorktreeBranch(restoredCanvasSession.branchName);
    args.setSelectedCanvasWorktreePath(
      restoredCanvasSession.worktreePath ??
        args.resolveCanvasWorktreePath(restoredCanvasSession.branchName),
    );
  }
  if (allowOverrideActive) {
    if (
      restored.activeTabId &&
      mergedTabs.some((tab) => tab.id === restored.activeTabId)
    ) {
      args.setActiveTabId(restored.activeTabId);
    } else if (restored.activeCanvasSessionTabId) {
      args.setActiveTabId("agentCanvas");
    } else if (restored.activeTerminalPaneIdToRespawn) {
      const paneId = respawnedTerminalResult.paneIdMap.get(
        restored.activeTerminalPaneIdToRespawn,
      );
      if (paneId) {
        args.setSelectedCanvasSessionTabId(`terminal-${paneId}`);
        args.setActiveTabId("agentCanvas");
      }
    }
  } else if (!mergedTabs.some((tab) => tab.id === args.activeTabId)) {
    args.setActiveTabId(mergedTabs[0]?.id ?? "agentCanvas");
  }

  args.onHydrated(args.targetProjectPath);
  success = true;
  args.flushStartupMetrics(
    args.finishPhase(args.startupToken, "restore_project_agent_tabs", success),
  );
}
