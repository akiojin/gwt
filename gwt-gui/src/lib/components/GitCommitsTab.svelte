<script lang="ts">
  import type { CommitEntry } from "../types";

  let {
    projectPath,
    branch,
    baseBranch,
  }: {
    projectPath: string;
    branch: string;
    baseBranch: string;
  } = $props();

  let commits: CommitEntry[] = $state([]);
  let loading: boolean = $state(true);
  let loadingMore: boolean = $state(false);
  let error: string | null = $state(null);
  let hasMore: boolean = $state(false);

  const PAGE_SIZE = 20;

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  function relativeTime(timestamp: number): string {
    const now = Date.now();
    const diff = now - timestamp * 1000;
    const seconds = Math.floor(diff / 1000);
    const minutes = Math.floor(seconds / 60);
    const hours = Math.floor(minutes / 60);
    const days = Math.floor(hours / 24);
    const weeks = Math.floor(days / 7);
    const months = Math.floor(days / 30);

    if (seconds < 60) return "just now";
    if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
    if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
    if (days < 7) return `${days} day${days === 1 ? "" : "s"} ago`;
    if (weeks < 5) return `${weeks} week${weeks === 1 ? "" : "s"} ago`;
    return `${months} month${months === 1 ? "" : "s"} ago`;
  }

  function absoluteTime(timestamp: number): string {
    const d = new Date(timestamp * 1000);
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, "0");
    const day = String(d.getDate()).padStart(2, "0");
    const h = String(d.getHours()).padStart(2, "0");
    const min = String(d.getMinutes()).padStart(2, "0");
    return `${y}-${m}-${day} ${h}:${min}`;
  }

  async function load() {
    loading = true;
    error = null;
    commits = [];
    hasMore = false;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const result = await invoke<CommitEntry[]>("get_branch_commits", {
        projectPath,
        branch,
        baseBranch,
        offset: 0,
        limit: PAGE_SIZE,
      });
      commits = result ?? [];
      hasMore = commits.length >= PAGE_SIZE;
    } catch (err) {
      error = toErrorMessage(err);
    } finally {
      loading = false;
    }
  }

  async function loadMore() {
    if (loadingMore) return;
    loadingMore = true;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const result = await invoke<CommitEntry[]>("get_branch_commits", {
        projectPath,
        branch,
        baseBranch,
        offset: commits.length,
        limit: PAGE_SIZE,
      });
      const more = result ?? [];
      commits = [...commits, ...more];
      hasMore = more.length >= PAGE_SIZE;
    } catch (err) {
      error = toErrorMessage(err);
    } finally {
      loadingMore = false;
    }
  }

  $effect(() => {
    void projectPath;
    void branch;
    void baseBranch;
    load();
  });
</script>

<div class="commits-tab">
  {#if loading}
    <div class="commits-loading">Loading...</div>
  {:else if error}
    <div class="commits-error">{error}</div>
  {:else if commits.length === 0}
    <div class="commits-empty">No commits</div>
  {:else}
    <div class="commits-list">
      {#each commits as commit (commit.sha)}
        <div class="commit-row">
          <span class="commit-sha mono">{commit.sha.slice(0, 7)}</span>
          <span class="commit-message">{commit.message}</span>
          <span class="commit-time" title={absoluteTime(commit.timestamp)}>
            {relativeTime(commit.timestamp)}
          </span>
        </div>
      {/each}
    </div>
    {#if hasMore}
      <button class="show-more-btn" disabled={loadingMore} onclick={loadMore}>
        {loadingMore ? "Loading..." : "Show more"}
      </button>
    {/if}
  {/if}
</div>

<style>
  .commits-tab {
    padding: 8px 0;
  }

  .commits-loading,
  .commits-empty {
    font-size: 12px;
    color: var(--text-muted);
    padding: 8px 0;
  }

  .commits-error {
    padding: 10px 12px;
    border: 1px solid rgba(255, 0, 0, 0.35);
    background: rgba(255, 0, 0, 0.08);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: 12px;
  }

  .commits-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .commit-row {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 5px 8px;
    border-radius: 4px;
    font-size: 12px;
  }

  .commit-row:hover {
    background: var(--bg-hover);
  }

  .commit-sha {
    color: var(--accent);
    flex-shrink: 0;
  }

  .commit-message {
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1;
  }

  .commit-time {
    color: var(--text-muted);
    white-space: nowrap;
    flex-shrink: 0;
    font-size: 11px;
    cursor: default;
  }

  .mono {
    font-family: monospace;
  }

  .show-more-btn {
    margin-top: 8px;
    padding: 6px 16px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-secondary);
    font-size: 12px;
    cursor: pointer;
    font-family: inherit;
  }

  .show-more-btn:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .show-more-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
</style>
