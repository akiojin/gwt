import { describe, expect, it } from "vitest";
import {
  addTabToActiveGroup,
  canSplitTab,
  createInitialTabLayout,
  flattenTabIdsByLayout,
  moveTabToGroup,
  normalizeTabLayoutState,
  removeTabFromLayout,
  reorderTabsInGroup,
  setActiveGroup,
  splitTabToGroupEdge,
} from "./tabLayout";

describe("tabLayout", () => {
  it("creates a single-group layout from flat tabs", () => {
    const layout = createInitialTabLayout(
      [{ id: "assistant" }, { id: "agent-1" }, { id: "terminal-1" }],
      "agent-1",
    );

    const group = layout.groups[layout.activeGroupId];
    expect(group?.tabIds).toEqual(["assistant", "agent-1", "terminal-1"]);
    expect(group?.activeTabId).toBe("agent-1");
  });

  it("adds new tabs to the active group", () => {
    const layout = createInitialTabLayout([{ id: "assistant" }], "assistant");
    const next = addTabToActiveGroup(layout, "settings");

    expect(next.groups[next.activeGroupId]?.tabIds).toEqual([
      "assistant",
      "settings",
    ]);
    expect(next.groups[next.activeGroupId]?.activeTabId).toBe("settings");
  });

  it("reorders tabs within a group", () => {
    const layout = createInitialTabLayout(
      [{ id: "assistant" }, { id: "settings" }, { id: "issues" }],
      "assistant",
    );
    const next = reorderTabsInGroup(
      layout,
      layout.activeGroupId,
      "issues",
      "assistant",
      "before",
    );

    expect(next.groups[next.activeGroupId]?.tabIds).toEqual([
      "issues",
      "assistant",
      "settings",
    ]);
  });

  it("prevents split when the source group only has one tab", () => {
    const layout = createInitialTabLayout([{ id: "assistant" }], "assistant");
    expect(canSplitTab(layout, "assistant")).toBe(false);
    expect(
      splitTabToGroupEdge(layout, "assistant", layout.activeGroupId, "right"),
    ).toEqual(layout);
  });

  it("splits a tab into a new adjacent group", () => {
    const layout = createInitialTabLayout(
      [{ id: "assistant" }, { id: "agent-1" }],
      "assistant",
    );
    const next = splitTabToGroupEdge(
      layout,
      "agent-1",
      layout.activeGroupId,
      "right",
    );

    expect(Object.values(next.groups)).toHaveLength(2);
    expect(flattenTabIdsByLayout(next)).toEqual(["assistant", "agent-1"]);
    expect(next.activeGroupId).not.toBe(layout.activeGroupId);
    expect(next.groups[next.activeGroupId]?.tabIds).toEqual(["agent-1"]);
    expect(next.root.type).toBe("split");
  });

  it("moves a tab into another group and collapses an emptied source group", () => {
    const layout = createInitialTabLayout(
      [{ id: "assistant" }, { id: "agent-1" }, { id: "terminal-1" }],
      "assistant",
    );
    const split = splitTabToGroupEdge(
      layout,
      "terminal-1",
      layout.activeGroupId,
      "right",
    );
    const sourceGroupId = split.groups[split.activeGroupId]?.tabIds.includes("terminal-1")
      ? Object.keys(split.groups).find((id) => id !== split.activeGroupId) ?? ""
      : split.activeGroupId;
    const targetGroupId = split.activeGroupId;

    const moved = moveTabToGroup(
      split,
      "assistant",
      targetGroupId,
      "terminal-1",
      "before",
    );
    const collapsed = removeTabFromLayout(moved, "agent-1");

    expect(flattenTabIdsByLayout(collapsed)).toEqual(["assistant", "terminal-1"]);
    expect(Object.values(collapsed.groups)).toHaveLength(1);
    expect(collapsed.root.type).toBe("group");
    expect(collapsed.groups[collapsed.activeGroupId]?.tabIds).toEqual([
      "assistant",
      "terminal-1",
    ]);
    expect(collapsed.activeGroupId).not.toBe(sourceGroupId);
  });

  it("can retarget the active group explicitly", () => {
    const layout = createInitialTabLayout(
      [{ id: "assistant" }, { id: "agent-1" }],
      "assistant",
    );
    const split = splitTabToGroupEdge(
      layout,
      "agent-1",
      layout.activeGroupId,
      "right",
    );
    const otherGroupId =
      Object.keys(split.groups).find((id) => id !== split.activeGroupId) ?? "";
    const next = setActiveGroup(split, otherGroupId);

    expect(next.activeGroupId).toBe(otherGroupId);
  });

  it("collapses a dangling split tree back to one full group", () => {
    const layout = createInitialTabLayout(
      [{ id: "assistant" }, { id: "settings" }, { id: "issues" }],
      "assistant",
    );
    const split = splitTabToGroupEdge(
      layout,
      "issues",
      layout.activeGroupId,
      "right",
    );
    const survivingGroupId =
      Object.keys(split.groups).find((id) => id !== split.activeGroupId) ??
      split.activeGroupId;

    const normalized = normalizeTabLayoutState({
      ...split,
      root: {
        type: "split",
        id: "split-corrupt",
        axis: "horizontal",
        sizes: [0.5, 0.5],
        children: [
          { type: "group", groupId: survivingGroupId },
          { type: "group", groupId: "missing-group" },
        ],
      },
    });

    expect(normalized.root).toEqual({
      type: "group",
      groupId: survivingGroupId,
    });
    expect(Object.keys(normalized.groups)).toEqual([survivingGroupId]);
    expect(normalized.groups[survivingGroupId]?.tabIds).toEqual([
      "assistant",
      "settings",
      "issues",
    ]);
    expect(normalized.activeGroupId).toBe(survivingGroupId);
  });
});
