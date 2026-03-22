<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "$lib/tauriInvoke";
  import type {
    BranchBrowserPanelConfig,
    BranchBrowserPanelState,
    BranchInfo,
    BranchInventoryEntry,
  } from "../types";
  import { branchInventoryKey } from "../branchInventory";
  import {
    divergenceClass,
    divergenceIndicator,
    safetyTitleForLevel,
    sortBranches,
    type SidebarFilterType,
  } from "./sidebarHelpers";

  let { config }: { config: BranchBrowserPanelConfig } = $props();

  let activeFilter: SidebarFilterType = $state("Local");
  let searchQuery = $state("");
  let loading = $state(true);
  let errorMessage: string | null = $state(null);
  let remotePrimaryNames = $state(new Set<string>());
  let requestToken = 0;
  let lastHydrationKey = $state("");
  let lastStateEmitKey = $state("");

  const filters: SidebarFilterType[] = ["Local", "Remote", "All"];

  let branchEntries = $state<BranchInventoryEntry[]>([]);

  function matchesFilter(
    entry: BranchInventoryEntry,
    filter: SidebarFilterType,
  ): boolean {
    if (filter === "Local") return entry.has_local;
    if (filter === "Remote") return entry.has_remote;
    return true;
  }

  let filteredEntries = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    const matchingEntries = branchEntries.filter((entry) => {
      if (!matchesFilter(entry, activeFilter)) return false;
      if (!q) return true;
      const branch = entry.primary_branch;
      const haystack =
        `${branch.display_name ?? ""} ${branch.name} ${entry.canonical_name}`.toLowerCase();
      return haystack.includes(q);
    });
    const sortedBranches = sortBranches(
      matchingEntries.map((entry) => entry.primary_branch),
      activeFilter,
      remotePrimaryNames,
      "name",
    );
    const orderedNames = new Map(
      sortedBranches.map((branch, index) => [branch.name, index]),
    );
    return [...matchingEntries].sort(
      (a, b) =>
        (orderedNames.get(a.primary_branch.name) ?? Number.MAX_SAFE_INTEGER) -
        (orderedNames.get(b.primary_branch.name) ?? Number.MAX_SAFE_INTEGER),
    );
  });

  let selectedEntry = $derived.by(() => {
    const selectedBranchName =
      config.selectedBranch?.name?.trim() ??
      config.selectedBranchName?.trim() ??
      "";
    const key = branchInventoryKey(selectedBranchName);
    return branchEntries.find((entry) => entry.canonical_name === key) ?? null;
  });
  let resolvedSelectedBranch = $derived<BranchInfo | null>(
    config.selectedBranch ?? selectedEntry?.primary_branch ?? null,
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
      remotePrimaryNames = new Set(
        entries
          .filter((entry) => !entry.has_local && entry.has_remote)
          .map((entry) => entry.primary_branch.name),
      );
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

  function actionLabel(entry: BranchInventoryEntry): string {
    switch (entry.resolution_action) {
      case "focusExisting":
        return "Focus Worktree";
      case "resolveAmbiguity":
        return "Resolve Ambiguity";
      default:
        return "Create Worktree";
    }
  }

  onMount(() => {
    if (!config.projectPath) return;
    void fetchBranches(config.projectPath);
  });

  $effect(() => {
    const path = config.projectPath;
    const refreshKey = config.refreshKey;
    void refreshKey;
    if (!path) return;
    void fetchBranches(path);
  });

  $effect(() => {
    const nextKey = JSON.stringify([
      config.initialFilter ?? "Local",
      config.initialQuery ?? "",
    ]);
    if (nextKey === lastHydrationKey) return;
    lastHydrationKey = nextKey;
    activeFilter = config.initialFilter ?? "Local";
    searchQuery = config.initialQuery ?? "";
  });

  $effect(() => {
    const nextState: BranchBrowserPanelState = {
      filter: activeFilter,
      query: searchQuery,
      selectedBranchName:
        config.selectedBranch?.name?.trim() ??
        config.selectedBranchName?.trim() ??
        null,
    };
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
      {#if resolvedSelectedBranch}
        <div class="detail-card">
          <div class="detail-header">
            <span class="detail-kind">Selected</span>
            <span class="detail-title">{resolvedSelectedBranch.display_name ?? resolvedSelectedBranch.name}</span>
          </div>
          <div class="detail-grid">
            <div class="detail-row">
              <span class="detail-label">Branch</span>
              <span class="detail-value mono">{resolvedSelectedBranch.name}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Commit</span>
              <span class="detail-value mono">{resolvedSelectedBranch.commit}</span>
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
              onclick={() => {
                if (selectedEntry?.resolution_action === "resolveAmbiguity") return;
                config.onBranchActivate?.(resolvedSelectedBranch!);
              }}
            >
              {selectedEntry ? actionLabel(selectedEntry) : "Create Worktree"}
            </button>
          </div>
        </div>
      {:else}
        <div class="state-msg">Select a branch or worktree to inspect it.</div>
      {/if}
    </section>

    <section class="branch-list-panel">
      {#if loading}
        <div class="state-msg">Loading branches...</div>
      {:else if errorMessage}
        <div class="state-msg error">{errorMessage}</div>
      {:else if filteredEntries.length === 0}
        <div class="state-msg">No branches found.</div>
      {:else}
        <div class="branch-list">
          {#each filteredEntries as entry (entry.id)}
            <button
              type="button"
              class="branch-row"
              class:selected={selectedEntry?.id === entry.id}
              onclick={() => config.onBranchSelect(entry.primary_branch)}
              ondblclick={() =>
                entry.resolution_action !== "resolveAmbiguity" &&
                config.onBranchActivate?.(entry.primary_branch)}
            >
              <div class="branch-primary">
                <span class="branch-name">{entry.primary_branch.display_name ?? entry.primary_branch.name}</span>
                {#if entry.primary_branch.display_name && entry.primary_branch.display_name !== entry.primary_branch.name}
                  <span class="branch-sub">{entry.primary_branch.name}</span>
                {/if}
              </div>
              <div class="branch-meta">
                {#if entry.worktree?.safety_level}
                  <span
                    class={`safety-pill ${entry.worktree.safety_level}`}
                    title={safetyTitleForLevel(entry.worktree.safety_level)}
                  >
                    {entry.worktree.safety_level}
                  </span>
                {/if}
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
    </section>

    <section class="detail-panel" data-testid="branch-browser-detail">
      {#if resolvedSelectedBranch}
        <div class="detail-card">
          <div class="detail-header">
            <span class="detail-kind">Selected</span>
            <span class="detail-title">{resolvedSelectedBranch.display_name ?? resolvedSelectedBranch.name}</span>
          </div>
          <div class="detail-grid">
            <div class="detail-row">
              <span class="detail-label">Branch</span>
              <span class="detail-value mono">{resolvedSelectedBranch.name}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Commit</span>
              <span class="detail-value mono">{resolvedSelectedBranch.commit}</span>
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
              onclick={() => {
                if (selectedEntry?.resolution_action === "resolveAmbiguity") return;
                config.onBranchActivate?.(resolvedSelectedBranch!);
              }}
            >
              {selectedEntry ? actionLabel(selectedEntry) : "Create Worktree"}
            </button>
          </div>
        </div>
      {:else}
        <div class="state-msg">Select a branch or worktree to inspect it.</div>
      {/if}
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
