import type { AgentCanvasCardLayout, AgentCanvasViewport } from "./agentCanvas";

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

export interface RenderableCanvasCardRuntime {
  id: string;
  kind: "assistant" | "worktree" | "agent" | "terminal";
  title: string;
  subtitle: string;
  detail: string;
  worktree?: AgentCanvasWorktreeLike;
  tab?: AgentCanvasTabLike;
  worktreeCardId?: string | null;
}

const CARD_WIDTH = 280;
const CARD_HEIGHT = 164;
const SESSION_CARD_WIDTH = 540;
const SESSION_CARD_HEIGHT = 400;
const BOARD_PADDING = 40;
const BOARD_MIN_WIDTH = 1760;
const BOARD_MIN_HEIGHT = 1220;
const SESSION_VISIBILITY_MARGIN = 120;
const SESSION_COLUMN_GAP = 48;
const ROW_GAP = 36;

export function preferredSelectedCardIdRuntime(args: {
  selectedSessionTabId: string | null;
  selectedWorktreeBranch: string | null;
  worktreeCards: Array<{ id: string; worktree: AgentCanvasWorktreeLike }>;
}): string {
  if (args.selectedSessionTabId) return `session:${args.selectedSessionTabId}`;
  if (args.selectedWorktreeBranch) {
    const matching = args.worktreeCards.find(
      (card) => card.worktree.branch === args.selectedWorktreeBranch,
    );
    if (matching) return matching.id;
  }
  return "assistant";
}

export function buildDefaultLayoutsRuntime(
  cards: RenderableCanvasCardRuntime[],
): Record<string, AgentCanvasCardLayout> {
  const next: Record<string, AgentCanvasCardLayout> = {
    assistant: {
      x: BOARD_PADDING,
      y: BOARD_PADDING,
      width: CARD_WIDTH,
      height: CARD_HEIGHT,
    },
  };

  const worktreeRow = new Map<string, number>();
  const sessionBaseX = BOARD_PADDING + CARD_WIDTH + 96;
  const firstWorktreeY = BOARD_PADDING + CARD_HEIGHT + 72;
  const rowStride = SESSION_CARD_HEIGHT + ROW_GAP;
  const worktreeYOffset = Math.round((SESSION_CARD_HEIGHT - CARD_HEIGHT) / 2);
  let rowIndex = 0;
  for (const card of cards) {
    if (card.kind !== "worktree") continue;
    next[card.id] = {
      x: BOARD_PADDING,
      y: firstWorktreeY + rowIndex * rowStride + worktreeYOffset,
      width: CARD_WIDTH,
      height: CARD_HEIGHT,
    };
    worktreeRow.set(card.id, rowIndex);
    rowIndex += 1;
  }

  const sessionCounts = new Map<string, number>();
  let orphanRow = rowIndex;
  for (const card of cards) {
    if (card.kind !== "agent" && card.kind !== "terminal") continue;
    const worktreeId = card.worktreeCardId ?? "";
    const baseRow =
      worktreeId && worktreeRow.has(worktreeId)
        ? worktreeRow.get(worktreeId)!
        : orphanRow++;
    const sessionIndex = sessionCounts.get(worktreeId || card.id) ?? 0;
    sessionCounts.set(worktreeId || card.id, sessionIndex + 1);
    next[card.id] = {
      x: sessionBaseX + sessionIndex * (SESSION_CARD_WIDTH + SESSION_COLUMN_GAP),
      y: firstWorktreeY + baseRow * rowStride,
      width: SESSION_CARD_WIDTH,
      height: SESSION_CARD_HEIGHT,
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
  layout: AgentCanvasCardLayout | undefined,
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
  edges: Array<{ sourceCardId: string; targetCardId: string; id: string }>;
  cardLayouts: Record<string, AgentCanvasCardLayout>;
}) {
  return args.edges
    .map((edge) => {
      const source = args.cardLayouts[edge.sourceCardId];
      const target = args.cardLayouts[edge.targetCardId];
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
  cardLayouts: Record<string, AgentCanvasCardLayout>,
) {
  const layouts = Object.values(cardLayouts);
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

export function buildLiveSessionCardIdsRuntime(args: {
  renderableCards: RenderableCanvasCardRuntime[];
  cardLayouts: Record<string, AgentCanvasCardLayout>;
  boardViewport: { left: number; top: number; right: number; bottom: number };
}) {
  const visible = new Set<string>();
  for (const card of args.renderableCards) {
    if (card.kind !== "agent" && card.kind !== "terminal") continue;
    if (isLayoutVisibleRuntime(args.cardLayouts[card.id], args.boardViewport)) {
      visible.add(card.id);
    }
  }
  return visible;
}
