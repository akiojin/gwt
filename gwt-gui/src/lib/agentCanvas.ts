import type { Tab, WorktreeInfo } from "./types";

export type AgentCanvasTileType = "assistant" | "worktree" | "agent" | "terminal";
export type AgentCanvasViewport = {
  x: number;
  y: number;
  zoom: number;
};

export type AgentCanvasTileLayout = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type AgentCanvasPersistedState = {
  viewport: AgentCanvasViewport;
  tileLayouts: Record<string, AgentCanvasTileLayout>;
  selectedTileId: string | null;
};

export type AgentCanvasWorktreeTile = {
  id: string;
  type: "worktree";
  worktree: WorktreeInfo;
};

export type AgentCanvasSessionTile = {
  id: string;
  type: "agent" | "terminal";
  tab: Tab;
  worktreeTileId: string | null;
};

export type AgentCanvasEdge = {
  id: string;
  sourceTileId: string;
  targetTileId: string;
};

export type AgentCanvasTile =
  | AgentCanvasWorktreeTile
  | AgentCanvasSessionTile
  | {
      id: "assistant";
      type: "assistant";
    };

export type AgentCanvasGraph = {
  worktrees: WorktreeInfo[];
  worktreeTiles: AgentCanvasWorktreeTile[];
  sessionTiles: AgentCanvasSessionTile[];
  edges: AgentCanvasEdge[];
};

export type AgentCanvasState = {
  viewport: AgentCanvasViewport;
  tiles: AgentCanvasTile[];
  edges: AgentCanvasEdge[];
  graph: AgentCanvasGraph;
};

export function createDefaultAgentCanvasViewport(): AgentCanvasViewport {
  return {
    x: 0,
    y: 0,
    zoom: 1,
  };
}

function fallbackWorktree(projectPath: string, currentBranch: string): WorktreeInfo {
  return {
    path: projectPath,
    branch: currentBranch || "Project Root",
    commit: "",
    status: "active",
    is_main: false,
    has_changes: false,
    has_unpushed: false,
    is_current: true,
    is_protected: false,
    is_agent_running: false,
    agent_status: "unknown",
    ahead: 0,
    behind: 0,
    is_gone: false,
    last_tool_usage: null,
    safety_level: "safe",
  };
}

export function buildAgentCanvasGraph(
  projectPath: string,
  currentBranch: string,
  tabs: Tab[],
  worktrees: WorktreeInfo[],
): AgentCanvasGraph {
  const normalizedWorktrees = worktrees.length > 0 ? worktrees : [fallbackWorktree(projectPath, currentBranch)];
  const worktreeTiles = normalizedWorktrees.map((worktree) => ({
    id: `worktree:${worktree.path}`,
    type: "worktree" as const,
    worktree,
  }));
  const worktreeByPath = new Map(normalizedWorktrees.map((worktree) => [worktree.path, worktree]));
  const worktreeByBranch = new Map(
    normalizedWorktrees.map((worktree) => [(worktree.branch ?? "").trim(), worktree]),
  );
  const currentWorktree = normalizedWorktrees.find((worktree) => worktree.is_current) ?? normalizedWorktrees[0] ?? null;

  const sessionTabs = tabs.filter(
    (tab): tab is Tab & { type: "agent" | "terminal" } =>
      tab.type === "agent" || tab.type === "terminal",
  );
  const sessionTiles = sessionTabs.map((tab) => {
    const matchedWorktree =
      (tab.worktreePath ? worktreeByPath.get(tab.worktreePath) : null) ??
      (tab.branchName ? worktreeByBranch.get(tab.branchName.trim()) : null) ??
      (!tab.branchName && currentWorktree ? currentWorktree : null);
    return {
      id: `session:${tab.id}`,
      type: tab.type,
      tab,
      worktreeTileId: matchedWorktree ? `worktree:${matchedWorktree.path}` : null,
    } satisfies AgentCanvasSessionTile;
  });

  const edges = sessionTiles
    .filter((tile) => tile.worktreeTileId !== null)
    .map((tile) => ({
      id: `${tile.worktreeTileId!}->${tile.id}`,
      sourceTileId: tile.worktreeTileId!,
      targetTileId: tile.id,
    }));

  return {
    worktrees: normalizedWorktrees,
    worktreeTiles,
    sessionTiles,
    edges,
  };
}

export function buildAgentCanvasState(
  projectPath: string,
  currentBranch: string,
  tabs: Tab[],
  worktrees: WorktreeInfo[],
  viewport: AgentCanvasViewport = createDefaultAgentCanvasViewport(),
): AgentCanvasState {
  const graph = buildAgentCanvasGraph(projectPath, currentBranch, tabs, worktrees);
  return {
    viewport,
    tiles: [
      { id: "assistant", type: "assistant" },
      ...graph.worktreeTiles,
      ...graph.sessionTiles,
    ],
    edges: graph.edges,
    graph,
  };
}
