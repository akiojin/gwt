<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { ProjectIndexSearchResult } from "../types";

  let { projectPath }: { projectPath: string } = $props();

  let query = $state("");
  let results = $state<ProjectIndexSearchResult[]>([]);
  let searching = $state(false);
  let error = $state<string | null>(null);
  let statusError = $state<string | null>(null);
  let indexStatus = $state<{
    indexed: boolean;
    totalFiles: number;
    dbSizeBytes: number;
  } | null>(null);
  let statusLoading = $state(true);

  async function loadStatus() {
    statusLoading = true;
    statusError = null;
    try {
      indexStatus = await invoke("get_index_status_cmd", {
        projectRoot: projectPath,
      });
    } catch (e) {
      indexStatus = null;
      statusError = String(e);
    } finally {
      statusLoading = false;
    }
  }

  async function handleSearch() {
    const q = query.trim();
    if (!q) return;

    searching = true;
    error = null;
    try {
      results = await invoke("search_project_index_cmd", {
        projectRoot: projectPath,
        query: q,
        nResults: 20,
      });
    } catch (e) {
      error = String(e);
      results = [];
    } finally {
      searching = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      handleSearch();
    }
  }

  function formatDistance(d: number | null | undefined): string {
    if (d == null) return "";
    const clampedDistance = Math.max(0, Math.min(1, d));
    const similarityPercent = Math.round((1 - clampedDistance) * 100);
    return `${similarityPercent}%`;
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  $effect(() => {
    if (projectPath) {
      loadStatus();
    }
  });
</script>

<div class="project-index-panel">
  <div class="search-bar">
    <input
      type="text"
      placeholder="Search project files..."
      bind:value={query}
      onkeydown={handleKeydown}
      class="search-input"
    />
    <button onclick={handleSearch} disabled={searching || !query.trim()} class="search-btn">
      {searching ? "Searching..." : "Search"}
    </button>
  </div>

  {#if indexStatus && !statusLoading}
    <div class="status-bar">
      {#if indexStatus.indexed}
        <span class="status-ok">{indexStatus.totalFiles} files indexed</span>
        <span class="status-size">({formatSize(indexStatus.dbSizeBytes)})</span>
      {:else}
        <span class="status-warn">Index not available</span>
      {/if}
    </div>
  {/if}

  {#if error}
    <div class="error-message">{error}</div>
  {/if}

  {#if statusError}
    <div class="error-message">Failed to load index status: {statusError}</div>
  {/if}

  <div class="results">
    {#if results.length > 0}
      {#each results as item}
        <div class="result-item">
          <div class="result-header">
            <span class="result-path">{item.path}</span>
            {#if item.distance != null}
              <span class="result-score">{formatDistance(item.distance)}</span>
            {/if}
          </div>
          {#if item.description}
            <div class="result-desc">{item.description}</div>
          {/if}
        </div>
      {/each}
    {:else if !searching && !error && query.trim()}
      <div class="no-results">No results found</div>
    {/if}
  </div>
</div>

<style>
  .project-index-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 12px;
    gap: 8px;
    overflow: hidden;
  }

  .search-bar {
    display: flex;
    gap: 8px;
  }

  .search-input {
    flex: 1;
    padding: 6px 10px;
    border: 1px solid var(--border-color, #444);
    border-radius: 4px;
    background: var(--bg-secondary, #1e1e1e);
    color: var(--text-primary, #ccc);
    font-size: 13px;
    outline: none;
  }

  .search-input:focus {
    border-color: var(--accent-color, #58a6ff);
  }

  .search-btn {
    padding: 6px 14px;
    border: 1px solid var(--border-color, #444);
    border-radius: 4px;
    background: var(--bg-secondary, #1e1e1e);
    color: var(--text-primary, #ccc);
    cursor: pointer;
    font-size: 13px;
    white-space: nowrap;
  }

  .search-btn:hover:not(:disabled) {
    background: var(--bg-hover, #2a2a2a);
  }

  .search-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .status-bar {
    font-size: 11px;
    color: var(--text-secondary, #888);
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .status-ok {
    color: var(--text-secondary, #888);
  }

  .status-size {
    color: var(--text-tertiary, #666);
  }

  .status-warn {
    color: var(--warning-color, #e5c07b);
  }

  .error-message {
    color: var(--error-color, #e06c75);
    font-size: 12px;
    padding: 6px 8px;
    border-radius: 4px;
    background: var(--error-bg, rgba(224, 108, 117, 0.1));
  }

  .results {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .result-item {
    padding: 6px 8px;
    border-radius: 4px;
    cursor: default;
  }

  .result-item:hover {
    background: var(--bg-hover, #2a2a2a);
  }

  .result-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
  }

  .result-path {
    font-family: var(--font-mono, monospace);
    font-size: 12px;
    color: var(--accent-color, #58a6ff);
    word-break: break-all;
  }

  .result-score {
    font-size: 11px;
    color: var(--text-secondary, #888);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .result-desc {
    font-size: 11px;
    color: var(--text-secondary, #888);
    margin-top: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .no-results {
    text-align: center;
    color: var(--text-secondary, #888);
    font-size: 13px;
    padding: 24px 0;
  }
</style>
