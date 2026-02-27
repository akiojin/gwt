import { describe, expect, it } from "vitest";
import {
  defaultAppTabs,
  reorderTabsByDrop,
  shouldAllowRestoredActiveTab,
} from "./appTabs";
import type { Tab } from "./types";

describe("appTabs", () => {
  it("uses Project Mode as the only default tab", () => {
    expect(defaultAppTabs()).toEqual([
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
    ]);
  });

  it("does not allow restoring active tab from removed summary tab", () => {
    expect(shouldAllowRestoredActiveTab("summary")).toBe(false);
    expect(shouldAllowRestoredActiveTab("projectMode")).toBe(true);
    expect(shouldAllowRestoredActiveTab("legacyMode")).toBe(false);
  });

  it("moves dragged tab before target tab", () => {
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];

    expect(reorderTabsByDrop(tabs, "versionHistory", "settings", "before")).toEqual([
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
      { id: "settings", label: "Settings", type: "settings" },
    ]);
  });

  it("moves dragged tab after target tab", () => {
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];

    expect(reorderTabsByDrop(tabs, "projectMode", "versionHistory", "after")).toEqual([
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
    ]);
  });

  it("returns the original array when no reorder is needed", () => {
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];

    expect(reorderTabsByDrop(tabs, "settings", "versionHistory", "before")).toBe(tabs);
    expect(reorderTabsByDrop(tabs, "unknown", "versionHistory", "before")).toBe(tabs);
    expect(reorderTabsByDrop(tabs, "settings", "settings", "after")).toBe(tabs);
  });

  it("returns original array when overTabId not found", () => {
    const tabs: Tab[] = [
      { id: "a", label: "A", type: "projectMode" },
      { id: "b", label: "B", type: "settings" },
    ];
    expect(reorderTabsByDrop(tabs, "a", "missing", "before")).toBe(tabs);
  });

  it("returns original array when dragTabId not found", () => {
    const tabs: Tab[] = [
      { id: "a", label: "A", type: "projectMode" },
      { id: "b", label: "B", type: "settings" },
    ];
    expect(reorderTabsByDrop(tabs, "missing", "b", "before")).toBe(tabs);
  });
});
