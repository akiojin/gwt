import { describe, expect, it } from "vitest";
import {
  defaultAppTabs,
  reorderTabsByDrop,
  shouldAllowRestoredActiveTab,
} from "./appTabs";
import type { Tab } from "./types";

describe("appTabs", () => {
  it("uses Agent Mode as the only default tab", () => {
    expect(defaultAppTabs()).toEqual([
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
    ]);
  });

  it("does not allow restoring active tab from removed summary tab", () => {
    expect(shouldAllowRestoredActiveTab("summary")).toBe(false);
    expect(shouldAllowRestoredActiveTab("agentMode")).toBe(true);
  });

  it("moves dragged tab before target tab", () => {
    const tabs: Tab[] = [
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];

    expect(reorderTabsByDrop(tabs, "versionHistory", "settings", "before")).toEqual([
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
      { id: "settings", label: "Settings", type: "settings" },
    ]);
  });

  it("moves dragged tab after target tab", () => {
    const tabs: Tab[] = [
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];

    expect(reorderTabsByDrop(tabs, "agentMode", "versionHistory", "after")).toEqual([
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
    ]);
  });

  it("returns the original array when no reorder is needed", () => {
    const tabs: Tab[] = [
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];

    expect(reorderTabsByDrop(tabs, "settings", "versionHistory", "before")).toBe(tabs);
    expect(reorderTabsByDrop(tabs, "unknown", "versionHistory", "before")).toBe(tabs);
    expect(reorderTabsByDrop(tabs, "settings", "settings", "after")).toBe(tabs);
  });
});
