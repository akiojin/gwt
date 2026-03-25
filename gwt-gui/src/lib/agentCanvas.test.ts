import { describe, expect, it } from "vitest";
import {
  buildAgentCanvasGraph,
  buildAgentCanvasState,
  createDefaultAgentCanvasViewport,
} from "./agentCanvas";
import type { Tab, WorktreeInfo } from "./types";

const mainWorktree: WorktreeInfo = {
  path: "/tmp/project",
  branch: "main",
  commit: "main123",
  status: "active",
  is_main: true,
  has_changes: false,
  has_unpushed: false,
  is_current: true,
  is_protected: true,
  is_agent_running: false,
  agent_status: "unknown",
  ahead: 0,
  behind: 0,
  is_gone: false,
  last_tool_usage: null,
  safety_level: "safe",
};

const featureWorktree: WorktreeInfo = {
  ...mainWorktree,
  path: "/tmp/project/.gwt/worktrees/feature-demo",
  branch: "feature/demo",
  is_main: false,
  is_current: false,
  is_protected: false,
  safety_level: "warning",
};

describe("agentCanvas", () => {
  it("binds sessions to worktree identity by path before branch label", () => {
    const tabs: Tab[] = [
      {
        id: "agent-pane-1",
        label: "#1654 Demo",
        type: "agent",
        paneId: "pane-1",
        branchName: "renamed-display-branch",
        worktreePath: featureWorktree.path,
      },
    ];

    const graph = buildAgentCanvasGraph("/tmp/project", "main", tabs, [
      mainWorktree,
      featureWorktree,
    ]);

    expect(graph.sessionTiles[0].worktreeTileId).toBe(`worktree:${featureWorktree.path}`);
    expect(graph.edges).toEqual([
      {
        id: `worktree:${featureWorktree.path}->session:agent-pane-1`,
        sourceTileId: `worktree:${featureWorktree.path}`,
        targetTileId: "session:agent-pane-1",
      },
    ]);
  });

  it("falls back to the current worktree when a session has no explicit branch", () => {
    const tabs: Tab[] = [
      {
        id: "terminal-pane-1",
        label: "Terminal",
        type: "terminal",
        paneId: "pane-1",
      },
    ];

    const graph = buildAgentCanvasGraph("/tmp/project", "main", tabs, [mainWorktree]);

    expect(graph.sessionTiles[0].worktreeTileId).toBe(`worktree:${mainWorktree.path}`);
  });

  it("builds a default viewport-backed canvas state", () => {
    const state = buildAgentCanvasState(
      "/tmp/project",
      "main",
      [],
      [mainWorktree],
      createDefaultAgentCanvasViewport(),
    );

    expect(state.viewport).toEqual({ x: 0, y: 0, zoom: 1 });
    expect(state.tiles.map((tile) => tile.id)).toContain("assistant");
    expect(state.tiles.map((tile) => tile.id)).toContain(`worktree:${mainWorktree.path}`);
  });
});
