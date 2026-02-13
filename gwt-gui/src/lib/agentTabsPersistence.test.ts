import { describe, it, expect, beforeEach } from "vitest";
import {
  AGENT_TAB_RESTORE_MAX_RETRIES,
  shouldRetryAgentTabRestore,
  PROJECT_AGENT_TABS_STORAGE_KEY,
  loadStoredProjectAgentTabs,
  persistStoredProjectAgentTabs,
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

  it("loadStoredProjectAgentTabs returns null for missing entry", () => {
    expect(loadStoredProjectAgentTabs("/repo", store)).toBeNull();
  });

  it("loadStoredProjectAgentTabs trims and deduplicates paneIds", () => {
    store.setItem(
      PROJECT_AGENT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 1,
        byProjectPath: {
          "/repo": {
            tabs: [
              { paneId: " p1 ", label: "one" },
              { paneId: "p1", label: "dup" },
              { paneId: "", label: "empty" },
              { paneId: "p2", label: 123 },
              "invalid",
            ],
            activePaneId: " p2 ",
          },
        },
      }),
    );

    const loaded = loadStoredProjectAgentTabs("/repo", store);
    expect(loaded).toEqual({
      tabs: [
        { paneId: "p1", label: "one" },
        { paneId: "p2", label: "" },
      ],
      activePaneId: "p2",
    });
  });

  it("persistStoredProjectAgentTabs merges by projectPath", () => {
    store.setItem(
      PROJECT_AGENT_TABS_STORAGE_KEY,
      JSON.stringify({
        version: 1,
        byProjectPath: {
          "/a": { tabs: [{ paneId: "pa", label: "A" }], activePaneId: "pa" },
        },
      }),
    );

    persistStoredProjectAgentTabs("/b", {
      tabs: [{ paneId: "pb", label: "B" }],
      activePaneId: null,
    }, store);

    const raw = store.getItem(PROJECT_AGENT_TABS_STORAGE_KEY);
    expect(raw).toBeTruthy();
    const parsed = JSON.parse(raw || "{}");
    expect(parsed).toEqual({
      version: 1,
      byProjectPath: {
        "/a": { tabs: [{ paneId: "pa", label: "A" }], activePaneId: "pa" },
        "/b": { tabs: [{ paneId: "pb", label: "B" }], activePaneId: null },
      },
    });
  });

  it("buildRestoredAgentTabs restores only existing panes and clears missing active", () => {
    const restored = buildRestoredAgentTabs(
      {
        tabs: [
          { paneId: "p1", label: "one" },
          { paneId: "p2", label: "two" },
        ],
        activePaneId: "p2",
      },
      [makeTerminal("p1")],
    );

    expect(restored.tabs).toEqual([
      {
        id: "agent-p1",
        label: "one",
        type: "agent",
        paneId: "p1",
        agentId: "codex",
      },
    ]);
    expect(restored.activeTabId).toBeNull();
  });

  it("buildRestoredAgentTabs restores active when it exists", () => {
    const restored = buildRestoredAgentTabs(
      {
        tabs: [
          { paneId: "p1", label: "one" },
          { paneId: "p2", label: "two" },
        ],
        activePaneId: "p2",
      },
      [makeTerminal("p1"), makeTerminal("p2")],
    );

    expect(restored.tabs.length).toBe(2);
    expect(restored.activeTabId).toBe("agent-p2");
    expect(restored.tabs).toEqual([
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

  it("buildRestoredAgentTabs leaves unknown agent names without agentId", () => {
    const restored = buildRestoredAgentTabs(
      {
        tabs: [{ paneId: "p1", label: "one" }],
        activePaneId: null,
      },
      [makeTerminal("p1", "unknown-agent")],
    );

    expect(restored.tabs).toEqual([
      { id: "agent-p1", label: "one", type: "agent", paneId: "p1" },
    ]);
  });

  it("shouldRetryAgentTabRestore handles transient empty matches", () => {
    expect(shouldRetryAgentTabRestore(2, 0, 0)).toBe(true);
    expect(shouldRetryAgentTabRestore(2, 1, 0)).toBe(false);
    expect(shouldRetryAgentTabRestore(0, 0, 0)).toBe(false);
    expect(shouldRetryAgentTabRestore(2, 0, AGENT_TAB_RESTORE_MAX_RETRIES - 1)).toBe(false);
  });

  it("shouldRetryAgentTabRestore handles transient empty matches", () => {
    expect(shouldRetryAgentTabRestore(2, 0, 0)).toBe(true);
    expect(shouldRetryAgentTabRestore(2, 1, 0)).toBe(false);
    expect(shouldRetryAgentTabRestore(0, 0, 0)).toBe(false);
    expect(shouldRetryAgentTabRestore(2, 0, AGENT_TAB_RESTORE_MAX_RETRIES - 1)).toBe(false);
  });
});
