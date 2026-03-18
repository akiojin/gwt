import { describe, expect, it } from "vitest";
import type { BranchInfo, Tab } from "./types";
import {
  findAgentTabByBranchName,
  isRawWorktreeBranch,
  normalizeBranchName,
  resolveWorktreeTabLabel,
  syncAgentTabLabels,
} from "./worktreeTabLabels";

const branchFixture: BranchInfo = {
  name: "feature/issue-1644",
  display_name: "#1644 Worktree管理",
  commit: "1234567",
  is_current: false,
  is_agent_running: false,
  agent_status: "unknown",
  ahead: 0,
  behind: 0,
  divergence_status: "UpToDate",
  last_tool_usage: null,
};

describe("worktreeTabLabels", () => {
  it("normalizes origin prefix only", () => {
    expect(normalizeBranchName("origin/feature/x")).toBe("feature/x");
    expect(normalizeBranchName("develop")).toBe("develop");
  });

  it("keeps base branches raw", () => {
    expect(isRawWorktreeBranch("main")).toBe(true);
    expect(isRawWorktreeBranch("origin/develop")).toBe(true);
    expect(isRawWorktreeBranch("feature/issue-1644")).toBe(false);
  });

  it("resolves tab label from display_name when available", () => {
    expect(resolveWorktreeTabLabel("feature/issue-1644", [branchFixture])).toBe(
      "#1644 Worktree管理",
    );
  });

  it("keeps raw name for base branches even if display_name exists", () => {
    expect(
      resolveWorktreeTabLabel("develop", [
        { ...branchFixture, name: "develop", display_name: "Should not use" },
      ]),
    ).toBe("develop");
  });

  it("syncs only agent tabs with branch identity", () => {
    const tabs: Tab[] = [
      {
        id: "agent-1",
        label: "feature/issue-1644",
        type: "agent",
        paneId: "pane-1",
        branchName: "feature/issue-1644",
      },
      {
        id: "terminal-1",
        label: "Terminal",
        type: "terminal",
        paneId: "pane-2",
      },
    ];

    expect(syncAgentTabLabels(tabs, [branchFixture])).toEqual([
      {
        id: "agent-1",
        label: "#1644 Worktree管理",
        type: "agent",
        paneId: "pane-1",
        branchName: "feature/issue-1644",
      },
      {
        id: "terminal-1",
        label: "Terminal",
        type: "terminal",
        paneId: "pane-2",
      },
    ]);
  });

  it("finds agent tab by branch identity instead of label", () => {
    const tabs: Tab[] = [
      {
        id: "agent-1",
        label: "#1644 Worktree管理",
        type: "agent",
        paneId: "pane-1",
        branchName: "feature/issue-1644",
      },
    ];

    expect(findAgentTabByBranchName(tabs, "feature/issue-1644")?.id).toBe(
      "agent-1",
    );
  });
});
