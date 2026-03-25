import type { AgentCanvasTileLayout, AgentCanvasViewport } from "./agentCanvas";

export interface AgentCanvasWorktreeLike {
  branch?: string | null;
  path: string;
}

export interface AgentCanvasTabLike {
  id: string;
  label: string;
  branchName?: string;
  worktreePath?: string;
  cwd?: string;
}

export interface RenderableCanvasTileRuntime {
  id: string;
  kind: "assistant" | "worktree" | "agent" | "terminal";
  title: string;
  subtitle: string;
  detail: string;
  worktree?: AgentCanvasWorktreeLike;
  tab?: AgentCanvasTabLike;
  worktreeTileId?: string | null;
}

const TILE_WIDTH = 280;
const TILE_HEIGHT = 164;
const SESSION_TILE_WIDTH = 540;
const SESSION_TILE_HEIGHT = 400;
const BOARD_PADDING = 40;
const BOARD_MIN_WIDTH = 1760;
const BOARD_MIN_HEIGHT = 1220;
const SESSION_VISIBILITY_MARGIN = 120;
const SESSION_COLUMN_GAP = 48;
const ROW_GAP = 36;

export function preferredSelectedTileIdRuntime(args: {
  selectedSessionTabId: string | null;
  selectedWorktreeBranch: string | null;
  worktreeTiles: Array<{ id: string; worktree: AgentCanvasWorktreeLike }>;
}): string {
  if (args.selectedSessionTabId) return `session:${args.selectedSessionTabId}`;
  if (args.selectedWorktreeBranch) {
    const matching = args.worktreeTiles.find(
      (tile) => tile.worktree.branch === args.selectedWorktreeBranch,
    );
    if (matching) return matching.id;
  }
  return "assistant";
}

export function buildDefaultLayoutsRuntime(
  tiles: RenderableCanvasTileRuntime[],
): Record<string, AgentCanvasTileLayout> {
  const next: Record<string, AgentCanvasTileLayout> = {
    assistant: {
      x: BOARD_PADDING,
      y: BOARD_PADDING,
      width: TILE_WIDTH,
      height: TILE_HEIGHT,
    },
  };

  const worktreeRow = new Map<string, number>();
  const sessionBaseX = BOARD_PADDING + TILE_WIDTH + 96;
  const firstWorktreeY = BOARD_PADDING + TILE_HEIGHT + 72;
  const rowStride = SESSION_TILE_HEIGHT + ROW_GAP;
  const worktreeYOffset = Math.round((SESSION_TILE_HEIGHT - TILE_HEIGHT) / 2);
  let rowIndex = 0;
  for (const tile of tiles) {
    if (tile.kind !== "worktree") continue;
    next[tile.id] = {
      x: BOARD_PADDING,
      y: firstWorktreeY + rowIndex * rowStride + worktreeYOffset,
      width: TILE_WIDTH,
      height: TILE_HEIGHT,
    };
    worktreeRow.set(tile.id, rowIndex);
    rowIndex += 1;
  }

  const sessionCounts = new Map<string, number>();
  let orphanRow = rowIndex;
  for (const tile of tiles) {
    if (tile.kind !== "agent" && tile.kind !== "terminal") continue;
    const worktreeId = tile.worktreeTileId ?? "";
    const baseRow =
      worktreeId && worktreeRow.has(worktreeId)
        ? worktreeRow.get(worktreeId)!
        : orphanRow++;
    const sessionIndex = sessionCounts.get(worktreeId || tile.id) ?? 0;
    sessionCounts.set(worktreeId || tile.id, sessionIndex + 1);
    next[tile.id] = {
      x: sessionBaseX + sessionIndex * (SESSION_TILE_WIDTH + SESSION_COLUMN_GAP),
      y: firstWorktreeY + baseRow * rowStride,
      width: SESSION_TILE_WIDTH,
      height: SESSION_TILE_HEIGHT,
    };
  }

  return next;
}

export function buildBoardViewportRuntime(args: {
  scrollLeft: number;
  scrollTop: number;
  clientWidth: number;
  clientHeight: number;
  viewport: AgentCanvasViewport;
}) {
  const left = (args.scrollLeft - args.viewport.x) / args.viewport.zoom;
  const top = (args.scrollTop - args.viewport.y) / args.viewport.zoom;
  const right = left + args.clientWidth / args.viewport.zoom;
  const bottom = top + args.clientHeight / args.viewport.zoom;
  return { left, top, right, bottom };
}

export function isLayoutVisibleRuntime(
  layout: AgentCanvasTileLayout | undefined,
  boardViewport: { left: number; top: number; right: number; bottom: number },
): boolean {
  if (!layout) return false;
  return !(
    layout.x + layout.width < boardViewport.left - SESSION_VISIBILITY_MARGIN ||
    layout.x > boardViewport.right + SESSION_VISIBILITY_MARGIN ||
    layout.y + layout.height < boardViewport.top - SESSION_VISIBILITY_MARGIN ||
    layout.y > boardViewport.bottom + SESSION_VISIBILITY_MARGIN
  );
}

export function buildEdgeLinesRuntime(args: {
  edges: Array<{ sourceTileId: string; targetTileId: string; id: string }>;
  tileLayouts: Record<string, AgentCanvasTileLayout>;
}) {
  return args.edges
    .map((edge) => {
      const source = args.tileLayouts[edge.sourceTileId];
      const target = args.tileLayouts[edge.targetTileId];
      if (!source || !target) return null;
      return {
        ...edge,
        x1: source.x + source.width,
        y1: source.y + source.height / 2,
        x2: target.x,
        y2: target.y + target.height / 2,
      };
    })
    .filter((edge): edge is NonNullable<typeof edge> => edge !== null);
}

export function buildSurfaceBoundsRuntime(
  tileLayouts: Record<string, AgentCanvasTileLayout>,
) {
  const layouts = Object.values(tileLayouts);
  const width = layouts.reduce(
    (max, layout) => Math.max(max, layout.x + layout.width + BOARD_PADDING),
    BOARD_MIN_WIDTH,
  );
  const height = layouts.reduce(
    (max, layout) => Math.max(max, layout.y + layout.height + BOARD_PADDING),
    BOARD_MIN_HEIGHT,
  );
  return { width, height };
}

export function buildLiveSessionTileIdsRuntime(args: {
  renderableTiles: RenderableCanvasTileRuntime[];
  tileLayouts: Record<string, AgentCanvasTileLayout>;
  boardViewport: { left: number; top: number; right: number; bottom: number };
}) {
  const visible = new Set<string>();
  for (const tile of args.renderableTiles) {
    if (tile.kind !== "agent" && tile.kind !== "terminal") continue;
    if (isLayoutVisibleRuntime(args.tileLayouts[tile.id], args.boardViewport)) {
      visible.add(tile.id);
    }
  }
  return visible;
}
