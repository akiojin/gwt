import { describe, it, expect, beforeEach } from "vitest";
import {
  AGENT_TAB_RESTORE_MAX_RETRIES,
  shouldRetryAgentTabRestore,
  PROJECT_TABS_STORAGE_KEY,
  PROJECT_AGENT_TABS_STORAGE_KEY,
  loadStoredProjectTabs,
  persistStoredProjectTabs,
  buildRestoredProjectTabs,
} from "./agentTabsPersistence";

class MemoryStorage implements Storage {
  private data = new Map<string, string>();

  get length(): number {
    return this.data.size;
  }

  clear(): void {
    this.data.clear();
  }

  getItem(key: string): string | null {
    return this.data.has(key) ? this.data.get(key)! : null;
  }

  key(index: number): string | null {
    return Array.from(this.data.keys())[index] ?? null;
  }

  removeItem(key: string): void {
    this.data.delete(key);
  }

  setItem(key: string, value: string): void {
    this.data.set(key, value);
  }
}

function makeTerminal(paneId: string, agentName = "codex") {
  return {
    pane_id: paneId,
    agent_name: agentName,
    branch_name: "feature/x",
    status: "running",
  };
}

describe("agentTabsPersistence", () => {
  let store: MemoryStorage;

  beforeEach(() => {
    store = new MemoryStorage();
  });

  it("loadStoredProjectTabs returns null for missing entry", () => {
    expect(loadStoredProjectTabs("/repo", store)).toBeNull();
  });

  it("loadStoredProjectTabs trims and deduplicates tabs in v2 format", () => {
    store.setItem(
      PROJECT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 2,
        byProjectPath: {
          "/repo": {
            tabs: [
              { type: "agent", paneId: " p1 ", label: "one" },
              { type: "agent", paneId: "p1", label: "dup" },
              { type: "settings", id: "settings", label: "Settings" },
              { type: "settings", id: "settings", label: "dup settings" },
              { type: "versionHistory", id: "", label: 123 },
              "invalid",
            ],
            activeTabId: " settings ",
          },
        },
      }),
    );

    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded).toEqual({
      tabs: [
        { type: "agent", paneId: "p1", label: "one" },
        { type: "settings", id: "settings", label: "Settings" },
        { type: "versionHistory", id: "versionHistory", label: "Version History" },
      ],
      activeTabId: "settings",
    });
  });

  it("loadStoredProjectTabs falls back to legacy v1 agent-tab state", () => {
    store.setItem(
      PROJECT_AGENT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 1,
        byProjectPath: {
          "/repo": {
            tabs: [{ paneId: " p1 ", label: "one" }, { paneId: "p1", label: "dup" }],
            activePaneId: " p1 ",
          },
        },
      }),
    );

    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded).toEqual({
      tabs: [{ type: "agent", paneId: "p1", label: "one" }],
      activeTabId: "agent-p1",
    });
  });

  it("persistStoredProjectTabs merges by projectPath in v2 storage", () => {
    store.setItem(
      PROJECT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 2,
        byProjectPath: {
          "/a": {
            tabs: [{ type: "agentMode", id: "agentMode", label: "Agent Mode" }],
            activeTabId: "agentMode",
          },
        },
      }),
    );

    persistStoredProjectTabs(
      "/b",
      {
        tabs: [{ type: "settings", id: "settings", label: "Settings" }],
        activeTabId: "settings",
      },
      store,
    );

    const raw = store.getItem(PROJECT_TABS_STORAGE_KEY);
    expect(raw).toBeTruthy();
    const parsed = JSON.parse(raw || "{}");
    expect(parsed).toEqual({
      version: 2,
      byProjectPath: {
        "/a": {
          tabs: [{ type: "agentMode", id: "agentMode", label: "Agent Mode" }],
          activeTabId: "agentMode",
        },
        "/b": {
          tabs: [{ type: "settings", id: "settings", label: "Settings" }],
          activeTabId: "settings",
        },
      },
    });
  });

  it("buildRestoredProjectTabs restores in stored order and filters missing panes", () => {
    const restored = buildRestoredProjectTabs(
      {
        tabs: [
          { type: "agentMode", id: "agentMode", label: "Agent Mode" },
          { type: "settings", id: "settings", label: "Settings" },
          { type: "agent", paneId: "p1", label: "one" },
          { type: "versionHistory", id: "versionHistory", label: "Version History" },
          { type: "agent", paneId: "p2", label: "two" },
        ],
        activeTabId: "agent-p2",
      },
      [makeTerminal("p1")],
    );

    expect(restored.tabs).toEqual([
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "settings", label: "Settings", type: "settings" },
      {
        id: "agent-p1",
        label: "one",
        type: "agent",
        paneId: "p1",
        agentId: "codex",
      },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ]);
    expect(restored.activeTabId).toBeNull();
  });

  it("buildRestoredProjectTabs restores active when it exists", () => {
    const restored = buildRestoredProjectTabs(
      {
        tabs: [
          { type: "settings", id: "settings", label: "Settings" },
          { type: "agent", paneId: "p1", label: "one" },
          { type: "agent", paneId: "p2", label: "two" },
        ],
        activeTabId: "settings",
      },
      [makeTerminal("p1"), makeTerminal("p2")],
    );

    expect(restored.tabs.length).toBe(4);
    expect(restored.activeTabId).toBe("settings");
    expect(restored.tabs).toEqual([
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      {
        id: "settings",
        label: "Settings",
        type: "settings",
      },
      {
        id: "agent-p1",
        label: "one",
        type: "agent",
        paneId: "p1",
        agentId: "codex",
      },
      {
        id: "agent-p2",
        label: "two",
        type: "agent",
        paneId: "p2",
        agentId: "codex",
      },
    ]);
  });

  it("buildRestoredProjectTabs leaves unknown agent names without agentId", () => {
    const restored = buildRestoredProjectTabs(
      {
        tabs: [{ type: "agent", paneId: "p1", label: "one" }],
        activeTabId: null,
      },
      [makeTerminal("p1", "unknown-agent")],
    );

    expect(restored.tabs).toEqual([
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "agent-p1", label: "one", type: "agent", paneId: "p1" },
    ]);
  });

  it("shouldRetryAgentTabRestore handles transient empty matches", () => {
    expect(shouldRetryAgentTabRestore(2, 0, 0)).toBe(true);
    expect(shouldRetryAgentTabRestore(2, 1, 0)).toBe(false);
    expect(shouldRetryAgentTabRestore(0, 0, 0)).toBe(false);
    expect(shouldRetryAgentTabRestore(2, 0, AGENT_TAB_RESTORE_MAX_RETRIES - 1)).toBe(false);
  });
});
