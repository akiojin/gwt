<script lang="ts">
  import type { Tab, WorktreeInfo } from "../types";
  import { buildAgentCanvasGraph } from "../agentCanvas";
  import AssistantPanel from "./AssistantPanel.svelte";
  import TerminalView from "../terminal/TerminalView.svelte";

  let {
    projectPath,
    currentBranch = "",
    tabs,
    worktrees = [],
    selectedWorktreeBranch = null,
    onWorktreeSelect = () => {},
    selectedSessionTabId = null,
    onSessionSelect = () => {},
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
  let sessionCards = $derived(graph.sessionCards.map((card) => card.tab));
  let worktreeDetailsOpen = $state(false);
  let popupWorktreeBranch = $state<string | null>(null);
  let canvasWorktrees = $derived(graph.worktrees);
  let popupWorktree = $derived(
    popupWorktreeBranch
      ? canvasWorktrees.find((worktree) => worktree.branch === popupWorktreeBranch) ?? null
      : null,
  );

  function worktreeCardTestId(worktree: WorktreeInfo): string {
    const raw = worktree.branch ?? "project-root";
    return `agent-canvas-worktree-card-${raw.replace(/[^a-zA-Z0-9_-]+/g, "-")}`;
  }
</script>

<div class="agent-canvas">
  <div class="canvas-toolbar">
    <div>
      <h2>Agent Canvas</h2>
      <p>Canvas cards replace the old assistant and session tabs.</p>
    </div>
    <div class="toolbar-chip">Cards: {sessionCards.length + canvasWorktrees.length + 1}</div>
  </div>

  <div class="canvas-grid">
    <section class="canvas-card assistant-card" data-testid="agent-canvas-assistant-card">
      <div class="card-header">
        <span class="card-kind">Assistant</span>
        <span class="card-title">Assistant</span>
      </div>
      <div class="card-body assistant-body">
        <AssistantPanel
          isActive={true}
          {projectPath}
          onOpenSettings={onOpenSettings ?? (() => {})}
        />
      </div>
    </section>

    {#each canvasWorktrees as worktree ((worktree.branch ?? worktree.path))}
      <button
        type="button"
        class="canvas-card worktree-card"
        class:selected={selectedWorktreeBranch === worktree.branch}
        data-testid={worktreeCardTestId(worktree)}
        onclick={() => {
          if (worktree.branch) {
            onWorktreeSelect(worktree.branch);
            popupWorktreeBranch = worktree.branch;
          }
          worktreeDetailsOpen = true;
        }}
      >
        <div class="card-header">
          <span class="card-kind">Worktree</span>
          <span class="card-title">{worktree.branch || "Project Root"}</span>
        </div>
        <p class="card-copy">
          {worktree.path}
        </p>
      </button>

      {#each graph.sessionCards.filter((card) => card.worktreeCardId === `worktree:${worktree.path}`) as card (card.id)}
        <button
          class="canvas-card session-card"
          class:agent-session={card.tab.type === "agent"}
          class:terminal-session={card.tab.type === "terminal"}
          class:selected={selectedSessionTabId === card.tab.id}
          data-testid={`agent-canvas-session-${card.tab.id}`}
          type="button"
          onclick={() => onSessionSelect(card.tab.id)}
        >
          <span
            class="session-edge"
            aria-hidden="true"
            data-testid={`agent-canvas-edge-${card.id.replace(/[^a-zA-Z0-9_-]+/g, "-")}`}
          ></span>
          <div class="card-header">
            <span class="card-kind">{card.tab.type === "agent" ? "Agent" : "Terminal"}</span>
            <span class="card-title">{card.tab.label}</span>
          </div>
          <div class="card-body">
            {#if card.tab.paneId}
              <TerminalView
                paneId={card.tab.paneId}
                active={true}
                agentId={card.tab.type === "agent" ? card.tab.agentId ?? null : null}
                {voiceInputEnabled}
                {voiceInputListening}
                {voiceInputPreparing}
                {voiceInputSupported}
                {voiceInputAvailable}
                {voiceInputAvailabilityReason}
                {voiceInputError}
              />
            {:else}
              <div class="session-placeholder">
                {card.tab.type === "agent" ? "Agent starting..." : "Terminal starting..."}
              </div>
            {/if}
          </div>
        </button>
      {/each}
    {/each}
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
        <div class="card-header">
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
        <button
          type="button"
          class="dialog-close"
          onclick={() => (worktreeDetailsOpen = false)}
        >
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
    background:
      radial-gradient(circle at top left, color-mix(in srgb, var(--accent) 10%, transparent), transparent 28%),
      linear-gradient(180deg, color-mix(in srgb, var(--bg-secondary) 88%, transparent), var(--bg-primary));
    overflow: auto;
  }

  .canvas-toolbar {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
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

  .toolbar-chip {
    border: 1px solid var(--border-color);
    border-radius: 999px;
    padding: 6px 10px;
    color: var(--text-secondary);
    background: color-mix(in srgb, var(--bg-secondary) 75%, transparent);
  }

  .canvas-grid {
    display: grid;
    grid-template-columns: minmax(240px, 300px) minmax(340px, 1fr);
    gap: 18px 22px;
    align-items: start;
  }

  .canvas-card {
    position: relative;
    min-width: 0;
    min-height: 180px;
    border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
    background: color-mix(in srgb, var(--bg-secondary) 80%, var(--bg-primary));
    box-shadow: 0 14px 28px rgba(0, 0, 0, 0.16);
    border-radius: 16px;
    overflow: hidden;
  }

  .worktree-card {
    grid-column: 1;
    cursor: pointer;
    padding: 0;
    text-align: left;
    font: inherit;
  }

  .assistant-card {
    grid-column: 2;
    min-height: 420px;
  }

  .session-card {
    grid-column: 2;
    min-height: 280px;
    cursor: pointer;
    padding: 0;
    text-align: left;
    font: inherit;
  }

  .session-card.selected {
    border-color: color-mix(in srgb, var(--accent) 58%, var(--border-color));
    box-shadow:
      0 14px 28px rgba(0, 0, 0, 0.16),
      0 0 0 1px color-mix(in srgb, var(--accent) 36%, transparent);
  }

  .session-edge {
    position: absolute;
    left: -22px;
    top: 50%;
    width: 22px;
    height: 2px;
    background: color-mix(in srgb, var(--accent) 58%, var(--border-color));
  }

  .session-edge::before {
    content: "";
    position: absolute;
    left: -10px;
    top: -9px;
    width: 10px;
    height: 20px;
    border-left: 2px solid color-mix(in srgb, var(--accent) 58%, var(--border-color));
    border-top: 2px solid color-mix(in srgb, var(--accent) 58%, var(--border-color));
    border-bottom: 2px solid color-mix(in srgb, var(--accent) 58%, var(--border-color));
    border-radius: 10px 0 0 10px;
  }

  .card-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
    background: color-mix(in srgb, var(--bg-primary) 65%, transparent);
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

  .card-body {
    min-height: 0;
    height: calc(100% - 49px);
  }

  .assistant-body {
    height: calc(100% - 49px);
  }

  .card-copy {
    margin: 0;
    padding: 14px;
    color: var(--text-secondary);
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

  .dialog-body {
    display: grid;
    gap: 12px;
    padding: 16px;
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
    word-break: break-all;
  }

  .dialog-close {
    margin: 0 16px 16px;
    border: 1px solid var(--border-color);
    border-radius: 999px;
    padding: 8px 14px;
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
  }

  .session-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-muted);
  }

  @media (max-width: 1100px) {
    .canvas-grid {
      grid-template-columns: minmax(0, 1fr);
    }

    .worktree-card,
    .assistant-card,
    .session-card {
      grid-column: 1;
    }

    .session-edge {
      display: none;
    }
  }
</style>
