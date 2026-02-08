<script lang="ts">
  type FilterType = "Local" | "Remote" | "All";
  let activeFilter: FilterType = $state("Local");

  // Placeholder branches for layout verification
  const placeholderBranches = [
    { name: "main", active: true },
    { name: "develop", active: false },
    { name: "feature/multi-terminal", active: false },
    { name: "feature/auth", active: false },
    { name: "fix/login-bug", active: false },
  ];

  const filters: FilterType[] = ["Local", "Remote", "All"];
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
  <div class="branch-list">
    {#each placeholderBranches as branch}
      <div class="branch-item" class:active={branch.active}>
        <span class="branch-icon">*</span>
        <span class="branch-name">{branch.name}</span>
      </div>
    {/each}
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

  .branch-list {
    flex: 1;
    overflow-y: auto;
    padding: 4px 0;
  }

  .branch-item {
    display: flex;
    align-items: center;
    padding: 6px 12px;
    cursor: pointer;
    gap: 8px;
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
  }

  .branch-name {
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
