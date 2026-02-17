<script lang="ts">
  import type {
    GitHubIssueInfo,
    GitHubLabel,
    GhCliStatus,
    FetchIssuesResponse,
  } from "../types";
  import MarkdownRenderer from "./MarkdownRenderer.svelte";

  let {
    projectPath,
    onWorkOnIssue,
    onSwitchToWorktree,
  }: {
    projectPath: string;
    onWorkOnIssue: (issue: GitHubIssueInfo) => void;
    onSwitchToWorktree: (branchName: string) => void;
  } = $props();

  type ViewMode = "list" | "detail";
  type StateFilter = "open" | "closed";

  let viewMode: ViewMode = $state("list");
  let stateFilter: StateFilter = $state("open");
  let searchQuery: string = $state("");
  let labelFilter: string | null = $state(null);

  let ghCliStatus: GhCliStatus | null = $state(null as GhCliStatus | null);
  let issues: GitHubIssueInfo[] = $state([]);
  let loading: boolean = $state(true);
  let error: string | null = $state(null);
  let page: number = $state(1);
  let hasNextPage: boolean = $state(false);
  let loadingMore: boolean = $state(false);

  let selectedIssue: GitHubIssueInfo | null = $state(null);
  let detailIssue: GitHubIssueInfo | null = $state(null);
  let detailLoading: boolean = $state(false);
  let detailError: string | null = $state(null);

  let issueBranchMap: Map<number, string | null> = $state(new Map());

  let sentinelRef: HTMLDivElement | null = $state(null);

  let filteredIssues = $derived(
    (() => {
      let result = issues;

      const q = searchQuery.trim().toLowerCase();
      if (q) {
        result = result.filter((i) => i.title.toLowerCase().includes(q));
      }

      if (labelFilter) {
        result = result.filter((i) =>
          i.labels.some((l) => l.name === labelFilter)
        );
      }

      return result;
    })()
  );

  let allLabels = $derived(
    (() => {
      const labelSet = new Map<string, string>();
      for (const issue of issues) {
        for (const label of issue.labels) {
          if (!labelSet.has(label.name)) {
            labelSet.set(label.name, label.color);
          }
        }
      }
      return Array.from(labelSet.entries()).map(([name, color]) => ({
        name,
        color,
      }));
    })()
  );

  let ghCliAvailable = $derived(
    ghCliStatus !== null && ghCliStatus.available && ghCliStatus.authenticated
  );

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  function labelStyle(color: string): string {
    const hex = color.replace(/^#/, "");
    if (!/^[0-9a-fA-F]{6}$/.test(hex)) {
      return "";
    }
    const r = parseInt(hex.slice(0, 2), 16);
    const g = parseInt(hex.slice(2, 4), 16);
    const b = parseInt(hex.slice(4, 6), 16);
    // Determine contrast color
    const luma = (r * 299 + g * 587 + b * 114) / 1000;
    const textColor = luma > 128 ? "#1e1e2e" : "#cdd6f4";
    return `background-color: #${hex}; color: ${textColor};`;
  }

  function relativeTime(dateStr: string): string {
    try {
      const date = new Date(dateStr);
      const now = new Date();
      const diffMs = now.getTime() - date.getTime();
      const diffMins = Math.floor(diffMs / 60000);
      if (diffMins < 1) return "just now";
      if (diffMins < 60) return `${diffMins}m ago`;
      const diffHours = Math.floor(diffMins / 60);
      if (diffHours < 24) return `${diffHours}h ago`;
      const diffDays = Math.floor(diffHours / 24);
      if (diffDays < 30) return `${diffDays}d ago`;
      const diffMonths = Math.floor(diffDays / 30);
      return `${diffMonths}mo ago`;
    } catch {
      return dateStr;
    }
  }

  function inferBranchPrefix(labels: GitHubLabel[]): string {
    const names = labels.map((l) => l.name.toLowerCase());
    if (names.includes("bug")) return "bugfix/";
    if (names.includes("enhancement") || names.includes("feature")) return "feature/";
    if (names.includes("hotfix")) return "hotfix/";
    return "feature/";
  }

  async function checkGhCli() {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      ghCliStatus = await invoke<GhCliStatus>("check_gh_cli_status", {
        projectPath,
      });
    } catch {
      ghCliStatus = { available: false, authenticated: false };
    }
  }

  async function fetchIssues(pageNum: number, append = false) {
    if (append) {
      loadingMore = true;
    } else {
      loading = true;
    }
    error = null;

    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const resp = await invoke<FetchIssuesResponse>("fetch_github_issues", {
        projectPath,
        page: pageNum,
        perPage: 30,
        state: stateFilter,
      });

      if (append) {
        issues = [...issues, ...resp.issues];
      } else {
        issues = resp.issues;
      }
      page = pageNum;
      hasNextPage = resp.hasNextPage;

      // Check branches for loaded issues
      for (const issue of resp.issues) {
        if (!issueBranchMap.has(issue.number)) {
          void checkExistingBranch(issue.number);
        }
      }
    } catch (err) {
      error = toErrorMessage(err);
    } finally {
      loading = false;
      loadingMore = false;
    }
  }

  async function checkExistingBranch(issueNumber: number) {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const branch = await invoke<string | null>("find_existing_issue_branch", {
        projectPath,
        issueNumber,
      });
      issueBranchMap = new Map(issueBranchMap).set(issueNumber, branch ?? null);
    } catch {
      issueBranchMap = new Map(issueBranchMap).set(issueNumber, null);
    }
  }

  async function fetchIssueDetail(issue: GitHubIssueInfo) {
    detailLoading = true;
    detailError = null;

    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const detail = await invoke<GitHubIssueInfo>("fetch_github_issue_detail", {
        projectPath,
        issueNumber: issue.number,
      });
      detailIssue = detail;
    } catch (err) {
      detailError = toErrorMessage(err);
      detailIssue = issue;
    } finally {
      detailLoading = false;
    }
  }

  function openDetail(issue: GitHubIssueInfo) {
    selectedIssue = issue;
    detailIssue = issue;
    viewMode = "detail";
    void fetchIssueDetail(issue);
  }

  function backToList() {
    viewMode = "list";
    selectedIssue = null;
    detailIssue = null;
    detailError = null;
  }

  function handleStateFilterChange(filter: StateFilter) {
    if (stateFilter === filter) return;
    stateFilter = filter;
    issues = [];
    page = 1;
    hasNextPage = false;
    void fetchIssues(1);
  }

  function handleLabelFilterClick(name: string) {
    labelFilter = labelFilter === name ? null : name;
  }

  function handleRefresh() {
    issues = [];
    page = 1;
    hasNextPage = false;
    void fetchIssues(1);
  }

  async function handleOpenInGitHub(url: string) {
    try {
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(url);
    } catch {
      window.open(url, "_blank");
    }
  }

  function handleWorkOnIssue(issue: GitHubIssueInfo) {
    onWorkOnIssue(issue);
  }

  function handleSwitchToWorktree(branchName: string) {
    onSwitchToWorktree(branchName);
  }

  function isSpecIssue(issue: GitHubIssueInfo): boolean {
    return issue.labels.some(
      (l) => l.name.toLowerCase() === "spec" || l.name.toLowerCase().startsWith("spec:")
    );
  }

  // Setup IntersectionObserver for infinite scroll
  $effect(() => {
    if (!sentinelRef) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0]?.isIntersecting && hasNextPage && !loading && !loadingMore) {
          void fetchIssues(page + 1, true);
        }
      },
      { threshold: 0.1 }
    );
    observer.observe(sentinelRef);
    return () => observer.disconnect();
  });

  // Initial data load
  $effect(() => {
    void projectPath;
    void checkGhCli().then(() => {
      if (ghCliAvailable) {
        void fetchIssues(1);
      } else {
        loading = false;
      }
    });
  });
