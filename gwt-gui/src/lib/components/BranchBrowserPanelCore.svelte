<script lang="ts">
  import { invoke } from "$lib/tauriInvoke";
  import type {
    BranchBrowserPanelConfig,
    BranchBrowserPanelState,
    BranchInfo,
    BranchInventoryEntry,
  } from "../types";
  import {
    actionLabelRuntime,
    buildBranchBrowserStateRuntime,
    buildFetchRequestKeyRuntime,
    buildFilteredBranchEntriesRuntime,
    buildHydrationKeyRuntime,
    buildRemotePrimaryNamesRuntime,
    resolveSelectedBranchRuntime,
    resolveSelectedEntryRuntime,
  } from "../branchBrowserRuntime";
  import {
    divergenceClass,
    divergenceIndicator,
    safetyTitleForLevel,
    type SidebarFilterType,
  } from "./sidebarHelpers";
  import BranchBrowserDetailCard from "./BranchBrowserDetailCard.svelte";
  import BranchBrowserList from "./BranchBrowserList.svelte";

  let { config }: { config: BranchBrowserPanelConfig } = $props();

  let activeFilter: SidebarFilterType = $state("Local");
  let searchQuery = $state("");
  let loading = $state(true);
  let errorMessage: string | null = $state(null);
  let remotePrimaryNames = $state(new Set<string>());
  let requestToken = 0;
  let lastFetchRequestKey = $state("");
  let lastHydrationKey = $state("");
  let lastStateEmitKey = $state("");

  const filters: SidebarFilterType[] = ["Local", "Remote", "All"];

  let branchEntries = $state<BranchInventoryEntry[]>([]);

  let filteredEntries = $derived.by(() => {
    return buildFilteredBranchEntriesRuntime({
      branchEntries,
      activeFilter,
      searchQuery,
      remotePrimaryNames,
    });
  });

  let selectedEntry = $derived.by(() => {
    return resolveSelectedEntryRuntime({
      config,
      branchEntries,
    });
  });
  let resolvedSelectedBranch = $derived<BranchInfo | null>(
    resolveSelectedBranchRuntime({
      config,
      selectedEntry,
    }),
  );

  async function fetchBranches(path: string) {
    const token = ++requestToken;
    loading = true;
    errorMessage = null;

    try {
      const entries = await invoke<BranchInventoryEntry[]>("list_branch_inventory", {
        projectPath: path,
      });
      if (token !== requestToken) return;
      branchEntries = entries;
      remotePrimaryNames = buildRemotePrimaryNamesRuntime(entries);
    } catch (error) {
      if (token !== requestToken) return;
      errorMessage = error instanceof Error ? error.message : String(error);
      branchEntries = [];
      remotePrimaryNames = new Set();
    } finally {
      if (token === requestToken) {
        loading = false;
      }
    }
  }

  $effect(() => {
    const path = config.projectPath;
    const refreshKey = config.refreshKey;
    void refreshKey;
    if (!path) return;
    const nextFetchRequestKey = buildFetchRequestKeyRuntime(path, refreshKey);
    if (nextFetchRequestKey === lastFetchRequestKey) return;
    lastFetchRequestKey = nextFetchRequestKey;
    void fetchBranches(path);
  });

  $effect(() => {
    const nextKey = buildHydrationKeyRuntime(config);
    if (nextKey === lastHydrationKey) return;
    lastHydrationKey = nextKey;
    activeFilter = config.initialFilter ?? "Local";
    searchQuery = config.initialQuery ?? "";
  });

  $effect(() => {
    const nextState: BranchBrowserPanelState = buildBranchBrowserStateRuntime({
      activeFilter,
      searchQuery,
      config,
    });
    const nextKey = JSON.stringify(nextState);
    if (nextKey === lastStateEmitKey) return;
    lastStateEmitKey = nextKey;
    config.onStateChange?.(nextState);
  });
</script>

