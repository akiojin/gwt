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
    stripRemotePrefix,
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

  type BranchBrowserEntry = {
    key: string;
    branch: BranchInfo;
    hasLocal: boolean;
    hasRemote: boolean;
    worktree: WorktreeInfo | null;
  };

  function branchKey(name: string): string {
    return name.trim().startsWith("origin/") ? stripRemotePrefix(name) : name.trim();
  }

  function buildEntries(
    local: BranchInfo[],
    remote: BranchInfo[],
    worktrees: WorktreeInfo[],
    filter: SidebarFilterType,
  ): BranchBrowserEntry[] {
    const worktreeByBranch = buildWorktreeMap(worktrees);

    if (filter === "Local") {
      return local.map((branch) => ({
        key: branchKey(branch.name),
        branch,
        hasLocal: true,
        hasRemote: false,
        worktree: worktreeByBranch.get(branchKey(branch.name)) ?? null,
      }));
    }

    if (filter === "Remote") {
      return remote.map((branch) => ({
        key: branchKey(branch.name),
        branch,
        hasLocal: false,
        hasRemote: true,
        worktree: worktreeByBranch.get(branchKey(branch.name)) ?? null,
      }));
    }

    const merged = new Map<string, BranchBrowserEntry>();
    for (const branch of local) {
      const key = branchKey(branch.name);
      merged.set(key, {
        key,
        branch,
        hasLocal: true,
        hasRemote: false,
        worktree: worktreeByBranch.get(key) ?? null,
      });
    }
    for (const branch of remote) {
      const key = branchKey(branch.name);
      const existing = merged.get(key);
      if (existing) {
        merged.set(key, {
          ...existing,
          hasRemote: true,
        });
      } else {
        merged.set(key, {
          key,
          branch,
          hasLocal: false,
          hasRemote: true,
          worktree: worktreeByBranch.get(key) ?? null,
        });
      }
    }
    return Array.from(merged.values());
  }

  let branchEntries = $state<BranchBrowserEntry[]>([]);

  let filteredBranches = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    return sortBranches(
      branchEntries
        .map((entry) => entry.branch)
        .filter((branch) => {
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
    if (!selectedBranchName) return null;
    return (
      worktreeMap.get(selectedBranchName) ??
      worktreeMap.get(selectedBranchName.replace(/^origin\//, "")) ??
      null
    );
  });
  let selectedEntry = $derived.by(() => {
    const selectedBranchName = config.selectedBranch?.name?.trim() ?? "";
    const key = branchKey(selectedBranchName);
    return branchEntries.find((entry) => entry.key === key) ?? null;
  });

  async function fetchBranches(path: string) {
    const token = ++requestToken;
    loading = true;
    errorMessage = null;

    try {
      const [local, remote, worktrees] = await Promise.all([
        invoke<BranchInfo[]>("list_worktree_branches", { projectPath: path }),
        invoke<BranchInfo[]>("list_remote_branches", { projectPath: path }),
        invoke<WorktreeInfo[]>("list_worktrees", { projectPath: path }),
      ]);
      if (token !== requestToken) return;
      branches = activeFilter === "Local" ? local : activeFilter === "Remote" ? remote : local;
      remoteBranchNames = new Set(remote.map((branch) => branchKey(branch.name)));
      worktreeMap = buildWorktreeMap(worktrees);
      branchEntries = buildEntries(local, remote, worktrees, activeFilter);
    } catch (error) {
      if (token !== requestToken) return;
      errorMessage =
        error instanceof Error ? error.message : String(error);
      branches = [];
      branchEntries = [];
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
              class:selected={selectedEntry?.key === branchKey(branch.name)}
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
            <div class="detail-row">
              <span class="detail-label">Coverage</span>
              <span class="detail-value">
                {#if selectedEntry?.hasLocal && selectedEntry?.hasRemote}
                  Local + Remote
                {:else if selectedEntry?.hasLocal}
                  Local
                {:else if selectedEntry?.hasRemote}
                  Remote
                {:else}
                  Unknown
                {/if}
              </span>
            </div>
          </div>
          <div class="detail-actions">
            <button
              type="button"
              class="cleanup-btn"
              onclick={() => config.onBranchActivate?.(config.selectedBranch!)}
            >
              {selectedWorktree ? "Focus Worktree" : "Create Worktree"}
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

    .browser-body {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
