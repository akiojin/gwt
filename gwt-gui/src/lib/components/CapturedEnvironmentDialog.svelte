<script lang="ts">
  import type { CapturedEnvInfo } from "../types";

  let {
    open,
    loading,
    error,
    data,
    onclose,
  }: {
    open: boolean;
    loading: boolean;
    error: string | null;
    data: CapturedEnvInfo | null;
    onclose: () => void;
  } = $props();
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay modal-overlay" onclick={onclose}>
    <div class="env-debug-dialog modal-dialog-shell" onclick={(e) => e.stopPropagation()}>
      <h3>Captured Environment</h3>
      {#if loading}
        <p class="env-debug-loading">Loading...</p>
      {:else if error}
        <p class="env-debug-error">{error}</p>
      {:else if data}
        <div class="env-debug-meta">
          <span>Source: <strong>{data.source === "login_shell" ? "Login Shell" : data.source === "std_env_fallback" ? "Process Env (fallback)" : data.source}</strong></span>
          {#if data.reason}
            <span class="env-debug-reason">Reason: {data.reason}</span>
          {/if}
          <span>Variables: {data.entries.length}</span>
        </div>
        <div class="env-debug-list">
          {#each data.entries as entry}
            <div class="env-debug-row">
              <span class="env-debug-key">{entry.key}</span>
              <span class="env-debug-val">{entry.value}</span>
            </div>
          {/each}
        </div>
      {/if}
      <button class="about-close" onclick={onclose}>Close</button>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: var(--z-modal-base);
  }

  .about-close {
    padding: 6px 20px;
    border-radius: 999px;
    border: 1px solid var(--border-color);
    background: var(--bg-tertiary);
    color: var(--text-primary);
    cursor: pointer;
    align-self: flex-end;
  }

  .about-close:hover {
    background: var(--bg-hover);
  }

  .env-debug-dialog {
    background: var(--bg-secondary);
    color: var(--text-primary);
    border-radius: var(--radius-lg);
    width: min(760px, 94vw);
    max-height: min(620px, 86vh);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .env-debug-dialog h3 {
    margin: 0 0 16px;
  }

  .env-debug-meta {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    font-size: 13px;
  }

  .env-debug-reason {
    color: var(--text-warning, #f9e2af);
  }

  .env-debug-list {
    overflow-y: auto;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    background: var(--bg-tertiary);
  }

  .env-debug-row {
    display: flex;
    gap: 12px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border-color);
  }

  .env-debug-row:last-child {
    border-bottom: none;
  }

  .env-debug-key {
    min-width: 200px;
    color: var(--text-muted);
    font-family: monospace;
  }

  .env-debug-val {
    flex: 1;
    word-break: break-all;
    font-family: monospace;
  }

  .env-debug-loading,
  .env-debug-error {
    font-size: 13px;
  }

  .env-debug-error {
    color: var(--text-error, #f38ba8);
  }
</style>
