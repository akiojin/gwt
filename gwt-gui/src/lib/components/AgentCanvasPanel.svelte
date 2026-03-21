<script lang="ts">
  import type { Tab } from "../types";
  import AssistantPanel from "./AssistantPanel.svelte";
  import TerminalView from "../terminal/TerminalView.svelte";

  let {
    projectPath,
    currentBranch = "",
    tabs,
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

  let sessionCards = $derived(
    tabs.filter((tab) => tab.type === "agent" || tab.type === "terminal"),
  );
</script>

<div class="agent-canvas">
  <div class="canvas-toolbar">
    <div>
      <h2>Agent Canvas</h2>
      <p>Canvas cards replace the old assistant and session tabs.</p>
    </div>
    <div class="toolbar-chip">Cards: {sessionCards.length + 2}</div>
  </div>

  <div class="canvas-grid">
    <section class="canvas-card worktree-card" data-testid="agent-canvas-worktree-card">
      <div class="card-header">
        <span class="card-kind">Worktree</span>
        <span class="card-title">{currentBranch || "Project Root"}</span>
      </div>
      <p class="card-copy">
        Worktree cards will become the parent nodes for agent and terminal sessions.
      </p>
    </section>

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

    {#each sessionCards as tab (tab.id)}
      <button
        class="canvas-card session-card"
        class:agent-session={tab.type === "agent"}
        class:terminal-session={tab.type === "terminal"}
        class:selected={selectedSessionTabId === tab.id}
        data-testid={`agent-canvas-session-${tab.id}`}
        type="button"
        onclick={() => onSessionSelect(tab.id)}
      >
        <span class="session-edge" aria-hidden="true"></span>
        <div class="card-header">
          <span class="card-kind">{tab.type === "agent" ? "Agent" : "Terminal"}</span>
          <span class="card-title">{tab.label}</span>
        </div>
        <div class="card-body">
          {#if tab.paneId}
            <TerminalView
              paneId={tab.paneId}
              active={true}
              agentId={tab.type === "agent" ? tab.agentId ?? null : null}
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
              {tab.type === "agent" ? "Agent starting..." : "Terminal starting..."}
            </div>
          {/if}
        </div>
      </button>
    {/each}
  </div>
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
