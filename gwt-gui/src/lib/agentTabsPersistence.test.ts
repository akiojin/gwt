import { describe, it, expect, beforeEach } from "vitest";
import {
  AGENT_TAB_RESTORE_MAX_RETRIES,
  shouldRetryAgentTabRestore,
  PROJECT_TABS_STORAGE_KEY,
  PROJECT_AGENT_TABS_STORAGE_KEY,
  loadStoredProjectTabs,
  persistStoredProjectTabs,
  persistStoredProjectAgentTabs,
  buildRestoredProjectTabs,
  loadStoredProjectAgentTabs,
  buildRestoredAgentTabs,
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
              {
                type: "terminal",
                paneId: " t1 ",
                label: "term",
                cwd: "/tmp/one",
              },
              { type: "terminal", paneId: "t1", label: "dup term" },
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
        { type: "terminal", paneId: "t1", label: "term", cwd: "/tmp/one" },
        { type: "settings", id: "settings", label: "Settings" },
        {
          type: "versionHistory",
          id: "versionHistory",
          label: "Version History",
        },
      ],
      activeTabId: "settings",
    });
  });

  it("loadStoredProjectTabs falls back to legacy v1 state with terminal support", () => {
    store.setItem(
      PROJECT_AGENT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 1,
        byProjectPath: {
          "/repo": {
            tabs: [
              { paneId: " p1 ", label: "one" },
              {
                paneId: "t1",
                label: "term",
                type: "terminal",
                cwd: "/tmp/term",
              },
            ],
            activePaneId: "t1",
          },
        },
      }),
    );

    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded).toEqual({
      tabs: [
        { type: "agent", paneId: "p1", label: "one" },
        { type: "terminal", paneId: "t1", label: "term", cwd: "/tmp/term" },
      ],
      activeTabId: "terminal-t1",
    });
  });

  it("persistStoredProjectTabs merges by projectPath in v2 storage", () => {
    store.setItem(
      PROJECT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 2,
        byProjectPath: {
          "/a": {
            tabs: [{ type: "assistant", id: "assistant", label: "Assistant" }],
            activeTabId: "assistant",
          },
        },
      }),
    );

    persistStoredProjectTabs(
      "/b",
      {
        tabs: [
          { type: "settings", id: "settings", label: "Settings" },
          { type: "terminal", paneId: "t1", label: "term", cwd: "/tmp" },
        ],
        activeTabId: "terminal-t1",
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
          tabs: [{ type: "assistant", id: "assistant", label: "Assistant" }],
          activeTabId: "assistant",
        },
        "/b": {
          tabs: [
            { type: "settings", id: "settings", label: "Settings" },
            { type: "terminal", paneId: "t1", label: "term", cwd: "/tmp" },
          ],
          activeTabId: "terminal-t1",
        },
      },
    });
  });

  it("persistStoredProjectAgentTabs preserves non-agent tabs in existing entry", () => {
    persistStoredProjectTabs(
      "/repo",
      {
        tabs: [
          { type: "assistant", id: "assistant", label: "Assistant" },
          { type: "terminal", paneId: "t1", label: "term", cwd: "/tmp" },
          { type: "agent", paneId: "old", label: "old-agent" },
        ],
        activeTabId: "terminal-t1",
      },
      store,
    );

    persistStoredProjectAgentTabs(
      "/repo",
      {
        tabs: [{ paneId: "new", label: "new-agent" }],
        activePaneId: null,
      },
      store,
    );

    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded).toEqual({
      tabs: [
        { type: "assistant", id: "assistant", label: "Assistant" },
        { type: "terminal", paneId: "t1", label: "term", cwd: "/tmp" },
        { type: "agent", paneId: "new", label: "new-agent" },
      ],
      activeTabId: "terminal-t1",
    });
  });

  it("buildRestoredProjectTabs restores in stored order and filters missing panes", () => {
    const restored = buildRestoredProjectTabs(
      {
        tabs: [
          { type: "assistant", id: "assistant", label: "Assistant" },
          { type: "settings", id: "settings", label: "Settings" },
          { type: "agent", paneId: "p1", label: "one" },
          {
            type: "versionHistory",
            id: "versionHistory",
            label: "Version History",
          },
          { type: "agent", paneId: "p2", label: "two" },
        ],
        activeTabId: "agent-p2",
      },
      [makeTerminal("p1")],
    );

    expect(restored.tabs).toEqual([
      { id: "assistant", label: "Assistant", type: "assistant" },
      { id: "settings", label: "Settings", type: "settings" },
      {
        id: "agent-p1",
        label: "one",
        type: "agent",
        paneId: "p1",
        agentId: "codex",
      },
      {
        id: "versionHistory",
        label: "Version History",
        type: "versionHistory",
      },
    ]);
    expect(restored.activeTabId).toBeNull();
    expect(restored.terminalTabsToRespawn).toEqual([]);
    expect(restored.activeTerminalPaneIdToRespawn).toBeNull();
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
    expect(restored.terminalTabsToRespawn).toEqual([]);
    expect(restored.activeTerminalPaneIdToRespawn).toBeNull();
    expect(restored.tabs).toEqual([
      { id: "assistant", label: "Assistant", type: "assistant" },
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
      { id: "assistant", label: "Assistant", type: "assistant" },
      { id: "agent-p1", label: "one", type: "agent", paneId: "p1" },
    ]);
    expect(restored.terminalTabsToRespawn).toEqual([]);
    expect(restored.activeTerminalPaneIdToRespawn).toBeNull();
  });

  it("buildRestoredProjectTabs marks missing terminal tabs for respawn", () => {
    const restored = buildRestoredProjectTabs(
      {
        tabs: [
          {
            type: "terminal",
            paneId: "t-old",
            label: "myproject",
            cwd: "/home/user/myproject",
          },
          { type: "agent", paneId: "a-live", label: "feature-a" },
        ],
        activeTabId: "terminal-t-old",
      },
      [makeTerminal("a-live", "codex")],
    );

    expect(restored.tabs).toEqual([
      { id: "assistant", label: "Assistant", type: "assistant" },
      {
        id: "agent-a-live",
        label: "feature-a",
        type: "agent",
        paneId: "a-live",
        agentId: "codex",
      },
    ]);
    expect(restored.activeTabId).toBeNull();
    expect(restored.terminalTabsToRespawn).toEqual([
      {
        type: "terminal",
        paneId: "t-old",
        label: "myproject",
        cwd: "/home/user/myproject",
      },
    ]);
    expect(restored.activeTerminalPaneIdToRespawn).toBe("t-old");
  });

  it("shouldRetryAgentTabRestore handles transient empty matches", () => {
    expect(shouldRetryAgentTabRestore(2, 0, 0)).toBe(true);
    expect(shouldRetryAgentTabRestore(2, 1, 0)).toBe(false);
    expect(shouldRetryAgentTabRestore(0, 0, 0)).toBe(false);
    expect(
      shouldRetryAgentTabRestore(2, 0, AGENT_TAB_RESTORE_MAX_RETRIES - 1),
    ).toBe(false);
  });

  it("loadStoredProjectAgentTabs returns null when no stored tabs", () => {
    expect(loadStoredProjectAgentTabs("/repo", store)).toBeNull();
  });

  it("loadStoredProjectAgentTabs filters only agent tabs", () => {
    persistStoredProjectTabs(
      "/repo",
      {
        tabs: [
          { type: "agent", paneId: "p1", label: "agent-one" },
          { type: "terminal", paneId: "t1", label: "term", cwd: "/tmp" },
          { type: "agent", paneId: "p2", label: "agent-two" },
          { type: "settings", id: "settings", label: "Settings" },
        ],
        activeTabId: "agent-p1",
      },
      store,
    );

    const result = loadStoredProjectAgentTabs("/repo", store);
    expect(result).toEqual({
      tabs: [
        { paneId: "p1", label: "agent-one" },
        { paneId: "p2", label: "agent-two" },
      ],
      activePaneId: "p1",
    });
  });

  it("loadStoredProjectAgentTabs returns null activePaneId for non-agent active tab", () => {
    persistStoredProjectTabs(
      "/repo",
      {
        tabs: [
          { type: "agent", paneId: "p1", label: "agent-one" },
          { type: "settings", id: "settings", label: "Settings" },
        ],
        activeTabId: "settings",
      },
      store,
    );

    const result = loadStoredProjectAgentTabs("/repo", store);
    expect(result).toEqual({
      tabs: [{ paneId: "p1", label: "agent-one" }],
      activePaneId: null,
    });
  });

  it("persistStoredProjectAgentTabs sets agent-prefixed activeTabId", () => {
    persistStoredProjectAgentTabs(
      "/repo",
      {
        tabs: [{ paneId: "p1", label: "agent-one" }],
        activePaneId: "p1",
      },
      store,
    );

    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded?.activeTabId).toBe("agent-p1");
  });

  it("persistStoredProjectAgentTabs clears agent active when activePaneId is null and existing was agent", () => {
    persistStoredProjectTabs(
      "/repo",
      {
        tabs: [{ type: "agent", paneId: "p1", label: "agent-one" }],
        activeTabId: "agent-p1",
      },
      store,
    );

    persistStoredProjectAgentTabs(
      "/repo",
      {
        tabs: [{ paneId: "p1", label: "agent-one" }],
        activePaneId: null,
      },
      store,
    );

    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded?.activeTabId).toBeNull();
  });

  it("persistStoredProjectAgentTabs skips tabs with empty paneId", () => {
    persistStoredProjectAgentTabs(
      "/repo",
      {
        tabs: [
          { paneId: "", label: "empty" },
          { paneId: "p1", label: "valid" },
          { paneId: "  ", label: "whitespace" },
        ],
        activePaneId: null,
      },
      store,
    );

    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded?.tabs).toEqual([
      { type: "agent", paneId: "p1", label: "valid" },
    ]);
  });

  it("buildRestoredAgentTabs filters only agent tabs from restored result", () => {
    const result = buildRestoredAgentTabs(
      {
        tabs: [
          { paneId: "p1", label: "one" },
          { paneId: "p2", label: "two" },
        ],
        activePaneId: "p1",
      },
      [makeTerminal("p1"), makeTerminal("p2")],
    );

    expect(result.tabs.length).toBe(2);
    expect(result.tabs[0].type).toBe("agent");
    expect(result.tabs[0].paneId).toBe("p1");
    expect(result.activeTabId).toBe("agent-p1");
  });

  it("buildRestoredAgentTabs returns null activeTabId when stored has no activePaneId", () => {
    const result = buildRestoredAgentTabs(
      {
        tabs: [{ paneId: "p1", label: "one" }],
        activePaneId: null,
      },
      [makeTerminal("p1")],
    );

    expect(result.tabs.length).toBe(1);
    expect(result.activeTabId).toBeNull();
  });

  it("buildRestoredAgentTabs returns null activeTabId when active tab is not found", () => {
    const result = buildRestoredAgentTabs(
      {
        tabs: [{ paneId: "p1", label: "one" }],
        activePaneId: "missing",
      },
      [makeTerminal("p1")],
    );

    expect(result.activeTabId).toBeNull();
  });

  it("loadStoredProjectTabs returns null for empty projectPath", () => {
    expect(loadStoredProjectTabs("", store)).toBeNull();
    expect(loadStoredProjectTabs("   ", store)).toBeNull();
  });

  it("loadStoredProjectTabs returns null for null storage", () => {
    expect(loadStoredProjectTabs("/repo", null)).toBeNull();
  });

  it("persistStoredProjectTabs skips empty projectPath", () => {
    persistStoredProjectTabs(
      "",
      { tabs: [{ type: "settings", id: "settings", label: "S" }], activeTabId: null },
      store,
    );
    expect(store.getItem(PROJECT_TABS_STORAGE_KEY)).toBeNull();
  });

  it("persistStoredProjectTabs silently ignores corrupt existing JSON", () => {
    store.setItem(PROJECT_TABS_STORAGE_KEY, "not-json{");
    // Should not throw even though existing data is corrupt
    persistStoredProjectTabs(
      "/repo",
      { tabs: [{ type: "settings", id: "settings", label: "S" }], activeTabId: null },
      store,
    );
    // The corrupt JSON prevents both reading and writing
    const loaded = loadStoredProjectTabs("/repo", store);
    // The persist catches the error silently, so the corrupt data remains
    expect(loaded).toBeNull();
  });

  it("persistStoredProjectTabs handles non-v2 existing data", () => {
    store.setItem(PROJECT_TABS_STORAGE_KEY, JSON.stringify({ version: 99 }));
    persistStoredProjectTabs(
      "/repo",
      { tabs: [{ type: "settings", id: "settings", label: "S" }], activeTabId: null },
      store,
    );
    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded).toBeTruthy();
  });

  it("loadStoredProjectTabs returns null for corrupt v2 data", () => {
    store.setItem(PROJECT_TABS_STORAGE_KEY, JSON.stringify({ version: 2 }));
    expect(loadStoredProjectTabs("/repo", store)).toBeNull();
  });

  it("loadStoredProjectTabs returns null when byProjectPath is not an object", () => {
    store.setItem(
      PROJECT_TABS_STORAGE_KEY,
      JSON.stringify({ version: 2, byProjectPath: "invalid" }),
    );
    expect(loadStoredProjectTabs("/repo", store)).toBeNull();
  });

  it("loadStoredProjectTabs handles legacy v1 with agent active pane id", () => {
    store.setItem(
      PROJECT_AGENT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 1,
        byProjectPath: {
          "/repo": {
            tabs: [{ paneId: "p1", label: "one" }],
            activePaneId: "p1",
          },
        },
      }),
    );
    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded?.activeTabId).toBe("agent-p1");
  });

  it("loadStoredProjectTabs returns null for legacy v1 with wrong version", () => {
    store.setItem(
      PROJECT_AGENT_TABS_STORAGE_KEY,
      JSON.stringify({ version: 99, byProjectPath: {} }),
    );
    expect(loadStoredProjectTabs("/repo", store)).toBeNull();
  });

  it("loadStoredProjectTabs returns null for legacy v1 without byProjectPath", () => {
    store.setItem(
      PROJECT_AGENT_TABS_STORAGE_KEY,
      JSON.stringify({ version: 1 }),
    );
    expect(loadStoredProjectTabs("/repo", store)).toBeNull();
  });

  it("loadStoredProjectTabs returns null for legacy v1 with missing project entry", () => {
    store.setItem(
      PROJECT_AGENT_TABS_STORAGE_KEY,
      JSON.stringify({ version: 1, byProjectPath: {} }),
    );
    expect(loadStoredProjectTabs("/repo", store)).toBeNull();
  });

  it("parseStoredProjectTab handles assistant tab type correctly", () => {
    store.setItem(
      PROJECT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 2,
        byProjectPath: {
          "/repo": {
            tabs: [
              { type: "assistant", id: "custom-id", label: "Custom" },
              { type: "issues", id: "", label: "" },
            ],
            activeTabId: null,
          },
        },
      }),
    );
    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded!.tabs).toEqual([
      { type: "assistant", id: "assistant", label: "Assistant" },
      { type: "issues", id: "issues", label: "Issues" },
    ]);
  });

  it("migrates legacy projectMode tab entries and active tab id to assistant", () => {
    store.setItem(
      PROJECT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 2,
        byProjectPath: {
          "/repo": {
            tabs: [
              { type: "projectMode", id: "projectMode", label: "Project Mode" },
              { type: "settings", id: "settings", label: "Settings" },
            ],
            activeTabId: "projectMode",
          },
        },
      }),
    );

    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded).toEqual({
      tabs: [
        { type: "assistant", id: "assistant", label: "Assistant" },
        { type: "settings", id: "settings", label: "Settings" },
      ],
      activeTabId: "assistant",
    });
  });

  it("parseStoredProjectTab handles agent tab with known agentId", () => {
    store.setItem(
      PROJECT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 2,
        byProjectPath: {
          "/repo": {
            tabs: [
              { type: "agent", paneId: "p1", label: "Agent", agentId: "claude" },
              { type: "agent", paneId: "p2", label: "Agent2", agentId: "unknown-id" },
            ],
            activeTabId: null,
          },
        },
      }),
    );
    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded!.tabs[0]).toEqual({ type: "agent", paneId: "p1", label: "Agent", agentId: "claude" });
    expect(loaded!.tabs[1]).toEqual({ type: "agent", paneId: "p2", label: "Agent2" });
  });

  it("parseStoredProjectTab returns null for agent tab without paneId", () => {
    store.setItem(
      PROJECT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 2,
        byProjectPath: {
          "/repo": {
            tabs: [
              { type: "agent", paneId: "", label: "No ID" },
            ],
            activeTabId: null,
          },
        },
      }),
    );
    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded!.tabs).toEqual([]);
  });

  it("parseStoredProjectTab returns null for terminal tab without paneId", () => {
    store.setItem(
      PROJECT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 2,
        byProjectPath: {
          "/repo": {
            tabs: [
              { type: "terminal", paneId: "", label: "No ID" },
            ],
            activeTabId: null,
          },
        },
      }),
    );
    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded!.tabs).toEqual([]);
  });

  it("buildRestoredProjectTabs preserves assistant activeTabId normalization", () => {
    const restored = buildRestoredProjectTabs(
      {
        tabs: [
          { type: "assistant", id: "assistant", label: "Assistant" },
        ],
        activeTabId: "assistant",
      },
      [],
    );
    expect(restored.activeTabId).toBe("assistant");
  });

  it("buildRestoredProjectTabs deduplicates tabs by key", () => {
    const restored = buildRestoredProjectTabs(
      {
        tabs: [
          { type: "agent", paneId: "p1", label: "first" },
          { type: "agent", paneId: "p1", label: "duplicate" },
          { type: "settings", id: "settings", label: "S1" },
          { type: "settings", id: "settings", label: "S2" },
        ],
        activeTabId: null,
      },
      [makeTerminal("p1")],
    );
    const agentTabs = restored.tabs.filter((t) => t.type === "agent");
    expect(agentTabs).toHaveLength(1);
    expect(agentTabs[0].label).toBe("first");

    const settingsTabs = restored.tabs.filter((t) => t.type === "settings");
    expect(settingsTabs).toHaveLength(1);
  });

  it("buildRestoredProjectTabs handles terminal active tab with no pane to respawn", () => {
    const restored = buildRestoredProjectTabs(
      {
        tabs: [
          { type: "terminal", paneId: "t1", label: "term" },
        ],
        activeTabId: "terminal-t1",
      },
      [{ pane_id: "t1", agent_name: "", branch_name: "", status: "running" }],
    );
    expect(restored.activeTabId).toBe("terminal-t1");
    expect(restored.activeTerminalPaneIdToRespawn).toBeNull();
  });

  it("persistStoredProjectAgentTabs preserves non-agent active tab when activePaneId is null", () => {
    persistStoredProjectTabs(
      "/repo",
      {
        tabs: [
          { type: "terminal", paneId: "t1", label: "term" },
          { type: "agent", paneId: "p1", label: "agent" },
        ],
        activeTabId: "terminal-t1",
      },
      store,
    );

    persistStoredProjectAgentTabs(
      "/repo",
      {
        tabs: [{ paneId: "p1", label: "agent" }],
        activePaneId: null,
      },
      store,
    );

    const loaded = loadStoredProjectTabs("/repo", store);
    expect(loaded?.activeTabId).toBe("terminal-t1");
  });

  it("buildRestoredProjectTabs uses stored agentId as fallback when terminal agent is unknown", () => {
    store.setItem(
      PROJECT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 2,
        byProjectPath: {
          "/repo": {
            tabs: [
              { type: "agent", paneId: "p1", label: "agent", agentId: "gemini" },
            ],
            activeTabId: null,
          },
        },
      }),
    );

    const stored = loadStoredProjectTabs("/repo", store)!;
    const restored = buildRestoredProjectTabs(
      stored,
      [makeTerminal("p1", "unknown-agent")],
    );
    expect(restored.tabs.find((t) => t.paneId === "p1")?.agentId).toBe("gemini");
  });

  it("buildRestoredProjectTabs includes terminal with cwd", () => {
    const restored = buildRestoredProjectTabs(
      {
        tabs: [
          { type: "terminal", paneId: "t1", label: "term", cwd: "/home" },
        ],
        activeTabId: null,
      },
      [{ pane_id: "t1", agent_name: "", branch_name: "", status: "running" }],
    );
    const termTab = restored.tabs.find((t) => t.paneId === "t1");
    expect(termTab?.cwd).toBe("/home");
  });
});
