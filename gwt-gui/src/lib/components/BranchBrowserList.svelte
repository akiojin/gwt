<script lang="ts">
  import type { BranchInfo, BranchInventoryEntry } from "../types";
  import {
    divergenceClass,
    divergenceIndicator,
  } from "./sidebarHelpers";

  let {
    loading,
    errorMessage,
    entries,
    selectedEntryId = null,
    onselect,
    onactivate,
  }: {
    loading: boolean;
    errorMessage: string | null;
    entries: BranchInventoryEntry[];
    selectedEntryId?: string | null;
    onselect: (branch: BranchInfo) => void;
    onactivate: (entry: BranchInventoryEntry) => void;
  } = $props();
</script>

{#if loading}
  <div class="state-msg">Loading branches...</div>
{:else if errorMessage}
  <div class="state-msg error">{errorMessage}</div>
{:else if entries.length === 0}
  <div class="state-msg">No branches found.</div>
{:else}
  <div class="branch-list">
    {#each entries as entry (entry.id)}
      <button
        type="button"
        class="branch-row"
        class:selected={selectedEntryId === entry.id}
        onclick={() => onselect(entry.primary_branch)}
        ondblclick={() =>
          entry.resolution_action !== "resolveAmbiguity" && onactivate(entry)}
      >
        <div class="branch-primary">
          <span class="branch-name">{entry.primary_branch.display_name ?? entry.primary_branch.name}</span>
          {#if entry.primary_branch.display_name && entry.primary_branch.display_name !== entry.primary_branch.name}
            <span class="branch-sub">{entry.primary_branch.name}</span>
          {:else}
            <span class="branch-sub">{entry.primary_branch.name}</span>
          {/if}
        </div>
        <div class="branch-meta">
          {#if divergenceIndicator(entry.primary_branch)}
            <span
              class={`divergence-pill ${divergenceClass(entry.primary_branch.divergence_status)}`}
            >
              {divergenceIndicator(entry.primary_branch)}
            </span>
          {/if}
        </div>
      </button>
    {/each}
  </div>
{/if}

<style>
  .state-msg {
    padding: 16px;
    color: var(--text-muted);
  }

  .state-msg.error {
    color: var(--red);
  }

  .branch-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .branch-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 14px 16px;
    border-radius: 16px;
    border: 1px solid color-mix(in srgb, var(--border-color) 84%, transparent);
    background: color-mix(in srgb, var(--bg-secondary) 72%, transparent);
    color: var(--text-primary);
    text-align: left;
    cursor: pointer;
  }

  .branch-row.selected {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }

  .branch-primary {
    min-width: 0;
  }

  .branch-name,
  .branch-sub {
    min-width: 0;
    display: block;
  }

  .branch-sub {
    color: var(--text-muted);
    margin-top: 2px;
  }

  .branch-meta {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .divergence-pill {
    border-radius: 999px;
    padding: 4px 10px;
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
</style>