<div class="branch-browser-panel" data-testid="branch-browser-panel">
  <div class="browser-header">
    <div>
      <h2>Branch Browser</h2>
      <p>Browse `Local`, `Remote`, and `All` refs without reopening the old sidebar.</p>
    </div>
    <button
      type="button"
      class="cleanup-btn"
      onclick={() => config.onCleanupRequest?.()}
    >
      Cleanup
    </button>
  </div>

  <div class="browser-toolbar">
    <div class="filter-row">
      {#each filters as filter}
        <button
          type="button"
          class="filter-btn"
          class:active={activeFilter === filter}
          onclick={() => (activeFilter = filter)}
        >
          {filter}
        </button>
      {/each}
    </div>
    <input
      type="text"
      class="search-input"
      placeholder="Filter branches..."
      bind:value={searchQuery}
    />
  </div>

  <div class="browser-body single-surface" data-testid="branch-browser-surface">
    <section class="detail-panel" data-testid="branch-browser-detail">
      <BranchBrowserDetailCard
        selectedBranch={resolvedSelectedBranch}
        {selectedEntry}
        actionLabel={selectedEntry ? actionLabelRuntime(selectedEntry) : "Create Worktree"}
        onactivate={() => {
          if (selectedEntry?.resolution_action === "resolveAmbiguity") return;
          config.onBranchActivate?.(resolvedSelectedBranch!);
        }}
      />
    </section>

    <section class="branch-list-panel">
      <BranchBrowserList
        {loading}
        {errorMessage}
        entries={filteredEntries}
        selectedEntryId={selectedEntry?.id ?? null}
        onselect={(branch) => config.onBranchSelect(branch)}
        onactivate={(entry) => config.onBranchActivate?.(entry.primary_branch)}
      />
    </section>
  </div>
</div>

<style>
  .branch-browser-panel {
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 18px 18px;
    background:
      linear-gradient(180deg, color-mix(in srgb, var(--bg-secondary) 88%, transparent), var(--bg-primary)),
      radial-gradient(circle at top right, color-mix(in srgb, var(--cyan) 12%, transparent), transparent 32%);
    overflow: hidden;
  }

  .browser-header,
  .browser-toolbar,
  .filter-row,
  .branch-meta,
  .detail-header {
    display: flex;
    align-items: center;
  }

  .browser-header,
  .browser-toolbar {
    justify-content: space-between;
    gap: 16px;
  }

  .browser-header h2 {
    margin: 0;
    font-size: 1rem;
  }

  .browser-header p {
    margin: 4px 0 0;
    color: var(--text-muted);
  }

  .cleanup-btn,
  .filter-btn {
    border: 1px solid var(--border-color);
    background: color-mix(in srgb, var(--bg-secondary) 80%, transparent);
    color: var(--text-primary);
    border-radius: 999px;
    padding: 7px 12px;
    cursor: pointer;
    font: inherit;
  }

  .cleanup-btn:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }

  .filter-btn.active {
    border-color: color-mix(in srgb, var(--accent) 58%, var(--border-color));
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }

  .filter-row {
    gap: 8px;
    flex-wrap: wrap;
  }

  .search-input {
    min-width: 240px;
    border: 1px solid var(--border-color);
    background: var(--bg-primary);
    color: var(--text-primary);
    border-radius: 10px;
    padding: 8px 10px;
    font: inherit;
  }

  .browser-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .branch-list-panel,
  .detail-card,
  .detail-panel {
    min-width: 0;
    min-height: 0;
  }

  .branch-list-panel,
  .detail-card,
  .detail-panel {
    border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
    background: color-mix(in srgb, var(--bg-secondary) 82%, var(--bg-primary));
    border-radius: 16px;
    overflow: hidden;
  }

  .branch-list {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
    overflow: auto;
  }

  .branch-row {
    display: flex;
    justify-content: space-between;
    gap: 14px;
    width: 100%;
    padding: 12px 14px;
    border: none;
    border-bottom: 1px solid color-mix(in srgb, var(--border-color) 68%, transparent);
    background: transparent;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }

  .branch-row.selected {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }

  .branch-primary {
    min-width: 0;
    display: grid;
    gap: 4px;
  }

  .branch-name,
  .branch-sub,
  .detail-value {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .branch-sub {
    color: var(--text-muted);
    font-size: 0.85rem;
  }

  .branch-meta {
    gap: 8px;
    flex-shrink: 0;
  }

  .safety-pill,
  .divergence-pill,
  .detail-kind {
    border-radius: 999px;
    padding: 4px 8px;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .detail-kind {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }

  .safety-pill.safe {
    background: color-mix(in srgb, var(--green) 16%, transparent);
  }

  .safety-pill.warning {
    background: color-mix(in srgb, var(--yellow) 16%, transparent);
  }

  .safety-pill.danger,
  .safety-pill.disabled {
    background: color-mix(in srgb, var(--red) 16%, transparent);
  }

  .detail-panel {
    display: flex;
    flex: 0 0 auto;
  }

  .detail-card {
    width: 100%;
  }

  .branch-list-panel {
    flex: 1 1 auto;
  }

  .detail-header {
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid color-mix(in srgb, var(--border-color) 68%, transparent);
  }

  .detail-title {
    font-weight: 600;
  }

  .detail-grid {
    display: grid;
    gap: 12px;
    padding: 16px;
  }

  .detail-actions {
    padding: 0 16px 16px;
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

  .mono {
    font-family: monospace;
  }

  .state-msg {
    padding: 16px;
    color: var(--text-muted);
  }

  .state-msg.error {
    color: var(--red);
  }

  @media (max-width: 980px) {
    .browser-toolbar,
    .browser-header {
      flex-direction: column;
      align-items: stretch;
    }

    .search-input {
      min-width: 0;
      width: 100%;
    }
  }
</style>
