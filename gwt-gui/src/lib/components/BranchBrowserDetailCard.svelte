<script lang="ts">
  import type { BranchInfo, BranchInventoryEntry } from "../types";

  let {
    selectedBranch,
    selectedEntry,
    worktreePath = null,
    detailLoading = false,
    detailErrorMessage = null,
    actionLabel,
    onactivate,
  }: {
    selectedBranch: BranchInfo | null;
    selectedEntry: BranchInventoryEntry | null;
    worktreePath?: string | null;
    detailLoading?: boolean;
    detailErrorMessage?: string | null;
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
        <span class="detail-value mono">{worktreePath ?? "Not materialized"}</span>
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
    {#if detailLoading}
      <div class="state-msg inline">Refreshing detail…</div>
    {:else if detailErrorMessage}
      <div class="state-msg error inline">{detailErrorMessage}</div>
    {/if}
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
    border-radius: var(--radius-xl);
    background: color-mix(in srgb, var(--bg-secondary) 86%, transparent);
    overflow: hidden;
  }

  .detail-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border-color);
  }

  .detail-kind {
    border-radius: var(--radius-full);
    padding: var(--space-1) var(--space-3);
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
    gap: var(--space-3);
    padding: var(--space-4);
  }

  .detail-row {
    display: grid;
    gap: var(--space-1);
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
    padding: 0 var(--space-4) var(--space-4);
  }

  .cleanup-btn {
    border: 1px solid var(--border-color);
    background: color-mix(in srgb, var(--bg-secondary) 80%, transparent);
    color: var(--text-primary);
    border-radius: var(--radius-full);
    padding: var(--space-2) var(--space-3);
    cursor: pointer;
    font: inherit;
  }

  .state-msg {
    padding: var(--space-4);
    color: var(--text-muted);
  }

  .state-msg.inline {
    padding-top: 0;
  }

  .state-msg.error {
    color: var(--red);
  }
</style>
