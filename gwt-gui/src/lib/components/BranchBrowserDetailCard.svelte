<script lang="ts">
  import type { BranchInfo, BranchInventoryEntry } from "../types";

  let {
    selectedBranch,
    selectedEntry,
    actionLabel,
    onactivate,
  }: {
    selectedBranch: BranchInfo | null;
    selectedEntry: BranchInventoryEntry | null;
    actionLabel: string;
    onactivate: () => void;
  } = $props();
</script>

{#if selectedBranch}
  <div class="detail-card">
    <div class="detail-header">
      <span class="detail-kind">Selected</span>
      <span class="detail-title">{selectedBranch.display_name ?? selectedBranch.name}</span>
    </div>
    <div class="detail-grid">
      <div class="detail-row">
        <span class="detail-label">Branch</span>
        <span class="detail-value mono">{selectedBranch.name}</span>
      </div>
      <div class="detail-row">
        <span class="detail-label">Commit</span>
        <span class="detail-value mono">{selectedBranch.commit}</span>
      </div>
      <div class="detail-row">
        <span class="detail-label">Worktree</span>
        <span class="detail-value mono">{selectedEntry?.worktree?.path ?? "Not materialized"}</span>
      </div>
      <div class="detail-row">
        <span class="detail-label">Coverage</span>
        <span class="detail-value">
          {#if selectedEntry?.has_local && selectedEntry?.has_remote}
            Local + Remote
          {:else if selectedEntry?.has_local}
            Local
          {:else if selectedEntry?.has_remote}
            Remote
          {:else}
            Unknown
          {/if}
        </span>
      </div>
      <div class="detail-row">
        <span class="detail-label">Resolution</span>
        <span class="detail-value">
          {#if selectedEntry?.resolution_action === "focusExisting"}
            Existing worktree
          {:else if selectedEntry?.resolution_action === "resolveAmbiguity"}
            Multiple worktrees
          {:else}
            Create new worktree
          {/if}
        </span>
      </div>
    </div>
    <div class="detail-actions">
      <button
        type="button"
        class="cleanup-btn"
        disabled={selectedEntry?.resolution_action === "resolveAmbiguity"}
        onclick={onactivate}
      >
        {actionLabel}
      </button>
    </div>
  </div>
{:else}
  <div class="state-msg">Select a branch or worktree to inspect it.</div>
{/if}

<style>
  .detail-card {
    width: 100%;
    border: 1px solid var(--border-color);
    border-radius: 18px;
    background: color-mix(in srgb, var(--bg-secondary) 86%, transparent);
    overflow: hidden;
  }

  .detail-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--border-color);
  }

  .detail-kind {
    border-radius: 999px;
    padding: 4px 10px;
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }

  .detail-title {
    font-weight: 600;
  }

  .detail-grid {
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
    font-size: 0.78rem;
  }

  .detail-value {
    min-width: 0;
    word-break: break-word;
  }

  .mono {
    font-family: monospace;
  }

  .detail-actions {
    padding: 0 16px 16px;
  }

  .cleanup-btn {
    border: 1px solid var(--border-color);
    background: color-mix(in srgb, var(--bg-secondary) 80%, transparent);
    color: var(--text-primary);
    border-radius: 999px;
    padding: 7px 12px;
    cursor: pointer;
    font: inherit;
  }

  .state-msg {
    padding: 16px;
    color: var(--text-muted);
  }
</style>
