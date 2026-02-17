import { describe, it, expect } from "vitest";
import { getNextTabId, getPreviousTabId } from "./tabNavigation";
import type { Tab } from "./types";

// Helper to create test tabs
function makeTabs(...types: Array<Tab["type"]>): Tab[] {
  return types.map((type, i) => ({
    id: type === "summary" ? "summary" : `tab-${i}`,
    label: `Tab ${i}`,
    type,
  }));
}

describe("getNextTabId", () => {
  it("returns the next tab in display order", () => {
    const tabs = makeTabs("summary", "agent", "agent");
    expect(getNextTabId(tabs, "summary")).toBe("tab-1");
  });

  it("returns null at the rightmost tab (no wrap)", () => {
    const tabs = makeTabs("summary", "agent", "agent");
    expect(getNextTabId(tabs, "tab-2")).toBeNull();
  });

  it("returns null when only one tab", () => {
    const tabs = makeTabs("summary");
    expect(getNextTabId(tabs, "summary")).toBeNull();
  });

  it("returns null when activeTabId is not found", () => {
    const tabs = makeTabs("summary", "agent");
    expect(getNextTabId(tabs, "nonexistent")).toBeNull();
  });

  it("includes summary tab in navigation", () => {
    const tabs = makeTabs("agent", "summary", "agent");
    expect(getNextTabId(tabs, tabs[0].id)).toBe("summary");
  });

  it("follows D&D reordered tab array order", () => {
    // Simulating a reordered array: agent2 moved before agent1
    const tabs: Tab[] = [
      { id: "summary", label: "Summary", type: "summary" },
      { id: "agent-2", label: "Agent 2", type: "agent" },
      { id: "agent-1", label: "Agent 1", type: "agent" },
    ];
    expect(getNextTabId(tabs, "summary")).toBe("agent-2");
  });

  it("works with terminal tabs", () => {
    const tabs: Tab[] = [
      { id: "summary", label: "Summary", type: "summary" },
      { id: "agent-1", label: "Agent 1", type: "agent" },
      { id: "terminal-1", label: "Terminal", type: "terminal" },
    ];
    expect(getNextTabId(tabs, "agent-1")).toBe("terminal-1");
  });
});

describe("getPreviousTabId", () => {
  it("returns the previous tab in display order", () => {
    const tabs = makeTabs("summary", "agent", "agent");
    expect(getPreviousTabId(tabs, "tab-1")).toBe("summary");
  });

  it("returns null at the leftmost tab (no wrap)", () => {
    const tabs = makeTabs("summary", "agent", "agent");
    expect(getPreviousTabId(tabs, "summary")).toBeNull();
  });

  it("returns null when only one tab", () => {
    const tabs = makeTabs("agent");
    expect(getPreviousTabId(tabs, "tab-0")).toBeNull();
  });

  it("returns null when activeTabId is not found", () => {
    const tabs = makeTabs("summary", "agent");
    expect(getPreviousTabId(tabs, "nonexistent")).toBeNull();
  });

  it("follows D&D reordered tab array order", () => {
    const tabs: Tab[] = [
      { id: "agent-2", label: "Agent 2", type: "agent" },
      { id: "summary", label: "Summary", type: "summary" },
      { id: "agent-1", label: "Agent 1", type: "agent" },
    ];
    expect(getPreviousTabId(tabs, "summary")).toBe("agent-2");
  });
});
