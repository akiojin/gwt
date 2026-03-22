import type { Tab, WorktreeInfo } from "./types";

export type AgentCanvasCardType = "assistant" | "worktree" | "agent" | "terminal";
export type AgentCanvasViewport = {
  x: number;
  y: number;
  zoom: number;
};

export type AgentCanvasWorktreeCard = {
  id: string;
  type: "worktree";
  worktree: WorktreeInfo;
};

export type AgentCanvasSessionCard = {
  id: string;
  type: "agent" | "terminal";
  tab: Tab;
  worktreeCardId: string | null;
};

export type AgentCanvasEdge = {
  id: string;
  sourceCardId: string;
  targetCardId: string;
};

export type AgentCanvasCard =
  | AgentCanvasWorktreeCard
  | AgentCanvasSessionCard
  | {
      id: "assistant";
      type: "assistant";
    };

export type AgentCanvasGraph = {
  worktrees: WorktreeInfo[];
  worktreeCards: AgentCanvasWorktreeCard[];
  sessionCards: AgentCanvasSessionCard[];
  edges: AgentCanvasEdge[];
};

export type AgentCanvasState = {
  viewport: AgentCanvasViewport;
  cards: AgentCanvasCard[];
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
  const worktreeCards = normalizedWorktrees.map((worktree) => ({
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
  const sessionCards = sessionTabs.map((tab) => {
    const matchedWorktree =
      (tab.worktreePath ? worktreeByPath.get(tab.worktreePath) : null) ??
      (tab.branchName ? worktreeByBranch.get(tab.branchName.trim()) : null) ??
      (!tab.branchName && currentWorktree ? currentWorktree : null);
    return {
      id: `session:${tab.id}`,
      type: tab.type,
      tab,
      worktreeCardId: matchedWorktree ? `worktree:${matchedWorktree.path}` : null,
    } satisfies AgentCanvasSessionCard;
  });

  const edges = sessionCards
    .filter((card) => card.worktreeCardId !== null)
    .map((card) => ({
      id: `${card.worktreeCardId!}->${card.id}`,
      sourceCardId: card.worktreeCardId!,
      targetCardId: card.id,
    }));

  return {
    worktrees: normalizedWorktrees,
    worktreeCards,
    sessionCards,
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
    cards: [
      { id: "assistant", type: "assistant" },
      ...graph.worktreeCards,
      ...graph.sessionCards,
    ],
    edges: graph.edges,
    graph,
  };
}
