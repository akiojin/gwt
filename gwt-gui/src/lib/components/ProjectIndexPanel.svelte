<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { ProjectIndexSearchResult, GitHubIssueSearchResult } from "../types";
  import { openExternalUrl } from "../openExternalUrl";

  let { projectPath }: { projectPath: string } = $props();

  // Tab state
  let activeTab = $state<"files" | "issues">("files");

  // Files tab state
  let query = $state("");
  let results = $state<ProjectIndexSearchResult[]>([]);
  let searching = $state(false);
  let hasSearched = $state(false);
  let lastSearchedQuery = $state("");
  let error = $state<string | null>(null);
  let statusError = $state<string | null>(null);
  let indexStatus = $state<{
    indexed: boolean;
    totalFiles: number;
    dbSizeBytes: number;
  } | null>(null);
  let statusLoading = $state(true);

  // Issues tab state
  let issueQuery = $state("");
  let issueResults = $state<GitHubIssueSearchResult[]>([]);
  let issueSearching = $state(false);
  let issueError = $state<string | null>(null);
  let issueIndexing = $state(false);
  let issueIndexStatus = $state<string | null>(null);

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
    if (!q) {
      hasSearched = false;
      lastSearchedQuery = "";
      results = [];
      return;
    }

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
      hasSearched = true;
      lastSearchedQuery = q;
      searching = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      handleSearch();
    }
  }

  async function handleIndexIssues() {
    issueIndexing = true;
    issueIndexStatus = null;
    issueError = null;
    try {
      const result: { issuesIndexed: number; durationMs: number } = await invoke(
        "index_github_issues_cmd",
        { projectRoot: projectPath },
      );
      issueIndexStatus = `${result.issuesIndexed} issues indexed (${result.durationMs}ms)`;
    } catch (e) {
      issueError = String(e);
    } finally {
      issueIndexing = false;
    }
  }

  async function handleSearchIssues() {
    const q = issueQuery.trim();
    if (!q) return;

    issueSearching = true;
    issueError = null;
    try {
      issueResults = await invoke("search_github_issues_cmd", {
        projectRoot: projectPath,
        query: q,
        nResults: 20,
      });
    } catch (e) {
      issueError = String(e);
      issueResults = [];
    } finally {
      issueSearching = false;
    }
  }

  function handleIssueKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      handleSearchIssues();
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
  <div class="tabs">
    <button
      class="tab-btn"
      class:active={activeTab === "files"}
      onclick={() => (activeTab = "files")}
    >
      Files
    </button>
    <button
      class="tab-btn"
      class:active={activeTab === "issues"}
      onclick={() => (activeTab = "issues")}
    >
      Issues
    </button>
  </div>

  {#if activeTab === "files"}
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
      {:else if !searching && !error && hasSearched && lastSearchedQuery === query.trim()}
        <div class="no-results">No results found</div>
      {/if}
    </div>
  {:else}
    <div class="issues-toolbar">
      <button
        onclick={handleIndexIssues}
        disabled={issueIndexing}
        class="update-index-btn"
      >
        {issueIndexing ? "Updating..." : "Update Index"}
      </button>
      {#if issueIndexStatus}
        <span class="status-ok">{issueIndexStatus}</span>
      {/if}
    </div>

    <div class="search-bar">
      <input
        type="text"
        placeholder="Search GitHub Issues..."
        bind:value={issueQuery}
        onkeydown={handleIssueKeydown}
        class="search-input"
      />
      <button
        onclick={handleSearchIssues}
        disabled={issueSearching || !issueQuery.trim()}
        class="search-btn"
      >
        {issueSearching ? "Searching..." : "Search"}
      </button>
    </div>

    {#if issueError}
      <div class="error-message">{issueError}</div>
    {/if}

    <div class="results">
      {#if issueResults.length > 0}
        {#each issueResults as item}
          <button
            class="result-item result-item-clickable"
            onclick={() => openExternalUrl(item.url)}
          >
            <div class="result-header">
              <span class="issue-number">#{item.number}</span>
              <span class="issue-title">{item.title}</span>
              {#if item.distance != null}
                <span class="result-score">{formatDistance(item.distance)}</span>
              {/if}
            </div>
            <div class="issue-meta">
              <span class="issue-state" class:state-open={item.state === "open"} class:state-closed={item.state === "closed"}>{item.state}</span>
              {#each item.labels as label}
                <span class="label-badge">{label}</span>
              {/each}
            </div>
          </button>
        {/each}
      {:else if !issueSearching && !issueError && issueQuery.trim()}
        <div class="no-results">No results found</div>
      {/if}
    </div>
  {/if}


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

  .tabs {
    display: flex;
    gap: 2px;
    border-bottom: 1px solid var(--border-color, #444);
    padding-bottom: 8px;
  }

  .tab-btn {
    padding: 4px 12px;
    border: 1px solid transparent;
    border-radius: 4px 4px 0 0;
    background: none;
    color: var(--text-secondary, #888);
    cursor: pointer;
    font-size: 12px;
  }

  .tab-btn:hover {
    color: var(--text-primary, #ccc);
    background: var(--bg-hover, #2a2a2a);
  }

  .tab-btn.active {
    color: var(--text-primary, #ccc);
    border-color: var(--border-color, #444);
    border-bottom-color: var(--bg-primary, #1a1a1a);
    background: var(--bg-primary, #1a1a1a);
  }

  .issues-toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .update-index-btn {
    padding: 4px 10px;
    border: 1px solid var(--border-color, #444);
    border-radius: 4px;
    background: var(--bg-secondary, #1e1e1e);
    color: var(--text-primary, #ccc);
    cursor: pointer;
    font-size: 12px;
    white-space: nowrap;
  }

  .update-index-btn:hover:not(:disabled) {
    background: var(--bg-hover, #2a2a2a);
  }

  .update-index-btn:disabled {
    opacity: 0.5;
    cursor: default;
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
    font-size: 11px;
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

  .result-item-clickable {
    cursor: pointer;
    background: none;
    border: none;
    text-align: left;
    width: 100%;
    color: inherit;
  }

  .result-item:hover,
  .result-item-clickable:hover {
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

  .issue-number {
    font-family: var(--font-mono, monospace);
    font-size: 11px;
    color: var(--text-secondary, #888);
    flex-shrink: 0;
  }

  .issue-title {
    font-size: 12px;
    color: var(--accent-color, #58a6ff);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .issue-meta {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-top: 3px;
    flex-wrap: wrap;
  }

  .issue-state {
    font-size: 10px;
    padding: 1px 5px;
    border-radius: 10px;
    background: var(--bg-tertiary, #333);
    color: var(--text-secondary, #888);
  }

  .issue-state.state-open {
    color: #3fb950;
    background: rgba(63, 185, 80, 0.1);
  }

  .issue-state.state-closed {
    color: var(--text-secondary, #888);
  }

  .label-badge {
    font-size: 10px;
    padding: 1px 5px;
    border-radius: 10px;
    background: rgba(88, 166, 255, 0.15);
    color: var(--accent-color, #58a6ff);
  }

  .no-results {
    text-align: center;
    color: var(--text-secondary, #888);
    font-size: 13px;
    padding: 24px 0;
  }
</style>