</script>

<section class="issue-list-panel">
  {#if viewMode === "list"}
    <!-- Header -->
    <header class="ilp-header">
      <div class="ilp-header-top">
        <h2 class="ilp-title">Issues</h2>
        <div class="ilp-header-actions">
          <div class="ilp-state-toggle">
            <button
              class="state-btn"
              class:active={stateFilter === "open"}
              onclick={() => handleStateFilterChange("open")}
            >
              Open
            </button>
            <button
              class="state-btn"
              class:active={stateFilter === "closed"}
              onclick={() => handleStateFilterChange("closed")}
            >
              Closed
            </button>
          </div>
          <button class="ilp-refresh-btn" onclick={handleRefresh} title="Refresh">
            &#x21BB;
          </button>
        </div>
      </div>

      <div class="ilp-search-row">
        <input
          type="text"
          class="ilp-search-input"
          placeholder="Search issues..."
          bind:value={searchQuery}
        />
      </div>

      {#if allLabels.length > 0}
        <div class="ilp-label-filters">
          {#each allLabels as label (label.name)}
            <button
              class="ilp-label-chip"
              class:active={labelFilter === label.name}
              style={labelStyle(label.color)}
              onclick={() => handleLabelFilterClick(label.name)}
            >
              {label.name}
            </button>
          {/each}
          {#if labelFilter}
            <button
              class="ilp-label-clear"
              onclick={() => (labelFilter = null)}
            >
              Clear
            </button>
          {/if}
        </div>
      {/if}
    </header>

    <!-- Error / not available -->
    {#if !ghCliAvailable && !loading}
      <div class="ilp-error">
        GitHub CLI (gh) is not available. Install and authenticate gh to view issues.
      </div>
    {:else if error}
      <div class="ilp-error">{error}</div>
    {:else if loading && issues.length === 0}
      <div class="ilp-loading">Loading issues...</div>
    {:else if filteredIssues.length === 0}
      <div class="ilp-empty">No issues found.</div>
    {:else}
      <!-- Issue List -->
      <div class="ilp-list">
        {#each filteredIssues as issue (issue.number)}
          {@const existingBranch = issueBranchMap.get(issue.number)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="ilp-issue-row"
            onclick={() => openDetail(issue)}
          >
            <div class="ilp-issue-main">
              <span class="ilp-issue-number">#{issue.number}</span>
              <span class="ilp-issue-title">{issue.title}</span>
            </div>
            <div class="ilp-issue-meta">
              {#if issue.labels.length > 0}
                <span class="ilp-issue-labels">
                  {#each issue.labels as lbl (lbl.name)}
                    <span
                      class="ilp-issue-label"
                      style={labelStyle(lbl.color)}
                    >
                      {lbl.name}
                    </span>
                  {/each}
                </span>
              {/if}
              {#if issue.assignees.length > 0}
                <span class="ilp-issue-assignees">
                  {#each issue.assignees as assignee (assignee.login)}
                    <img
                      class="ilp-avatar"
                      src={assignee.avatarUrl}
                      alt={assignee.login}
                      title={assignee.login}
                      width="20"
                      height="20"
                    />
                  {/each}
                </span>
              {/if}
              {#if issue.milestone}
                <span class="ilp-issue-milestone" title={issue.milestone.title}>
                  {issue.milestone.title}
                </span>
              {/if}
              {#if issue.commentsCount > 0}
                <span class="ilp-issue-comments" title="{issue.commentsCount} comments">
                  {issue.commentsCount}
                </span>
              {/if}
              <span class="ilp-issue-updated" title={issue.updatedAt}>
                {relativeTime(issue.updatedAt)}
              </span>
              {#if existingBranch}
                <button
                  class="ilp-worktree-btn"
                  title="Switch to worktree: {existingBranch}"
                  onclick={(e) => { e.stopPropagation(); handleSwitchToWorktree(existingBranch); }}
                >
                  WT
                </button>
              {/if}
            </div>
          </div>
        {/each}

        <!-- Infinite scroll sentinel -->
        {#if hasNextPage}
          <div class="ilp-sentinel" bind:this={sentinelRef}></div>
        {/if}

        {#if loadingMore}
          <div class="ilp-loading-more">Loading more...</div>
        {/if}
      </div>
    {/if}
  {:else}
    <!-- Detail View -->
    <div class="ilp-detail">
      <header class="ilp-detail-header">
        <button class="ilp-back-btn" onclick={backToList}>
          &#x2190; Back
        </button>
      </header>

      {#if detailLoading}
        <div class="ilp-loading">Loading issue details...</div>
      {:else if detailIssue}
        <div class="ilp-detail-content">
          <div class="ilp-detail-title-row">
            <h2 class="ilp-detail-title">
              <span class="ilp-issue-number">#{detailIssue.number}</span>
              {detailIssue.title}
            </h2>
            <span class="ilp-detail-state" class:closed={detailIssue.state === "closed"}>
              {detailIssue.state}
            </span>
          </div>

          <div class="ilp-detail-meta">
            {#if detailIssue.labels.length > 0}
              <span class="ilp-issue-labels">
                {#each detailIssue.labels as lbl (lbl.name)}
                  <span
                    class="ilp-issue-label"
                    style={labelStyle(lbl.color)}
                  >
                    {lbl.name}
                  </span>
                {/each}
              </span>
            {/if}
            {#if detailIssue.assignees.length > 0}
              <span class="ilp-issue-assignees">
                {#each detailIssue.assignees as assignee (assignee.login)}
                  <img
                    class="ilp-avatar"
                    src={assignee.avatarUrl}
                    alt={assignee.login}
                    title={assignee.login}
                    width="24"
                    height="24"
                  />
                {/each}
              </span>
            {/if}
            {#if detailIssue.milestone}
              <span class="ilp-detail-milestone">
                {detailIssue.milestone.title}
              </span>
            {/if}
            <span class="ilp-detail-updated">
              Updated: {relativeTime(detailIssue.updatedAt)}
            </span>
            {#if detailIssue.commentsCount > 0}
              <span class="ilp-detail-comments">
                {detailIssue.commentsCount} comments
              </span>
            {/if}
          </div>

          {#if detailError}
            <div class="ilp-error">{detailError}</div>
          {/if}

          <!-- Body -->
          <div class="ilp-detail-body">
            {#if detailIssue.body}
              <MarkdownRenderer text={detailIssue.body} />
            {:else}
              <p class="ilp-empty">No description provided.</p>
            {/if}
          </div>

          <!-- Actions -->
          <div class="ilp-detail-actions">
            {#if issueBranchMap.get(detailIssue.number)}
              <button
                class="ilp-action-btn ilp-action-switch"
                onclick={() => handleSwitchToWorktree(issueBranchMap.get(detailIssue!.number)!)}
              >
                Switch to Worktree
              </button>
            {:else}
              <button
                class="ilp-action-btn ilp-action-work"
                onclick={() => handleWorkOnIssue(detailIssue!)}
              >
                Work on this
              </button>
            {/if}
            <button
              class="ilp-action-btn ilp-action-github"
              onclick={() => handleOpenInGitHub(detailIssue!.htmlUrl)}
            >
              Open in GitHub
            </button>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</section>

<style>
  .issue-list-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .ilp-header {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--border-color);
    margin-bottom: 12px;
  }

  .ilp-header-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .ilp-title {
    font-size: var(--ui-font-xl);
    font-weight: 800;
    color: var(--text-primary);
    margin: 0;
  }

  .ilp-header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .ilp-state-toggle {
    display: flex;
    gap: 4px;
  }

  .state-btn {
    padding: 4px 12px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-secondary);
    font-size: var(--ui-font-sm);
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s, background 0.15s;
  }

  .state-btn:hover {
    border-color: var(--accent);
  }

  .state-btn.active {
    border-color: var(--accent);
    background: var(--bg-surface);
    color: var(--text-primary);
  }

  .ilp-refresh-btn {
    padding: 4px 8px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-secondary);
    font-size: var(--ui-font-lg);
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s;
  }

  .ilp-refresh-btn:hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .ilp-search-row {
    display: flex;
  }

  .ilp-search-input {
    flex: 1;
    padding: 6px 10px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-md);
    font-family: inherit;
    outline: none;
  }

  .ilp-search-input:focus {
    border-color: var(--accent);
  }

  .ilp-label-filters {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    align-items: center;
  }

  .ilp-label-chip {
    padding: 2px 8px;
    border-radius: 12px;
    font-size: var(--ui-font-xs, 11px);
    font-weight: 600;
    cursor: pointer;
    border: 2px solid transparent;
    font-family: inherit;
    transition: border-color 0.15s, opacity 0.15s;
  }

  .ilp-label-chip:hover {
    opacity: 0.85;
  }

  .ilp-label-chip.active {
    border-color: var(--accent);
  }

  .ilp-label-clear {
    padding: 2px 8px;
    border-radius: 6px;
    font-size: var(--ui-font-xs, 11px);
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    color: var(--text-muted);
    cursor: pointer;
    font-family: inherit;
  }

  .ilp-error {
    padding: 12px;
    border: 1px solid rgba(255, 0, 0, 0.3);
    background: rgba(255, 0, 0, 0.06);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: var(--ui-font-md);
  }

  .ilp-loading,
  .ilp-empty {
    padding: 24px;
    text-align: center;
    color: var(--text-muted);
    font-size: var(--ui-font-md);
  }

  .ilp-list {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }

  .ilp-issue-row {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border-color);
    cursor: pointer;
    transition: background 0.15s;
  }

  .ilp-issue-row:hover {
    background: var(--bg-surface);
  }

  .ilp-issue-main {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  .ilp-issue-number {
    font-family: monospace;
    font-weight: 600;
    color: var(--text-muted);
    flex: 0 0 auto;
  }

  .ilp-issue-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-primary);
    font-weight: 500;
  }

  .ilp-issue-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .ilp-issue-labels {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }

  .ilp-issue-label {
    font-size: var(--ui-font-xs, 11px);
    padding: 1px 6px;
    border-radius: 10px;
    font-weight: 600;
    white-space: nowrap;
  }

  .ilp-issue-assignees {
    display: flex;
    gap: 2px;
  }

  .ilp-avatar {
    border-radius: 50%;
    border: 1px solid var(--border-color);
  }

  .ilp-issue-milestone {
    font-size: var(--ui-font-xs, 11px);
    color: var(--text-muted);
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
  }

  .ilp-issue-comments {
    font-size: var(--ui-font-xs, 11px);
    color: var(--text-muted);
  }

  .ilp-issue-updated {
    font-size: var(--ui-font-xs, 11px);
    color: var(--text-muted);
    margin-left: auto;
  }

  .ilp-worktree-btn {
    padding: 2px 6px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 4px;
    color: var(--text-secondary);
    font-size: var(--ui-font-xs, 11px);
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
  }

  .ilp-worktree-btn:hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .ilp-sentinel {
    height: 1px;
    flex: 0 0 1px;
  }

  .ilp-loading-more {
    padding: 12px;
    text-align: center;
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
  }

  /* Detail view */
  .ilp-detail {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow-y: auto;
  }

  .ilp-detail-header {
    display: flex;
    align-items: center;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--border-color);
    margin-bottom: 12px;
  }

  .ilp-back-btn {
    padding: 6px 12px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-secondary);
    font-size: var(--ui-font-md);
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s;
  }

  .ilp-back-btn:hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .ilp-detail-content {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .ilp-detail-title-row {
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }

  .ilp-detail-title {
    flex: 1;
    font-size: var(--ui-font-xl);
    font-weight: 800;
    color: var(--text-primary);
    margin: 0;
    line-height: 1.3;
  }

  .ilp-detail-state {
    flex: 0 0 auto;
    padding: 3px 10px;
    border-radius: 16px;
    font-size: var(--ui-font-sm);
    font-weight: 700;
    text-transform: capitalize;
    background: rgba(46, 160, 67, 0.2);
    color: #3fb950;
    border: 1px solid rgba(46, 160, 67, 0.3);
  }

  .ilp-detail-state.closed {
    background: rgba(139, 92, 246, 0.2);
    color: #a78bfa;
    border: 1px solid rgba(139, 92, 246, 0.3);
  }

  .ilp-detail-meta {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
  }

  .ilp-detail-milestone {
    padding: 1px 8px;
    border-radius: 4px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    font-size: var(--ui-font-xs, 11px);
  }

  .ilp-detail-updated,
  .ilp-detail-comments {
    font-size: var(--ui-font-xs, 11px);
    color: var(--text-muted);
  }

  .ilp-detail-body {
    border: 1px solid var(--border-color);
    border-radius: 10px;
    background: var(--bg-secondary);
    padding: 16px;
  }

  .ilp-detail-actions {
    display: flex;
    gap: 8px;
    padding-top: 8px;
  }

  .ilp-action-btn {
    padding: 8px 16px;
    border: none;
    border-radius: 6px;
    font-size: var(--ui-font-md);
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    transition: background 0.15s;
  }

  .ilp-action-work {
    background: var(--accent);
    color: var(--bg-primary);
  }

  .ilp-action-work:hover {
    background: var(--accent-hover);
  }

  .ilp-action-switch {
    background: var(--green, #a6e3a1);
    color: var(--bg-primary);
  }

  .ilp-action-switch:hover {
    opacity: 0.9;
  }

  .ilp-action-github {
    background: var(--bg-surface);
    color: var(--text-secondary);
    border: 1px solid var(--border-color);
  }

  .ilp-action-github:hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }
</style>
