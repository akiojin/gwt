<script lang="ts">
  import type { BranchInfo } from "../types";

  type FilterType = "Local" | "Remote" | "All";

  let {
    projectPath,
    onBranchSelect,
  }: {
    projectPath: string;
    onBranchSelect: (branch: BranchInfo) => void;
  } = $props();

  let activeFilter: FilterType = $state("Local");
  let branches: BranchInfo[] = $state([]);
  let loading: boolean = $state(false);
  let searchQuery: string = $state("");

  const filters: FilterType[] = ["Local", "Remote", "All"];

  let filteredBranches = $derived(
    searchQuery
      ? branches.filter((b) =>
          b.name.toLowerCase().includes(searchQuery.toLowerCase())
        )
      : branches
  );

  $effect(() => {
    // Re-fetch when filter or projectPath changes
    void activeFilter;
    void projectPath;
    fetchBranches();
  });

  async function fetchBranches() {
    loading = true;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      if (activeFilter === "Local") {
        branches = await invoke<BranchInfo[]>("list_branches", { projectPath });
      } else if (activeFilter === "Remote") {
        branches = await invoke<BranchInfo[]>("list_remote_branches", {
          projectPath,
        });
      } else {
        // All: merge local + remote
        const [local, remote] = await Promise.all([
          invoke<BranchInfo[]>("list_branches", { projectPath }),
          invoke<BranchInfo[]>("list_remote_branches", { projectPath }),
        ]);
        // Deduplicate by name
        const seen = new Set<string>();
        const merged: BranchInfo[] = [];
        for (const b of local) {
          seen.add(b.name);
          merged.push(b);
        }
        for (const b of remote) {
          if (!seen.has(b.name)) {
            merged.push(b);
          }
        }
        branches = merged;
      }
    } catch (err) {
      console.error("Failed to fetch branches:", err);
      branches = [];
    }
    loading = false;
  }

  function divergenceIndicator(branch: BranchInfo): string {
    switch (branch.divergence_status) {
      case "Ahead":
        return `+${branch.ahead}`;
      case "Behind":
        return `-${branch.behind}`;
      case "Diverged":
        return `+${branch.ahead} -${branch.behind}`;
      default:
        return "";
    }
  }

  function divergenceClass(status: string): string {
    switch (status) {
      case "Ahead":
        return "ahead";
      case "Behind":
        return "behind";
      case "Diverged":
        return "diverged";
      default:
        return "";
    }
  }
</script>

<aside class="sidebar">
  <div class="filter-bar">
    {#each filters as filter}
      <button
        class="filter-btn"
        class:active={activeFilter === filter}
        onclick={() => (activeFilter = filter)}
      >
        {filter}
      </button>
    {/each}
  </div>
  <div class="search-bar">
    <input
      type="text"
      class="search-input"
      placeholder="Filter branches..."
      bind:value={searchQuery}
    />
  </div>
  <div class="branch-list">
    {#if loading}
      <div class="loading-indicator">Loading...</div>
    {:else if filteredBranches.length === 0}
      <div class="empty-indicator">No branches found.</div>
    {:else}
      {#each filteredBranches as branch}
        <button
          class="branch-item"
          class:active={branch.is_current}
          onclick={() => onBranchSelect(branch)}
        >
          <span class="branch-icon">{branch.is_current ? "*" : " "}</span>
          <span class="branch-name">{branch.name}</span>
          {#if divergenceIndicator(branch)}
            <span
              class="divergence {divergenceClass(branch.divergence_status)}"
            >
              {divergenceIndicator(branch)}
            </span>
          {/if}
        </button>
      {/each}
    {/if}
  </div>
</aside>

<style>
  .sidebar {
    width: var(--sidebar-width);
    min-width: var(--sidebar-width);
    background-color: var(--bg-secondary);
    border-right: 1px solid var(--border-color);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .filter-bar {
    display: flex;
    padding: 8px;
    gap: 4px;
    border-bottom: 1px solid var(--border-color);
  }

  .filter-btn {
    flex: 1;
    background: none;
    border: 1px solid var(--border-color);
    color: var(--text-secondary);
    padding: 4px 8px;
    font-size: 11px;
    cursor: pointer;
    border-radius: 4px;
    font-family: inherit;
  }

  .filter-btn.active {
    background-color: var(--accent);
    color: var(--bg-primary);
    border-color: var(--accent);
  }

  .search-bar {
    padding: 6px 8px;
    border-bottom: 1px solid var(--border-color);
  }

  .search-input {
    width: 100%;
    padding: 5px 8px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 4px;
    color: var(--text-primary);
    font-size: 12px;
    font-family: inherit;
    outline: none;
  }

  .search-input:focus {
    border-color: var(--accent);
  }

  .search-input::placeholder {
    color: var(--text-muted);
  }

  .branch-list {
    flex: 1;
    overflow-y: auto;
    padding: 4px 0;
  }

  .loading-indicator,
  .empty-indicator {
    padding: 16px;
    text-align: center;
    color: var(--text-muted);
    font-size: 12px;
  }

  .branch-item {
    display: flex;
    align-items: center;
    padding: 6px 12px;
    cursor: pointer;
    gap: 8px;
    width: 100%;
    background: none;
    border: none;
    color: var(--text-primary);
    font-family: inherit;
    text-align: left;
  }

  .branch-item:hover {
    background-color: var(--bg-hover);
  }

  .branch-item.active {
    background-color: var(--bg-surface);
    color: var(--accent);
  }

  .branch-icon {
    color: var(--text-muted);
    font-size: 12px;
    font-family: monospace;
    width: 12px;
    flex-shrink: 0;
  }

  .branch-name {
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
  }

  .divergence {
    font-size: 10px;
    font-family: monospace;
    padding: 1px 4px;
    border-radius: 3px;
    flex-shrink: 0;
  }

  .divergence.ahead {
    color: var(--green);
  }

  .divergence.behind {
    color: var(--yellow);
  }

  .divergence.diverged {
    color: var(--red);
  }
</style>
