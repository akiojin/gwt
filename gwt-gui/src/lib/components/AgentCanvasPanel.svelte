<script lang="ts">
  import {
    type AgentCanvasCardLayout,
    buildAgentCanvasGraph,
    createDefaultAgentCanvasViewport,
    type AgentCanvasViewport,
  } from "../agentCanvas";
  import type { Tab, WorktreeInfo } from "../types";
  import AssistantPanel from "./AssistantPanel.svelte";
  import TerminalView from "../terminal/TerminalView.svelte";

  type CanvasCardKind = "assistant" | "worktree" | "agent" | "terminal";

  type RenderableCanvasCard = {
    id: string;
    kind: CanvasCardKind;
    title: string;
    subtitle: string;
    detail: string;
    worktree?: WorktreeInfo;
    tab?: Tab;
    worktreeCardId?: string | null;
  };

  type PointerDragState = {
    cardId: string;
    pointerId: number;
    startX: number;
    startY: number;
    originX: number;
    originY: number;
  };

  type PointerPanState = {
    pointerId: number;
    startX: number;
    startY: number;
    originX: number;
    originY: number;
  };

  const CARD_WIDTH = 280;
  const CARD_HEIGHT = 164;
  const BOARD_PADDING = 40;
  const BOARD_MIN_WIDTH = 1180;
  const BOARD_MIN_HEIGHT = 820;
  const MIN_ZOOM = 0.7;
  const MAX_ZOOM = 1.6;
  const ZOOM_STEP = 0.1;

  let {
    projectPath,
    currentBranch = "",
    tabs,
    worktrees = [],
    selectedWorktreeBranch = null,
    onWorktreeSelect = () => {},
    selectedSessionTabId = null,
    onSessionSelect = () => {},
    persistedViewport = undefined,
    persistedCardLayouts = undefined,
    persistedSelectedCardId = null,
    onViewportChange = () => {},
    onCardLayoutsChange = () => {},
    onSelectedCardChange = () => {},
    onOpenSettings,
    voiceInputEnabled = false,
    voiceInputListening = false,
    voiceInputPreparing = false,
    voiceInputSupported = true,
    voiceInputAvailable = false,
    voiceInputAvailabilityReason = null,
    voiceInputError = null,
  }: {
    projectPath: string;
    currentBranch?: string;
    tabs: Tab[];
    worktrees?: WorktreeInfo[];
    selectedWorktreeBranch?: string | null;
    onWorktreeSelect?: (branchName: string) => void;
    selectedSessionTabId?: string | null;
    onSessionSelect?: (tabId: string) => void;
    persistedViewport?: AgentCanvasViewport | undefined;
    persistedCardLayouts?: Record<string, AgentCanvasCardLayout> | undefined;
    persistedSelectedCardId?: string | null;
    onViewportChange?: (viewport: AgentCanvasViewport) => void;
    onCardLayoutsChange?: (layouts: Record<string, AgentCanvasCardLayout>) => void;
    onSelectedCardChange?: (cardId: string | null) => void;
    onOpenSettings?: () => void;
    voiceInputEnabled?: boolean;
    voiceInputListening?: boolean;
    voiceInputPreparing?: boolean;
    voiceInputSupported?: boolean;
    voiceInputAvailable?: boolean;
    voiceInputAvailabilityReason?: string | null;
    voiceInputError?: string | null;
  } = $props();

  let graph = $derived(buildAgentCanvasGraph(projectPath, currentBranch, tabs, worktrees));
  let canvasWorktrees = $derived(graph.worktrees);
  let popupWorktreeBranch = $state<string | null>(null);
  let worktreeDetailsOpen = $state(false);
  let viewport = $state<AgentCanvasViewport>(createDefaultAgentCanvasViewport());
  let cardLayouts = $state<Record<string, AgentCanvasCardLayout>>({});
  let selectedCardId = $state<string>("assistant");
  let dragState = $state<PointerDragState | null>(null);
  let panState = $state<PointerPanState | null>(null);
  let boardEl = $state<HTMLDivElement | null>(null);
  let lastSeedKey = $state("");
  let lastViewportEmitKey = $state("");
  let lastLayoutsEmitKey = $state("");
  let lastSelectedEmitKey = $state("");

  function worktreeCardTestId(worktree: WorktreeInfo): string {
    const raw = worktree.branch ?? "project-root";
    return `agent-canvas-worktree-card-${raw.replace(/[^a-zA-Z0-9_-]+/g, "-")}`;
  }

  function edgeTestId(cardId: string): string {
    return `agent-canvas-edge-${cardId.replace(/[^a-zA-Z0-9_-]+/g, "-")}`;
  }

  let sessionsByWorktreeId = $derived.by(() => {
    const mapped = new Map<string, (typeof graph.sessionCards)>();
    for (const card of graph.sessionCards) {
      if (!card.worktreeCardId) continue;
      const existing = mapped.get(card.worktreeCardId) ?? [];
      mapped.set(card.worktreeCardId, [...existing, card]);
    }
    return mapped;
  });

  let renderableCards = $derived.by<RenderableCanvasCard[]>(() => {
    const cards: RenderableCanvasCard[] = [
      {
        id: "assistant",
        kind: "assistant",
        title: "Assistant",
        subtitle: "Workspace assistant",
        detail: "Use the detail pane to interact without stretching the canvas.",
      },
    ];

    for (const worktreeCard of graph.worktreeCards) {
      const sessionCount = sessionsByWorktreeId.get(worktreeCard.id)?.length ?? 0;
      cards.push({
        id: worktreeCard.id,
        kind: "worktree",
        title: worktreeCard.worktree.branch || "Project Root",
        subtitle: sessionCount === 1 ? "1 linked session" : `${sessionCount} linked sessions`,
        detail: worktreeCard.worktree.path,
        worktree: worktreeCard.worktree,
      });
    }

    for (const sessionCard of graph.sessionCards) {
      cards.push({
        id: sessionCard.id,
        kind: sessionCard.type,
        title: sessionCard.tab.label,
        subtitle: sessionCard.type === "agent" ? "Agent session" : "Terminal session",
        detail:
          sessionCard.tab.branchName ||
          sessionCard.tab.worktreePath ||
          sessionCard.tab.cwd ||
          "No branch context",
        tab: sessionCard.tab,
        worktreeCardId: sessionCard.worktreeCardId,
      });
    }

    return cards;
  });

  function clampZoom(value: number): number {
    return Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, value));
  }

  function preferredSelectedCardId(): string {
    if (selectedSessionTabId) return `session:${selectedSessionTabId}`;
    if (selectedWorktreeBranch) {
      const matching = graph.worktreeCards.find(
        (card) => card.worktree.branch === selectedWorktreeBranch,
      );
      if (matching) return matching.id;
    }
    return "assistant";
  }

  function buildDefaultLayouts(cards: RenderableCanvasCard[]): Record<string, AgentCanvasCardLayout> {
    const next: Record<string, AgentCanvasCardLayout> = {
      assistant: {
        x: BOARD_PADDING,
        y: BOARD_PADDING,
        width: CARD_WIDTH,
        height: CARD_HEIGHT,
      },
    };

    const worktreeRow = new Map<string, number>();
    let rowIndex = 0;
    for (const card of cards) {
      if (card.kind !== "worktree") continue;
      next[card.id] = {
        x: BOARD_PADDING,
        y: BOARD_PADDING + CARD_HEIGHT + 60 + rowIndex * (CARD_HEIGHT + 28),
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
      const baseRow = worktreeId && worktreeRow.has(worktreeId) ? worktreeRow.get(worktreeId)! : orphanRow++;
      const sessionIndex = sessionCounts.get(worktreeId || card.id) ?? 0;
      sessionCounts.set(worktreeId || card.id, sessionIndex + 1);
      next[card.id] = {
        x: BOARD_PADDING + CARD_WIDTH + 80 + sessionIndex * (CARD_WIDTH + 40),
        y: BOARD_PADDING + CARD_HEIGHT + 60 + baseRow * (CARD_HEIGHT + 28),
        width: CARD_WIDTH,
        height: CARD_HEIGHT,
      };
    }

    return next;
  }

  $effect(() => {
    const seedKey = JSON.stringify([
      projectPath,
      persistedViewport ?? null,
      persistedCardLayouts ?? null,
      persistedSelectedCardId ?? null,
    ]);
    if (seedKey !== lastSeedKey) {
      lastSeedKey = seedKey;
      viewport = persistedViewport ?? createDefaultAgentCanvasViewport();
      cardLayouts = persistedCardLayouts ? { ...persistedCardLayouts } : {};
      if (persistedSelectedCardId) {
        selectedCardId = persistedSelectedCardId;
      }
    }

    const defaults = buildDefaultLayouts(renderableCards);
    const nextLayouts: Record<string, AgentCanvasCardLayout> = {};
    let needsLayoutSync = Object.keys(cardLayouts).length !== renderableCards.length;
    for (const card of renderableCards) {
      const existing = cardLayouts[card.id];
      if (!existing) needsLayoutSync = true;
      nextLayouts[card.id] = existing ?? defaults[card.id];
    }
    if (needsLayoutSync) {
      cardLayouts = nextLayouts;
    }

    const preferred = preferredSelectedCardId();
    const fallbackSelected =
      preferred in nextLayouts ? preferred : renderableCards[0]?.id ?? "assistant";
    if (!(selectedCardId in nextLayouts)) {
      if (selectedCardId !== fallbackSelected) {
        selectedCardId = fallbackSelected;
      }
    } else if (
      selectedSessionTabId &&
      preferred in nextLayouts &&
      selectedCardId !== preferred
    ) {
      selectedCardId = preferred;
    }
  });

  $effect(() => {
    const key = JSON.stringify(viewport);
    if (key === lastViewportEmitKey) return;
    lastViewportEmitKey = key;
    onViewportChange(viewport);
  });

  $effect(() => {
    const key = JSON.stringify(cardLayouts);
    if (key === lastLayoutsEmitKey) return;
    lastLayoutsEmitKey = key;
    onCardLayoutsChange(cardLayouts);
  });

  $effect(() => {
    const key = selectedCardId || "";
    if (key === lastSelectedEmitKey) return;
    lastSelectedEmitKey = key;
    onSelectedCardChange(selectedCardId || null);
  });

  let popupWorktree = $derived(
    popupWorktreeBranch
      ? canvasWorktrees.find((worktree) => worktree.branch === popupWorktreeBranch) ?? null
      : null,
  );

  let selectedCard = $derived.by(
    () => renderableCards.find((card) => card.id === selectedCardId) ?? renderableCards[0] ?? null,
  );

  let edgeLines = $derived.by(() =>
    graph.edges
      .map((edge) => {
        const source = cardLayouts[edge.sourceCardId];
        const target = cardLayouts[edge.targetCardId];
        if (!source || !target) return null;
        return {
          ...edge,
          x1: source.x + source.width,
          y1: source.y + source.height / 2,
          x2: target.x,
          y2: target.y + target.height / 2,
        };
      })
      .filter((edge): edge is NonNullable<typeof edge> => edge !== null),
  );

  let surfaceBounds = $derived.by(() => {
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
  });

  let zoomPercent = $derived(Math.round(viewport.zoom * 100));

  function updateViewportZoom(nextZoom: number) {
    viewport = {
      ...viewport,
      zoom: clampZoom(nextZoom),
    };
  }

  function zoomIn() {
    updateViewportZoom(viewport.zoom + ZOOM_STEP);
  }

  function zoomOut() {
    updateViewportZoom(viewport.zoom - ZOOM_STEP);
  }

  function resetViewport() {
    viewport = createDefaultAgentCanvasViewport();
  }

  function selectCard(card: RenderableCanvasCard) {
    selectedCardId = card.id;
    if (card.kind === "worktree" && card.worktree?.branch) {
      onWorktreeSelect(card.worktree.branch);
    } else if ((card.kind === "agent" || card.kind === "terminal") && card.tab) {
      onSessionSelect(card.tab.id);
    }
  }

  function activateWorktreeCard(worktree: WorktreeInfo) {
    if (worktree.branch) {
      onWorktreeSelect(worktree.branch);
      popupWorktreeBranch = worktree.branch;
    }
    const cardId = `worktree:${worktree.path}`;
    selectedCardId = cardId;
    worktreeDetailsOpen = true;
  }

  function handleCardKeydown(card: RenderableCanvasCard, event: KeyboardEvent) {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    if (card.kind === "worktree" && card.worktree) {
      activateWorktreeCard(card.worktree);
      return;
    }
    selectCard(card);
  }

  function beginCardDrag(cardId: string, event: PointerEvent) {
    if (event.button !== 0) return;
    const layout = cardLayouts[cardId];
    if (!layout) return;
    event.preventDefault();
    event.stopPropagation();
    selectedCardId = cardId;
    dragState = {
      cardId,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      originX: layout.x,
      originY: layout.y,
    };
    boardEl?.setPointerCapture?.(event.pointerId);
  }

  function beginPan(event: PointerEvent) {
    if (event.button !== 0) return;
    if (event.target !== event.currentTarget) return;
    panState = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      originX: viewport.x,
      originY: viewport.y,
    };
    boardEl?.setPointerCapture?.(event.pointerId);
  }

  function handlePointerMove(event: PointerEvent) {
    if (dragState && event.pointerId === dragState.pointerId) {
      const dx = (event.clientX - dragState.startX) / viewport.zoom;
      const dy = (event.clientY - dragState.startY) / viewport.zoom;
      const current = cardLayouts[dragState.cardId];
      if (!current) return;
      cardLayouts = {
        ...cardLayouts,
        [dragState.cardId]: {
          ...current,
          x: Math.max(BOARD_PADDING, dragState.originX + dx),
          y: Math.max(BOARD_PADDING, dragState.originY + dy),
        },
      };
      return;
    }

    if (panState && event.pointerId === panState.pointerId) {
      viewport = {
        ...viewport,
        x: panState.originX + (event.clientX - panState.startX),
        y: panState.originY + (event.clientY - panState.startY),
      };
    }
  }

  function clearPointerState(event?: PointerEvent) {
    if (event) {
      boardEl?.releasePointerCapture?.(event.pointerId);
    }
    dragState = null;
    panState = null;
  }

  function handleWheel(event: WheelEvent) {
    if (!event.metaKey && !event.ctrlKey) return;
    event.preventDefault();
    const direction = event.deltaY < 0 ? ZOOM_STEP : -ZOOM_STEP;
    updateViewportZoom(viewport.zoom + direction);
  }
