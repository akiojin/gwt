<script lang="ts">
  import {
    type AgentCanvasTileLayout,
    buildAgentCanvasGraph,
    createDefaultAgentCanvasViewport,
    type AgentCanvasViewport,
  } from "../agentCanvas";
  import {
    buildBoardViewportRuntime,
    buildDefaultLayoutsRuntime,
    buildEdgeLinesRuntime,
    buildLiveSessionTileIdsRuntime,
    buildSurfaceBoundsRuntime,
    isLayoutVisibleRuntime,
    preferredSelectedTileIdRuntime,
  } from "../agentCanvasRuntime";
  import type { Tab, WorktreeInfo } from "../types";
  import AssistantPanel from "./AssistantPanel.svelte";
  import TerminalView from "../terminal/TerminalView.svelte";

  type CanvasTileKind = "assistant" | "worktree" | "agent" | "terminal";

  type RenderableCanvasTile = {
    id: string;
    kind: CanvasTileKind;
    title: string;
    subtitle: string;
    detail: string;
    worktree?: WorktreeInfo;
    tab?: Tab;
    worktreeTileId?: string | null;
  };

  type PointerDragState = {
    tileId: string;
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

  const TILE_WIDTH = 280;
  const TILE_HEIGHT = 164;
  const BOARD_PADDING = 40;
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
    persistedTileLayouts = undefined,
    persistedSelectedTileId = null,
    onViewportChange = () => {},
    onTileLayoutsChange = () => {},
    onSelectedTileChange = () => {},
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
    persistedTileLayouts?: Record<string, AgentCanvasTileLayout> | undefined;
    persistedSelectedTileId?: string | null;
    onViewportChange?: (viewport: AgentCanvasViewport) => void;
    onTileLayoutsChange?: (layouts: Record<string, AgentCanvasTileLayout>) => void;
    onSelectedTileChange?: (tileId: string | null) => void;
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
  let detailOverlayTileId = $state<string | null>(null);
  let viewport = $state<AgentCanvasViewport>(createDefaultAgentCanvasViewport());
  let tileLayouts = $state<Record<string, AgentCanvasTileLayout>>({});
  let selectedTileId = $state<string>("assistant");
  let dragState = $state<PointerDragState | null>(null);
  let panState = $state<PointerPanState | null>(null);
  let boardEl = $state<HTMLDivElement | null>(null);
  let boardViewport = $state({ left: 0, top: 0, right: 0, bottom: 0 });
  let lastSeedKey = $state("");
  let lastViewportEmitKey = $state("");
  let lastLayoutsEmitKey = $state("");
  let lastSelectedEmitKey = $state("");

  function worktreeTileTestId(worktree: WorktreeInfo): string {
    const raw = worktree.branch ?? "project-root";
    return `agent-canvas-worktree-tile-${raw.replace(/[^a-zA-Z0-9_-]+/g, "-")}`;
  }

  function edgeTestId(tileId: string): string {
    return `agent-canvas-edge-${tileId.replace(/[^a-zA-Z0-9_-]+/g, "-")}`;
  }

  let sessionsByWorktreeId = $derived.by(() => {
    const mapped = new Map<string, (typeof graph.sessionTiles)>();
    for (const tile of graph.sessionTiles) {
      if (!tile.worktreeTileId) continue;
      const existing = mapped.get(tile.worktreeTileId) ?? [];
      mapped.set(tile.worktreeTileId, [...existing, tile]);
    }
    return mapped;
  });

  let renderableTiles = $derived.by<RenderableCanvasTile[]>(() => {
    const tiles: RenderableCanvasTile[] = [
      {
        id: "assistant",
        kind: "assistant",
        title: "Assistant",
        subtitle: "Workspace assistant",
        detail: "Click tiles to open focused detail without splitting the workspace.",
      },
    ];

    for (const worktreeTile of graph.worktreeTiles) {
      const sessionCount = sessionsByWorktreeId.get(worktreeTile.id)?.length ?? 0;
      tiles.push({
        id: worktreeTile.id,
        kind: "worktree",
        title: worktreeTile.worktree.branch || "Project Root",
        subtitle: sessionCount === 1 ? "1 linked session" : `${sessionCount} linked sessions`,
        detail: worktreeTile.worktree.path,
        worktree: worktreeTile.worktree,
      });
    }

    for (const sessionTile of graph.sessionTiles) {
      tiles.push({
        id: sessionTile.id,
        kind: sessionTile.type,
        title: sessionTile.tab.label,
        subtitle: sessionTile.type === "agent" ? "Agent session" : "Terminal session",
        detail:
          sessionTile.tab.branchName ||
          sessionTile.tab.worktreePath ||
          sessionTile.tab.cwd ||
          "No branch context",
        tab: sessionTile.tab,
        worktreeTileId: sessionTile.worktreeTileId,
      });
    }

    return tiles;
  });

  function clampZoom(value: number): number {
    return Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, value));
  }

  function preferredSelectedTileId(): string {
    return preferredSelectedTileIdRuntime({
      selectedSessionTabId,
      selectedWorktreeBranch,
      worktreeTiles: graph.worktreeTiles,
    });
  }

  function buildDefaultLayouts(tiles: RenderableCanvasTile[]): Record<string, AgentCanvasTileLayout> {
    return buildDefaultLayoutsRuntime(tiles);
  }

  function updateBoardViewport() {
    const el = boardEl;
    if (!el) return;
    boardViewport = buildBoardViewportRuntime({
      scrollLeft: el.scrollLeft,
      scrollTop: el.scrollTop,
      clientWidth: el.clientWidth,
      clientHeight: el.clientHeight,
      viewport,
    });
  }

  function isLayoutVisible(layout: AgentCanvasTileLayout | undefined): boolean {
    return isLayoutVisibleRuntime(layout, boardViewport);
  }

  $effect(() => {
    const seedKey = JSON.stringify([
      projectPath,
      persistedViewport ?? null,
      persistedTileLayouts ?? null,
      persistedSelectedTileId ?? null,
    ]);
    if (seedKey !== lastSeedKey) {
      lastSeedKey = seedKey;
      viewport = persistedViewport ?? createDefaultAgentCanvasViewport();
      tileLayouts = persistedTileLayouts ? { ...persistedTileLayouts } : {};
      if (persistedSelectedTileId) {
        selectedTileId = persistedSelectedTileId;
      }
    }

    const defaults = buildDefaultLayouts(renderableTiles);
    const nextLayouts: Record<string, AgentCanvasTileLayout> = {};
    let needsLayoutSync = Object.keys(tileLayouts).length !== renderableTiles.length;
    for (const tile of renderableTiles) {
      const existing = tileLayouts[tile.id];
      if (!existing) needsLayoutSync = true;
      nextLayouts[tile.id] = existing ?? defaults[tile.id];
    }
    if (needsLayoutSync) {
      tileLayouts = nextLayouts;
    }

    const preferred = preferredSelectedTileId();
    const fallbackSelected =
      preferred in nextLayouts ? preferred : renderableTiles[0]?.id ?? "assistant";
    if (!(selectedTileId in nextLayouts)) {
      if (selectedTileId !== fallbackSelected) {
        selectedTileId = fallbackSelected;
      }
    } else if (
      selectedSessionTabId &&
      preferred in nextLayouts &&
      selectedTileId !== preferred
    ) {
      selectedTileId = preferred;
    }
  });

  $effect(() => {
    const key = JSON.stringify(viewport);
    if (key === lastViewportEmitKey) return;
    lastViewportEmitKey = key;
    onViewportChange(viewport);
    updateBoardViewport();
  });

  $effect(() => {
    const key = JSON.stringify(tileLayouts);
    if (key === lastLayoutsEmitKey) return;
    lastLayoutsEmitKey = key;
    onTileLayoutsChange(tileLayouts);
  });

  $effect(() => {
    const key = selectedTileId || "";
    if (key === lastSelectedEmitKey) return;
    lastSelectedEmitKey = key;
    onSelectedTileChange(selectedTileId || null);
  });

  let popupWorktree = $derived(
    popupWorktreeBranch
      ? canvasWorktrees.find((worktree) => worktree.branch === popupWorktreeBranch) ?? null
      : null,
  );

  let overlayTile = $derived.by(
    () =>
      detailOverlayTileId
        ? renderableTiles.find((tile) => tile.id === detailOverlayTileId) ?? null
        : null,
  );

  let edgeLines = $derived.by(() =>
    buildEdgeLinesRuntime({
      edges: graph.edges,
      tileLayouts,
    }),
  );

  let surfaceBounds = $derived.by(() => buildSurfaceBoundsRuntime(tileLayouts));

  let liveSessionTileIds = $derived.by(() =>
    buildLiveSessionTileIdsRuntime({
      renderableTiles,
      tileLayouts,
      boardViewport,
    }),
  );

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

  function selectTile(tile: RenderableCanvasTile) {
    selectedTileId = tile.id;
    if (tile.kind === "worktree" && tile.worktree?.branch) {
      onWorktreeSelect(tile.worktree.branch);
      detailOverlayTileId = null;
    } else if ((tile.kind === "agent" || tile.kind === "terminal") && tile.tab) {
      onSessionSelect(tile.tab.id);
      detailOverlayTileId = null;
    } else if (tile.kind === "assistant") {
      detailOverlayTileId = tile.id;
    }
  }

  function activateWorktreeTile(worktree: WorktreeInfo) {
    if (worktree.branch) {
      onWorktreeSelect(worktree.branch);
      popupWorktreeBranch = worktree.branch;
    }
    const tileId = `worktree:${worktree.path}`;
    selectedTileId = tileId;
    detailOverlayTileId = null;
    worktreeDetailsOpen = true;
  }

  function handleTileKeydown(tile: RenderableCanvasTile, event: KeyboardEvent) {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    if (tile.kind === "worktree" && tile.worktree) {
      activateWorktreeTile(tile.worktree);
      return;
    }
    selectTile(tile);
  }

  function beginTileDrag(tileId: string, event: PointerEvent) {
    if (event.button !== 0) return;
    const layout = tileLayouts[tileId];
    if (!layout) return;
    event.preventDefault();
    event.stopPropagation();
    selectedTileId = tileId;
    dragState = {
      tileId,
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
    const target = event.target as HTMLElement;
    if (target.closest('.canvas-tile')) return;
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
      const current = tileLayouts[dragState.tileId];
      if (!current) return;
      tileLayouts = {
        ...tileLayouts,
        [dragState.tileId]: {
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

  $effect(() => {
    const el = boardEl;
    if (!el) return;
    updateBoardViewport();

    const handleScroll = () => updateBoardViewport();
    const observer = new ResizeObserver(() => updateBoardViewport());
    observer.observe(el);
    el.addEventListener("scroll", handleScroll, { passive: true });

    return () => {
      observer.disconnect();
      el.removeEventListener("scroll", handleScroll);
    };
  });
</script>

<div class="agent-canvas">
  <div class="canvas-toolbar">
    <div>
      <h2>Agent Canvas</h2>
      <p>Drag tiles on the board and open focused detail in a popup without splitting the workspace.</p>
    </div>
    <div class="toolbar-group">
      <div class="toolbar-chip">Tiles: {renderableTiles.length}</div>
      <div class="zoom-controls" data-testid="agent-canvas-zoom-controls">
        <button type="button" class="zoom-btn" onclick={zoomOut} aria-label="Zoom out">-</button>
        <button type="button" class="zoom-btn ghost" onclick={resetViewport}>
          <span data-testid="agent-canvas-zoom-label">{zoomPercent}%</span>
        </button>
        <button type="button" class="zoom-btn" onclick={zoomIn} aria-label="Zoom in">+</button>
      </div>
    </div>
  </div>

  <section class="canvas-board-panel" data-testid="agent-canvas-surface">
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
              data-testid={edgeTestId(edge.targetTileId)}
            ></line>
          {/each}
        </svg>

        {#each renderableTiles as tile (tile.id)}
          <div
            class="canvas-tile"
            class:selected={selectedTileId === tile.id}
            class:assistant-tile={tile.kind === "assistant"}
            class:worktree-tile={tile.kind === "worktree"}
            class:session-tile={tile.kind === "agent" || tile.kind === "terminal"}
            role="button"
            tabindex="0"
            data-testid={
              tile.kind === "assistant"
                ? "agent-canvas-assistant-tile"
                : tile.kind === "worktree" && tile.worktree
                  ? worktreeTileTestId(tile.worktree)
                  : tile.tab
                    ? `agent-canvas-session-${tile.tab.id}`
                    : `agent-canvas-tile-${tile.id}`
            }
            style={`transform: translate(${tileLayouts[tile.id]?.x ?? 0}px, ${tileLayouts[tile.id]?.y ?? 0}px); width:${tileLayouts[tile.id]?.width ?? TILE_WIDTH}px; height:${tileLayouts[tile.id]?.height ?? TILE_HEIGHT}px;`}
            onclick={() => {
              if (tile.kind === "worktree" && tile.worktree) {
                activateWorktreeTile(tile.worktree);
              } else {
                selectTile(tile);
              }
            }}
            onkeydown={(event) => handleTileKeydown(tile, event)}
          >
            <div class="tile-header">
              <div class="tile-heading">
                <span class="tile-kind">{tile.kind === "agent" ? "Agent" : tile.kind === "terminal" ? "Terminal" : tile.kind === "worktree" ? "Worktree" : "Assistant"}</span>
                <span class="tile-title">{tile.title}</span>
              </div>
              <button
                type="button"
                class="tile-drag-handle"
                aria-label="Drag tile"
                onpointerdown={(event) => beginTileDrag(tile.id, event)}
                onclick={(event) => event.stopPropagation()}
              >
                ::
              </button>
            </div>

            <div class="tile-body">
              {#if tile.kind === "worktree" && tile.worktree}
                <p class="tile-subtitle">{tile.subtitle}</p>
                <p class="tile-copy">{tile.detail}</p>
                <div class="tile-footer">
                  <span class="tile-status">{tile.worktree.safety_level}</span>
                  <span>{sessionsByWorktreeId.get(tile.id)?.length ?? 0} sessions</span>
                </div>
              {:else if tile.kind === "agent" || tile.kind === "terminal"}
                <div class="session-surface" data-testid={`agent-canvas-session-surface-${tile.tab?.id ?? tile.id}`}>
                  {#if tile.tab?.paneId && liveSessionTileIds.has(tile.id)}
                    <TerminalView
                      paneId={tile.tab.paneId}
                      active={true}
                      focusOnActivate={selectedTileId === tile.id}
                      showControls={selectedTileId === tile.id}
                      agentId={tile.kind === "agent" ? tile.tab.agentId ?? null : null}
                      {voiceInputEnabled}
                      {voiceInputListening}
                      {voiceInputPreparing}
                      {voiceInputSupported}
                      {voiceInputAvailable}
                      {voiceInputAvailabilityReason}
                      {voiceInputError}
                    />
                  {:else if tile.tab?.paneId}
                    <div class="session-placeholder">
                      Preview paused while this tile is outside the active viewport.
                    </div>
                  {:else}
                    <div class="session-placeholder">
                      {tile.kind === "agent" ? "Agent starting..." : "Terminal starting..."}
                    </div>
                  {/if}
                </div>
                <div class="tile-footer session-footer">
                  <span class="tile-status">{tile.kind === "agent" ? "Agent" : "Shell"}</span>
                  <span>{tile.subtitle}</span>
                </div>
              {:else}
                <p class="tile-subtitle">{tile.subtitle}</p>
                <p class="tile-copy">{tile.detail}</p>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    </div>
  </section>

  {#if overlayTile}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="detail-overlay"
      data-testid="agent-canvas-detail-overlay"
      onclick={() => (detailOverlayTileId = null)}
    >
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="detail-dialog"
        data-testid="agent-canvas-detail-dialog"
        role="dialog"
        aria-modal="true"
        tabindex="0"
        onclick={(event) => event.stopPropagation()}
      >
        <div class="dialog-header">
          <span class="tile-kind">
            {overlayTile.kind === "assistant"
              ? "Assistant"
              : overlayTile.kind === "worktree"
                ? "Worktree"
                : overlayTile.kind === "agent"
                  ? "Agent"
                  : "Terminal"}
          </span>
          <span class="tile-title">{overlayTile.title}</span>
          <button
            type="button"
            class="dialog-close-inline"
            onclick={() => (detailOverlayTileId = null)}
          >
            Close
          </button>
        </div>
        <div class="detail-dialog-body">
          {#if overlayTile.kind === "assistant"}
            <AssistantPanel
              isActive={true}
              {projectPath}
              onOpenSettings={onOpenSettings ?? (() => {})}
            />
          {/if}
        </div>
      </div>
    </div>
  {/if}

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
          <span class="tile-kind">Worktree</span>
          <span class="tile-title">{popupWorktree?.branch || currentBranch || "Project Root"}</span>
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
            <span class="detail-value">{graph.sessionTiles.filter((tile) => popupWorktree ? tile.worktreeTileId === `worktree:${popupWorktree.path}` : true).length}</span>
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
  .tile-header,
  .tile-heading,
  .tile-footer,
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

  .canvas-board-panel {
    flex: 1;
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

  .canvas-tile {
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

  .canvas-tile.selected {
    border-color: color-mix(in srgb, var(--accent) 58%, var(--border-color));
    box-shadow:
      0 14px 28px rgba(0, 0, 0, 0.18),
      0 0 0 1px color-mix(in srgb, var(--accent) 38%, transparent);
  }

  .tile-header,
  .dialog-header {
    justify-content: space-between;
    gap: 12px;
    padding: 12px 14px;
    border-bottom: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
    background: color-mix(in srgb, var(--bg-primary) 68%, transparent);
  }

  .tile-heading {
    gap: 10px;
    min-width: 0;
    flex: 1;
  }

  .tile-kind {
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

  .tile-title {
    font-weight: 600;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tile-drag-handle {
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: grab;
    font-family: monospace;
    letter-spacing: 0.08em;
  }

  .tile-drag-handle:active {
    cursor: grabbing;
  }

  .tile-body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    min-height: 0;
    flex: 1;
  }

  .tile-subtitle,
  .tile-copy {
    margin: 0;
  }

  .tile-subtitle {
    color: var(--text-secondary);
    font-size: 0.9rem;
  }

  .tile-copy {
    color: var(--text-muted);
    overflow-wrap: anywhere;
    line-height: 1.45;
  }

  .tile-footer {
    margin-top: auto;
    justify-content: space-between;
    gap: 10px;
    color: var(--text-secondary);
    font-size: 0.8rem;
  }

  .canvas-tile.session-tile .tile-body {
    padding: 0;
    gap: 0;
  }

  .session-surface {
    flex: 1;
    min-height: 0;
    border-bottom: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
    background: color-mix(in srgb, var(--bg-primary) 92%, transparent);
    min-width: 0;
  }

  .session-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    padding: 18px;
    color: var(--text-muted);
    text-align: center;
    line-height: 1.45;
  }

  .session-footer {
    margin-top: 0;
    padding: 10px 14px 12px;
  }

  .tile-status {
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .detail-dialog-body,
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

  .detail-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.46);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 20;
  }

  .detail-dialog {
    width: min(1100px, calc(100vw - 48px));
    height: min(760px, calc(100vh - 48px));
    border-radius: 18px;
    overflow: hidden;
    border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
    background: color-mix(in srgb, var(--bg-secondary) 85%, var(--bg-primary));
    box-shadow: 0 22px 44px rgba(0, 0, 0, 0.24);
    display: flex;
    flex-direction: column;
  }

  .detail-dialog-body {
    min-height: 0;
    flex: 1;
  }

  .dialog-close-inline {
    border: 1px solid var(--border-color);
    border-radius: 999px;
    padding: 6px 12px;
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
  }

  @media (max-width: 1200px) {
    .detail-dialog {
      width: calc(100vw - 24px);
      height: calc(100vh - 24px);
    }
  }
</style>
