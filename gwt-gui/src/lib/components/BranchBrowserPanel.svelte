<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "$lib/tauriInvoke";
  import type { BranchBrowserPanelConfig, BranchInfo, WorktreeInfo } from "../types";
  import {
    buildWorktreeMap,
    divergenceClass,
    divergenceIndicator,
    getSafetyLevel,
    getSafetyTitle,
    sortBranches,
    type SidebarFilterType,
  } from "./sidebarHelpers";

  let { config }: { config: BranchBrowserPanelConfig } = $props();

  let activeFilter: SidebarFilterType = $state("Local");
  let searchQuery = $state("");
  let loading = $state(true);
  let errorMessage: string | null = $state(null);
  let branches: BranchInfo[] = $state([]);
  let remoteBranchNames = $state(new Set<string>());
  let worktreeMap = $state(new Map<string, WorktreeInfo>());
  let requestToken = 0;

  const filters: SidebarFilterType[] = ["Local", "Remote", "All"];

  let filteredBranches = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    return sortBranches(
      branches.filter((branch) => {
        if (!q) return true;
        const haystack = `${branch.display_name ?? ""} ${branch.name}`.toLowerCase();
        return haystack.includes(q);
      }),
      activeFilter,
      remoteBranchNames,
      "name",
    );
  });

  let selectedWorktree = $derived.by(() => {
    const selectedBranchName = config.selectedBranch?.name?.trim() ?? "";
    return selectedBranchName ? worktreeMap.get(selectedBranchName) ?? null : null;
  });

  async function fetchBranches(path: string) {
    const token = ++requestToken;
    loading = true;
    errorMessage = null;

    try {
      if (activeFilter === "Local") {
        const [local, worktrees] = await Promise.all([
          invoke<BranchInfo[]>("list_worktree_branches", { projectPath: path }),
          invoke<WorktreeInfo[]>("list_worktrees", { projectPath: path }),
        ]);
        if (token !== requestToken) return;
        branches = local;
        remoteBranchNames = new Set();
        worktreeMap = buildWorktreeMap(worktrees);
      } else if (activeFilter === "Remote") {
        const [remote, worktrees] = await Promise.all([
          invoke<BranchInfo[]>("list_remote_branches", { projectPath: path }),
          invoke<WorktreeInfo[]>("list_worktrees", { projectPath: path }),
        ]);
        if (token !== requestToken) return;
        branches = remote;
        remoteBranchNames = new Set(remote.map((branch) => branch.name.trim()));
        worktreeMap = buildWorktreeMap(worktrees);
      } else {
        const [local, remote, worktrees] = await Promise.all([
          invoke<BranchInfo[]>("list_worktree_branches", { projectPath: path }),
          invoke<BranchInfo[]>("list_remote_branches", { projectPath: path }),
          invoke<WorktreeInfo[]>("list_worktrees", { projectPath: path }),
        ]);
        if (token !== requestToken) return;
        const merged = new Map<string, BranchInfo>();
        for (const branch of local) {
          merged.set(branch.name, branch);
        }
        for (const branch of remote) {
          if (merged.has(branch.name)) continue;
          merged.set(branch.name, branch);
        }
        branches = Array.from(merged.values());
        remoteBranchNames = new Set(remote.map((branch) => branch.name.trim()));
        worktreeMap = buildWorktreeMap(worktrees);
      }
    } catch (error) {
      if (token !== requestToken) return;
      errorMessage =
        error instanceof Error ? error.message : String(error);
      branches = [];
      remoteBranchNames = new Set();
      worktreeMap = new Map();
    } finally {
      if (token === requestToken) {
        loading = false;
      }
    }
  }

  onMount(() => {
    if (!config.projectPath) return;
    void fetchBranches(config.projectPath);
  });

  $effect(() => {
    const path = config.projectPath;
    const refreshKey = config.refreshKey;
    const filter = activeFilter;
    void refreshKey;
    if (!path) return;
    void filter;
    void fetchBranches(path);
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

  <div class="browser-body">
    <section class="branch-list-panel">
      {#if loading}
        <div class="state-msg">Loading branches...</div>
      {:else if errorMessage}
        <div class="state-msg error">{errorMessage}</div>
      {:else if filteredBranches.length === 0}
        <div class="state-msg">No branches found.</div>
      {:else}
        <div class="branch-list">
          {#each filteredBranches as branch}
            <button
              type="button"
              class="branch-row"
              class:selected={config.selectedBranch?.name === branch.name}
              onclick={() => config.onBranchSelect(branch)}
              ondblclick={() => config.onBranchActivate?.(branch)}
            >
              <div class="branch-primary">
                <span class="branch-name">{branch.display_name ?? branch.name}</span>
                {#if branch.display_name && branch.display_name !== branch.name}
                  <span class="branch-sub">{branch.name}</span>
                {/if}
              </div>
              <div class="branch-meta">
                {#if getSafetyLevel(branch, worktreeMap)}
                  <span
                    class={`safety-pill ${getSafetyLevel(branch, worktreeMap)}`}
                    title={getSafetyTitle(branch, worktreeMap)}
                  >
                    {getSafetyLevel(branch, worktreeMap)}
                  </span>
                {/if}
                {#if divergenceIndicator(branch)}
                  <span class={`divergence-pill ${divergenceClass(branch.divergence_status)}`}>
                    {divergenceIndicator(branch)}
                  </span>
                {/if}
              </div>
            </button>
          {/each}
        </div>
      {/if}
    </section>

    <section class="detail-panel" data-testid="branch-browser-detail">
      {#if config.selectedBranch}
        <div class="detail-card">
          <div class="detail-header">
            <span class="detail-kind">Selected</span>
            <span class="detail-title">{config.selectedBranch.display_name ?? config.selectedBranch.name}</span>
          </div>
          <div class="detail-grid">
            <div class="detail-row">
              <span class="detail-label">Branch</span>
              <span class="detail-value mono">{config.selectedBranch.name}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Commit</span>
              <span class="detail-value mono">{config.selectedBranch.commit}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Worktree</span>
              <span class="detail-value mono">{selectedWorktree?.path ?? "Not materialized"}</span>
            </div>
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
    display: grid;
    grid-template-columns: minmax(280px, 420px) minmax(280px, 1fr);
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
    max-height: 100%;
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
    align-items: stretch;
    justify-content: stretch;
  }

  .detail-card {
    width: 100%;
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

    .browser-body {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