</script>

<div class="agent-canvas">
  <div class="canvas-toolbar">
    <div>
      <h2>Agent Canvas</h2>
      <p>Drag cards on the board and open the selected card in the detail pane.</p>
    </div>
    <div class="toolbar-group">
      <div class="toolbar-chip">Cards: {renderableCards.length}</div>
      <div class="zoom-controls" data-testid="agent-canvas-zoom-controls">
        <button type="button" class="zoom-btn" onclick={zoomOut} aria-label="Zoom out">-</button>
        <button type="button" class="zoom-btn ghost" onclick={resetViewport}>
          <span data-testid="agent-canvas-zoom-label">{zoomPercent}%</span>
        </button>
        <button type="button" class="zoom-btn" onclick={zoomIn} aria-label="Zoom in">+</button>
      </div>
    </div>
  </div>

  <div class="canvas-shell">
    <section class="canvas-board-panel">
      <div
        class="canvas-board"
        bind:this={boardEl}
        role="presentation"
        data-testid="agent-canvas-board"
        onpointerdown={beginPan}
        onpointermove={handlePointerMove}
        onpointerup={clearPointerState}
        onpointercancel={clearPointerState}
        onwheel={handleWheel}
      >
        <div
          class="canvas-surface"
          style={`width:${surfaceBounds.width}px;height:${surfaceBounds.height}px;transform: translate(${viewport.x}px, ${viewport.y}px) scale(${viewport.zoom});`}
        >
          <svg
            class="canvas-edges"
            viewBox={`0 0 ${surfaceBounds.width} ${surfaceBounds.height}`}
            width={surfaceBounds.width}
            height={surfaceBounds.height}
            aria-hidden="true"
          >
            {#each edgeLines as edge (edge.id)}
              <line
                x1={edge.x1}
                y1={edge.y1}
                x2={edge.x2}
                y2={edge.y2}
                class="edge-line"
                data-testid={edgeTestId(edge.targetCardId)}
              ></line>
            {/each}
          </svg>

          {#each renderableCards as card (card.id)}
            <div
              class="canvas-card"
              class:selected={selectedCardId === card.id}
              class:assistant-card={card.kind === "assistant"}
              class:worktree-card={card.kind === "worktree"}
              class:session-card={card.kind === "agent" || card.kind === "terminal"}
              role="button"
              tabindex="0"
              data-testid={
                card.kind === "assistant"
                  ? "agent-canvas-assistant-card"
                  : card.kind === "worktree" && card.worktree
                    ? worktreeCardTestId(card.worktree)
                    : card.tab
                      ? `agent-canvas-session-${card.tab.id}`
                      : `agent-canvas-card-${card.id}`
              }
              style={`transform: translate(${cardLayouts[card.id]?.x ?? 0}px, ${cardLayouts[card.id]?.y ?? 0}px); width:${cardLayouts[card.id]?.width ?? CARD_WIDTH}px; height:${cardLayouts[card.id]?.height ?? CARD_HEIGHT}px;`}
              onclick={() => {
                if (card.kind === "worktree" && card.worktree) {
                  activateWorktreeCard(card.worktree);
                } else {
                  selectCard(card);
                }
              }}
              onkeydown={(event) => handleCardKeydown(card, event)}
            >
              <div class="card-header">
                <div class="card-heading">
                  <span class="card-kind">{card.kind === "agent" ? "Agent" : card.kind === "terminal" ? "Terminal" : card.kind === "worktree" ? "Worktree" : "Assistant"}</span>
                  <span class="card-title">{card.title}</span>
                </div>
                <button
                  type="button"
                  class="card-drag-handle"
                  aria-label="Drag card"
                  onpointerdown={(event) => beginCardDrag(card.id, event)}
                  onclick={(event) => event.stopPropagation()}
                >
                  ::
                </button>
              </div>

              <div class="card-body">
                <p class="card-subtitle">{card.subtitle}</p>
                <p class="card-copy">{card.detail}</p>
                {#if card.kind === "worktree" && card.worktree}
                  <div class="card-footer">
                    <span class="card-status">{card.worktree.safety_level}</span>
                    <span>{sessionsByWorktreeId.get(card.id)?.length ?? 0} sessions</span>
                  </div>
                {:else if card.kind === "agent" || card.kind === "terminal"}
                  <div class="card-footer">
                    <span class="card-status">{card.kind === "agent" ? "Interactive" : "Shell"}</span>
                    <span>{card.tab?.paneId ? "Live detail available" : "Waiting for pane"}</span>
                  </div>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      </div>
    </section>

    <section class="canvas-detail" data-testid="agent-canvas-detail">
      {#if selectedCard?.kind === "assistant"}
        <AssistantPanel
          isActive={true}
          {projectPath}
          onOpenSettings={onOpenSettings ?? (() => {})}
        />
      {:else if (selectedCard?.kind === "agent" || selectedCard?.kind === "terminal") && selectedCard.tab}
        {#if selectedCard.tab.paneId}
          <TerminalView
            paneId={selectedCard.tab.paneId}
            active={true}
            agentId={selectedCard.kind === "agent" ? selectedCard.tab.agentId ?? null : null}
            {voiceInputEnabled}
            {voiceInputListening}
            {voiceInputPreparing}
            {voiceInputSupported}
            {voiceInputAvailable}
            {voiceInputAvailabilityReason}
            {voiceInputError}
          />
        {:else}
          <div class="detail-placeholder">
            {selectedCard.kind === "agent" ? "Agent starting..." : "Terminal starting..."}
          </div>
        {/if}
      {:else if selectedCard?.kind === "worktree" && selectedCard.worktree}
        <div class="worktree-detail-card">
          <div class="detail-header">
            <span class="detail-kind">Worktree</span>
            <h3>{selectedCard.worktree.branch || currentBranch || "Project Root"}</h3>
          </div>
          <div class="detail-grid">
            <div class="detail-row">
              <span class="detail-label">Project</span>
              <span class="detail-value">{projectPath}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Branch</span>
              <span class="detail-value">{selectedCard.worktree.branch || currentBranch || "Project Root"}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Worktree Path</span>
              <span class="detail-value">{selectedCard.worktree.path}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Linked Sessions</span>
              <span class="detail-value">{sessionsByWorktreeId.get(selectedCard.id)?.length ?? 0}</span>
            </div>
          </div>
          <button
            type="button"
            class="detail-action"
            onclick={() => activateWorktreeCard(selectedCard.worktree!)}
          >
            Open popup
          </button>
        </div>
      {/if}
    </section>
  </div>

  {#if worktreeDetailsOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="worktree-overlay"
      data-testid="agent-canvas-worktree-overlay"
      onclick={() => (worktreeDetailsOpen = false)}
    >
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="worktree-dialog"
        data-testid="agent-canvas-worktree-dialog"
        role="dialog"
        aria-modal="true"
        tabindex="0"
        onclick={(event) => event.stopPropagation()}
      >
        <div class="dialog-header">
          <span class="card-kind">Worktree</span>
          <span class="card-title">{popupWorktree?.branch || currentBranch || "Project Root"}</span>
        </div>
        <div class="dialog-body">
          <div class="detail-row">
            <span class="detail-label">Project</span>
            <span class="detail-value">{projectPath}</span>
          </div>
          <div class="detail-row">
            <span class="detail-label">Branch</span>
            <span class="detail-value">{popupWorktree?.branch || currentBranch || "Project Root"}</span>
          </div>
          <div class="detail-row">
            <span class="detail-label">Sessions</span>
            <span class="detail-value">{graph.sessionCards.filter((card) => popupWorktree ? card.worktreeCardId === `worktree:${popupWorktree.path}` : true).length}</span>
          </div>
          <div class="detail-row">
            <span class="detail-label">Worktree Path</span>
            <span class="detail-value">{popupWorktree?.path || projectPath}</span>
          </div>
        </div>
        <button type="button" class="dialog-close" onclick={() => (worktreeDetailsOpen = false)}>
          Close
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .agent-canvas {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px 18px 18px;
    min-height: 0;
    background:
      radial-gradient(circle at top left, color-mix(in srgb, var(--accent) 10%, transparent), transparent 28%),
      linear-gradient(180deg, color-mix(in srgb, var(--bg-secondary) 88%, transparent), var(--bg-primary));
    overflow: hidden;
  }

  .canvas-toolbar,
  .toolbar-group,
  .zoom-controls,
  .card-header,
  .card-heading,
  .card-footer,
  .detail-header,
  .dialog-header {
    display: flex;
    align-items: center;
  }

  .canvas-toolbar {
    justify-content: space-between;
    gap: 16px;
  }

  .canvas-toolbar h2 {
    margin: 0;
    font-size: 1rem;
  }

  .canvas-toolbar p {
    margin: 4px 0 0;
    color: var(--text-muted);
  }

  .toolbar-group {
    gap: 10px;
  }

  .toolbar-chip {
    border: 1px solid var(--border-color);
    border-radius: 999px;
    padding: 6px 10px;
    color: var(--text-secondary);
    background: color-mix(in srgb, var(--bg-secondary) 75%, transparent);
  }

  .zoom-controls {
    gap: 6px;
  }

  .zoom-btn {
    min-width: 36px;
    height: 32px;
    border-radius: 999px;
    border: 1px solid var(--border-color);
    background: color-mix(in srgb, var(--bg-secondary) 82%, transparent);
    color: var(--text-primary);
    cursor: pointer;
  }

  .zoom-btn.ghost {
    min-width: 74px;
  }

  .canvas-shell {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1.1fr) minmax(340px, 0.9fr);
    gap: 18px;
  }

  .canvas-board-panel,
  .canvas-detail {
    min-height: 0;
    border: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
    border-radius: 18px;
    overflow: hidden;
    background: color-mix(in srgb, var(--bg-secondary) 78%, var(--bg-primary));
    box-shadow: 0 14px 28px rgba(0, 0, 0, 0.16);
  }

  .canvas-board {
    position: relative;
    height: 100%;
    overflow: auto;
    cursor: grab;
    background:
      linear-gradient(90deg, color-mix(in srgb, var(--border-color) 18%, transparent) 1px, transparent 1px),
      linear-gradient(color-mix(in srgb, var(--border-color) 18%, transparent) 1px, transparent 1px),
      linear-gradient(180deg, color-mix(in srgb, var(--bg-primary) 94%, transparent), var(--bg-primary));
    background-size: 40px 40px, 40px 40px, auto;
  }

  .canvas-board:active {
    cursor: grabbing;
  }

  .canvas-surface {
    position: relative;
    transform-origin: 0 0;
  }

  .canvas-edges {
    position: absolute;
    inset: 0;
    pointer-events: none;
    overflow: visible;
  }

  .edge-line {
    stroke: color-mix(in srgb, var(--accent) 58%, var(--border-color));
    stroke-width: 3;
    stroke-linecap: round;
  }

  .canvas-card {
    position: absolute;
    display: flex;
    flex-direction: column;
    border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
    background: color-mix(in srgb, var(--bg-secondary) 84%, var(--bg-primary));
    border-radius: 18px;
    box-shadow: 0 14px 28px rgba(0, 0, 0, 0.18);
    overflow: hidden;
    cursor: pointer;
    outline: none;
  }

  .canvas-card.selected {
    border-color: color-mix(in srgb, var(--accent) 58%, var(--border-color));
    box-shadow:
      0 14px 28px rgba(0, 0, 0, 0.18),
      0 0 0 1px color-mix(in srgb, var(--accent) 38%, transparent);
  }

  .card-header,
  .dialog-header {
    justify-content: space-between;
    gap: 12px;
    padding: 12px 14px;
    border-bottom: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
    background: color-mix(in srgb, var(--bg-primary) 68%, transparent);
  }

  .card-heading {
    gap: 10px;
    min-width: 0;
    flex: 1;
  }

  .card-kind {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 76px;
    padding: 4px 8px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--text-secondary);
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .card-title {
    font-weight: 600;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .card-drag-handle {
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: grab;
    font-family: monospace;
    letter-spacing: 0.08em;
  }

  .card-drag-handle:active {
    cursor: grabbing;
  }

  .card-body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    min-height: 0;
    flex: 1;
  }

  .card-subtitle,
  .card-copy {
    margin: 0;
  }

  .card-subtitle {
    color: var(--text-secondary);
    font-size: 0.9rem;
  }

  .card-copy {
    color: var(--text-muted);
    overflow-wrap: anywhere;
    line-height: 1.45;
  }

  .card-footer {
    margin-top: auto;
    justify-content: space-between;
    gap: 10px;
    color: var(--text-secondary);
    font-size: 0.8rem;
  }

  .card-status {
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .canvas-detail {
    display: flex;
    min-height: 0;
  }

  .detail-placeholder,
  .worktree-detail-card {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .detail-placeholder {
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
  }

  .worktree-detail-card {
    flex-direction: column;
    gap: 18px;
    padding: 20px;
  }

  .detail-header {
    justify-content: space-between;
  }

  .detail-kind {
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-size: 0.8rem;
  }

  .detail-header h3 {
    margin: 0;
    font-size: 1rem;
  }

  .detail-grid,
  .dialog-body {
    display: grid;
    gap: 12px;
  }

  .detail-row {
    display: grid;
    gap: 4px;
  }

  .detail-label {
    color: var(--text-muted);
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .detail-value {
    overflow-wrap: anywhere;
  }

  .detail-action,
  .dialog-close {
    width: fit-content;
    border: 1px solid var(--border-color);
    border-radius: 999px;
    padding: 8px 14px;
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
  }

  .worktree-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.46);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 20;
  }

  .worktree-dialog {
    width: min(560px, calc(100vw - 32px));
    border-radius: 18px;
    overflow: hidden;
    border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
    background: color-mix(in srgb, var(--bg-secondary) 85%, var(--bg-primary));
    box-shadow: 0 22px 44px rgba(0, 0, 0, 0.24);
  }

  .dialog-close {
    margin: 0 16px 16px;
  }

  @media (max-width: 1200px) {
    .canvas-shell {
      grid-template-columns: minmax(0, 1fr);
    }

    .canvas-detail {
      min-height: 320px;
    }
  }
</style>
