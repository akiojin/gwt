import { describe, expect, it } from "vitest";
import type { Tab } from "./types";
import {
  buildWindowMenuTabsSignature,
  buildWindowMenuVisibleTabs,
  resolveActiveWindowMenuTabId,
} from "./windowMenuSync";

describe("windowMenuSync", () => {
  it("buildWindowMenuVisibleTabs includes only agent/terminal tabs", () => {
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      {
        id: "agent-pane-1",
        label: "feature/one",
        type: "agent",
        paneId: "pane-1",
        agentId: "codex",
      },
      {
        id: "terminal-pane-2",
        label: "Terminal",
        type: "terminal",
        paneId: "pane-2",
      },
      { id: "settings", label: "Settings", type: "settings" },
    ];

    expect(buildWindowMenuVisibleTabs(tabs)).toEqual([
      { id: "agent-pane-1", label: "feature/one", tab_type: "agent" },
      { id: "terminal-pane-2", label: "Terminal", tab_type: "terminal" },
    ]);
  });

  it("buildWindowMenuTabsSignature is stable for identical payloads", () => {
    const payload = [
      { id: "a", label: "A", tab_type: "agent" as const },
      { id: "t", label: "T", tab_type: "terminal" as const },
    ];

    const first = buildWindowMenuTabsSignature(payload);
    const second = buildWindowMenuTabsSignature(payload);

    expect(first).toBe(second);
  });

  it("buildWindowMenuTabsSignature changes when tab payload changes", () => {
    const baseline = [
      { id: "a", label: "A", tab_type: "agent" as const },
    ];
    const renamed = [
      { id: "a", label: "A2", tab_type: "agent" as const },
    ];

    expect(buildWindowMenuTabsSignature(baseline)).not.toBe(
      buildWindowMenuTabsSignature(renamed),
    );
  });

  it("resolveActiveWindowMenuTabId returns null for non-visible active tab", () => {
    const visibleTabs = [
      { id: "agent-1", label: "one", tab_type: "agent" as const },
    ];

    expect(resolveActiveWindowMenuTabId(visibleTabs, "agent-1")).toBe("agent-1");
    expect(resolveActiveWindowMenuTabId(visibleTabs, "settings")).toBeNull();
  });
});

