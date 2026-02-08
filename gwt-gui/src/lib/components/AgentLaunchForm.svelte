<script lang="ts">
  import type { AgentInfo } from "../types";

  let {
    selectedBranch = "",
    onLaunch,
    onClose,
  }: {
    selectedBranch?: string;
    onLaunch: (agentName: string, branch: string) => void;
    onClose: () => void;
  } = $props();

  let agents: AgentInfo[] = $state([]);
  let selectedAgent: string = $state("");
  // Intentionally capture the initial value - the branch field is editable by the user
  let branch: string = $state((() => selectedBranch)());
  let loading: boolean = $state(true);
  let launching: boolean = $state(false);

  $effect(() => {
    detectAgents();
  });

  async function detectAgents() {
    loading = true;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      agents = await invoke<AgentInfo[]>("detect_agents");
      if (agents.length > 0) {
        const available = agents.find((a) => a.available);
        if (available) {
          selectedAgent = available.name;
        }
      }
    } catch (err) {
      console.error("Failed to detect agents:", err);
      agents = [];
    }
    loading = false;
  }

  async function handleLaunch() {
    if (!selectedAgent || !branch) return;
    launching = true;
    onLaunch(selectedAgent, branch);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      onClose();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_interactive_supports_focus -->
<div class="overlay" onclick={onClose} onkeydown={handleKeydown} role="dialog" aria-modal="true" aria-label="Launch Agent">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <div class="dialog-header">
      <h2>Launch Agent</h2>
      <button class="close-btn" onclick={onClose}>[x]</button>
    </div>

    {#if loading}
      <div class="loading">Detecting agents...</div>
    {:else}
      <div class="dialog-body">
        <div class="field">
          <label for="agent-select">Agent</label>
          <div class="agent-cards">
            {#each agents as agent}
              <button
                class="agent-card"
                class:selected={selectedAgent === agent.name}
                class:unavailable={!agent.available}
                disabled={!agent.available}
                onclick={() => (selectedAgent = agent.name)}
              >
                <span class="agent-name">{agent.name}</span>
                <span class="agent-type">{agent.agent_type}</span>
                {#if !agent.available}
                  <span class="agent-status">Not available</span>
                {/if}
              </button>
            {/each}
          </div>
        </div>

        <div class="field">
          <label for="branch-input">Branch</label>
          <input
            id="branch-input"
            type="text"
            bind:value={branch}
            placeholder="Enter branch name..."
          />
        </div>
      </div>

      <div class="dialog-footer">
        <button class="btn btn-cancel" onclick={onClose}>Cancel</button>
        <button
          class="btn btn-launch"
          disabled={!selectedAgent || !branch || launching}
          onclick={handleLaunch}
        >
          {launching ? "Launching..." : "Launch"}
        </button>
      </div>
    {/if}
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .dialog {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 12px;
    width: 460px;
    max-width: 90vw;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4);
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: 1px solid var(--border-color);
  }

  .dialog-header h2 {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 14px;
    font-family: monospace;
    padding: 2px 4px;
  }

  .close-btn:hover {
    color: var(--text-primary);
  }

  .loading {
    padding: 40px;
    text-align: center;
    color: var(--text-muted);
  }

  .dialog-body {
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field label {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .agent-cards {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .agent-card {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    cursor: pointer;
    font-family: inherit;
    color: var(--text-primary);
    text-align: left;
    transition: border-color 0.15s;
  }

  .agent-card:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .agent-card.selected {
    border-color: var(--accent);
    background: var(--bg-surface);
  }

  .agent-card.unavailable {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .agent-name {
    font-size: 13px;
    font-weight: 500;
  }

  .agent-type {
    font-size: 11px;
    color: var(--text-muted);
    margin-left: auto;
  }

  .agent-status {
    font-size: 10px;
    color: var(--red);
  }

  .field input {
    padding: 8px 12px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: 13px;
    font-family: monospace;
    outline: none;
  }

  .field input:focus {
    border-color: var(--accent);
  }

  .dialog-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 16px 20px;
    border-top: 1px solid var(--border-color);
  }

  .btn {
    padding: 8px 16px;
    border: none;
    border-radius: 6px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    font-family: inherit;
    transition: background-color 0.15s;
  }

  .btn-cancel {
    background: var(--bg-surface);
    color: var(--text-secondary);
  }

  .btn-cancel:hover {
    background: var(--bg-hover);
  }

  .btn-launch {
    background: var(--accent);
    color: var(--bg-primary);
  }

  .btn-launch:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn-launch:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
