import { describe, expect, it } from "vitest";
import {
  defaultAppTabs,
  reorderTabsByDrop,
  shouldAllowRestoredActiveTab,
} from "./appTabs";
import type { Tab } from "./types";

describe("appTabs", () => {
  it("uses Agent Canvas and Branch Browser as the default top-level tabs", () => {
    expect(defaultAppTabs()).toEqual([
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "branchBrowser", label: "Branch Browser", type: "branchBrowser" },
    ]);
  });

  it("allows restoring shell-owned default tabs only", () => {
    expect(shouldAllowRestoredActiveTab("summary")).toBe(false);
    expect(shouldAllowRestoredActiveTab("agentCanvas")).toBe(true);
    expect(shouldAllowRestoredActiveTab("branchBrowser")).toBe(true);
    expect(shouldAllowRestoredActiveTab("legacyMode")).toBe(false);
  });

  it("moves dragged tab before target tab", () => {
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];

    expect(reorderTabsByDrop(tabs, "versionHistory", "settings", "before")).toEqual([
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
      { id: "settings", label: "Settings", type: "settings" },
    ]);
  });

  it("moves dragged tab after target tab", () => {
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];

    expect(reorderTabsByDrop(tabs, "agentCanvas", "versionHistory", "after")).toEqual([
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
    ]);
  });

  it("returns the original array when no reorder is needed", () => {
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];

    expect(reorderTabsByDrop(tabs, "settings", "versionHistory", "before")).toBe(tabs);
    expect(reorderTabsByDrop(tabs, "unknown", "versionHistory", "before")).toBe(tabs);
    expect(reorderTabsByDrop(tabs, "settings", "settings", "after")).toBe(tabs);
  });

  it("returns original array when overTabId not found", () => {
    const tabs: Tab[] = [
      { id: "a", label: "A", type: "agentCanvas" },
      { id: "b", label: "B", type: "settings" },
    ];
    expect(reorderTabsByDrop(tabs, "a", "missing", "before")).toBe(tabs);
  });

  it("returns original array when dragTabId not found", () => {
    const tabs: Tab[] = [
      { id: "a", label: "A", type: "agentCanvas" },
      { id: "b", label: "B", type: "settings" },
    ];
    expect(reorderTabsByDrop(tabs, "missing", "b", "before")).toBe(tabs);
  });
});
