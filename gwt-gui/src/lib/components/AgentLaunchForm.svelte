<script lang="ts">
  import type { AgentInfo } from "../types";

  let {
    selectedBranch = "",
    onLaunch,
    onClose,
  }: {
    selectedBranch?: string;
    onLaunch: (request: {
      agentId: string;
      branch: string;
      model?: string;
      agentVersion?: string;
      createBranch?: { name: string; base?: string | null };
    }) => Promise<void>;
    onClose: () => void;
  } = $props();

  type LaunchMode = "existing" | "new";

  let agents: AgentInfo[] = $state([]);
  let selectedAgent: string = $state("");
  let mode: LaunchMode = $state("existing");
  let model: string = $state("");
  let agentVersion: string = $state("latest");

  // Intentionally capture the initial values - fields are editable by the user
  let branch: string = $state((() => selectedBranch)());
  let baseBranch: string = $state((() => selectedBranch)());
  let newBranch: string = $state("");

  let loading: boolean = $state(true);
  let launching: boolean = $state(false);
  let errorMessage: string | null = $state(null);

  let selectedAgentInfo = $derived(
    agents.find((a) => a.id === selectedAgent) ?? null
  );
  let isFallbackAgent = $derived(
    selectedAgentInfo?.version === "bunx" || selectedAgentInfo?.version === "npx"
  );
  let supportsModel = $derived(
    selectedAgent === "codex" || selectedAgent === "claude"
  );

  $effect(() => {
    detectAgents();
  });

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  async function detectAgents() {
    loading = true;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      agents = await invoke<AgentInfo[]>("detect_agents");
      const available = agents.find((a) => a.available);
      if (available) selectedAgent = available.id;
    } catch (err) {
      console.error("Failed to detect agents:", err);
      agents = [];
    }
    loading = false;
  }

  async function handleLaunch() {
    errorMessage = null;
    if (!selectedAgent) return;
    launching = true;
    try {
      const options: { model?: string; agentVersion?: string } = {};
      if (supportsModel && model.trim()) {
        options.model = model.trim();
      }
      if (isFallbackAgent && agentVersion.trim()) {
        options.agentVersion = agentVersion.trim();
      }

      if (mode === "existing") {
        if (!branch.trim()) return;
        await onLaunch({
          agentId: selectedAgent,
          branch: branch.trim(),
          ...options,
        });
        onClose();
        return;
      }

      if (!baseBranch.trim() || !newBranch.trim()) return;
      await onLaunch({
        agentId: selectedAgent,
        branch: newBranch.trim(),
        ...options,
        createBranch: { name: newBranch.trim(), base: baseBranch.trim() },
      });
      onClose();
    } catch (err) {
      errorMessage = `Failed to launch agent: ${toErrorMessage(err)}`;
    } finally {
      launching = false;
    }
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
        {#if errorMessage}
          <div class="error">{errorMessage}</div>
        {/if}

        <div class="field">
          <label for="agent-select">Agent</label>
          <div class="agent-cards">
            {#each agents as agent}
              <button
                class="agent-card"
                class:selected={selectedAgent === agent.id}
                class:unavailable={!agent.available}
                disabled={!agent.available}
                onclick={() => (selectedAgent = agent.id)}
              >
                <span class="agent-name">{agent.name}</span>
                <span class="agent-type">{agent.version}</span>
                {#if !agent.available}
                  <span class="agent-status">Unavailable</span>
                {:else if agent.version === "bunx" || agent.version === "npx"}
                  <span class="agent-status">
                    Not installed ({agent.version})
                  </span>
                {:else if !agent.authenticated}
                  <span class="agent-status">Not authenticated</span>
                {/if}
              </button>
            {/each}
          </div>
        </div>

        {#if supportsModel}
          <div class="field">
            <label for="model-input">Model</label>
            <input
              id="model-input"
              type="text"
              bind:value={model}
              placeholder="Optional (e.g., gpt-5.2, sonnet)"
            />
          </div>
        {/if}

        {#if isFallbackAgent}
          <div class="field">
            <label for="agent-version-input">Agent Version</label>
            <input
              id="agent-version-input"
              type="text"
              bind:value={agentVersion}
              placeholder="latest"
            />
          </div>
        {/if}

        <div class="field">
          <span class="field-label" id="launch-mode-label">Mode</span>
          <div class="mode-toggle" role="group" aria-labelledby="launch-mode-label">
            <button
              class="mode-btn"
              class:active={mode === "existing"}
              onclick={() => (mode = "existing")}
            >
              Existing Branch
            </button>
            <button
              class="mode-btn"
              class:active={mode === "new"}
              onclick={() => (mode = "new")}
            >
              New Branch
            </button>
          </div>
        </div>

        {#if mode === "existing"}
        <div class="field">
          <label for="branch-input">Branch</label>
          <input
            id="branch-input"
            type="text"
            bind:value={branch}
            placeholder="Enter branch name..."
          />
        </div>
        {:else}
          <div class="field">
            <label for="base-branch-input">Base Branch</label>
            <input
              id="base-branch-input"
              type="text"
              bind:value={baseBranch}
              placeholder="e.g., develop or origin/develop"
            />
          </div>
          <div class="field">
            <label for="new-branch-input">New Branch Name</label>
            <input
              id="new-branch-input"
              type="text"
              bind:value={newBranch}
              placeholder="e.g., feature/my-change"
            />
          </div>
        {/if}
      </div>

      <div class="dialog-footer">
        <button class="btn btn-cancel" onclick={onClose}>Cancel</button>
        <button
          class="btn btn-launch"
          disabled={
            launching ||
            !selectedAgent ||
            (mode === "existing" ? !branch.trim() : !baseBranch.trim() || !newBranch.trim())
          }
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

  .error {
    padding: 10px 12px;
    border: 1px solid rgba(255, 0, 0, 0.35);
    background: rgba(255, 0, 0, 0.08);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: 12px;
    line-height: 1.4;
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

  .field-label {
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

  .mode-toggle {
    display: flex;
    gap: 6px;
  }

  .mode-btn {
    flex: 1;
    padding: 8px 10px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s, background 0.15s;
  }

  .mode-btn:hover {
    border-color: var(--accent);
  }

  .mode-btn.active {
    border-color: var(--accent);
    background: var(--bg-surface);
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
