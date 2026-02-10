<script lang="ts">
  import type { StashEntry } from "../types";

  let {
    projectPath,
  }: {
    projectPath: string;
  } = $props();

  let entries: StashEntry[] = $state([]);
  let loading: boolean = $state(true);
  let error: string | null = $state(null);

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  async function load() {
    loading = true;
    error = null;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      entries = await invoke<StashEntry[]>("get_stash_list", { projectPath });
    } catch (err) {
      error = toErrorMessage(err);
      entries = [];
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void projectPath;
    load();
  });
</script>

<div class="stash-tab">
  {#if loading}
    <div class="stash-loading">Loading...</div>
  {:else if error}
    <div class="stash-error">{error}</div>
  {:else if entries.length === 0}
    <div class="stash-empty">No stash entries</div>
  {:else}
    <div class="stash-list">
      {#each entries as entry (entry.index)}
        <div class="stash-row">
          <span class="stash-label mono">stash@&#123;{entry.index}&#125;:</span>
          <span class="stash-message">{entry.message}</span>
          <span class="stash-files">({entry.file_count} file{entry.file_count === 1 ? "" : "s"})</span>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .stash-tab {
    padding: 8px 0;
  }

  .stash-loading,
  .stash-empty {
    font-size: 12px;
    color: var(--text-muted);
    padding: 8px 0;
  }

  .stash-error {
    padding: 10px 12px;
    border: 1px solid rgba(255, 0, 0, 0.35);
    background: rgba(255, 0, 0, 0.08);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: 12px;
  }

  .stash-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .stash-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 6px 8px;
    border-radius: 4px;
    font-size: 12px;
  }

  .stash-row:hover {
    background: var(--bg-hover);
  }

  .stash-label {
    color: var(--yellow);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .stash-message {
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .stash-files {
    color: var(--text-muted);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .mono {
    font-family: monospace;
  }
</style>
