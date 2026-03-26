<script lang="ts">
  import {
    ISSUE_BRANCH_LOOKUP_UNKNOWN,
    type GitHubIssueInfo,
    type GitHubIssueSearchResult,
    type GitHubLabel,
    type GhCliStatus,
    type FetchIssuesResponse,
    type IssueBranchLookupState,
    type IssueBranchMatch,
  } from "../types";
  import { invoke } from "$lib/tauriInvoke";
  import { issueMatchesSearchQuery } from "$lib/issueSearch";
  import { openExternalUrl } from "../openExternalUrl";
  import MarkdownRenderer from "./MarkdownRenderer.svelte";
  import IssueSpecPanel from "./IssueSpecPanel.svelte";

  let {
    projectPath,
    onWorkOnIssue,
    onSwitchToWorktree,
    onIssueCountChange,
  }: {
    projectPath: string;
    onWorkOnIssue: (issue: GitHubIssueInfo) => void;
    onSwitchToWorktree: (branchName: string) => void;
    onIssueCountChange?: (count: number) => void;
  } = $props();

  type ViewMode = "list" | "detail";
  type StateFilter = "open" | "closed";
  type IssueCategory = "issues" | "specs";
  type SearchDisplayIssue = GitHubIssueInfo & {
    searchSource?: "browse" | "catalog" | "semantic";
    semanticDistance?: number | null;
  };

  let viewMode: ViewMode = $state("list");
  let stateFilter: StateFilter = $state("open");
  let categoryTab: IssueCategory = $state("issues");
  let searchQuery: string = $state("");
  let labelFilter: string | null = $state(null);

  let ghCliStatus: GhCliStatus | null = $state(null as GhCliStatus | null);
  let issues: GitHubIssueInfo[] = $state([]);
  let loading: boolean = $state(true);
  let error: string | null = $state(null);
  let page: number = $state(1);
  let hasNextPage: boolean = $state(false);
  let loadingMore: boolean = $state(false);
  let searchResults: SearchDisplayIssue[] = $state([]);
  let semanticSearchResults: GitHubIssueSearchResult[] = $state([]);
  let searchLoading: boolean = $state(false);
  let searchError: string | null = $state(null);
  let semanticSearchError: string | null = $state(null);
  let indexUpdateLoading: boolean = $state(false);
  let indexUpdateStatus: string | null = $state(null);
  let indexUpdateError: string | null = $state(null);

  let selectedIssue: GitHubIssueInfo | null = $state(null);
  let detailIssue: GitHubIssueInfo | null = $state(null);
  let detailLoading: boolean = $state(false);
  let detailError: string | null = $state(null);

  let issueBranchMap: Map<number, IssueBranchLookupState> = $state(new Map());
  let fetchRequestId = 0;
  let branchLookupGeneration = 0;

  let sentinelRef: HTMLDivElement | null = $state(null);
  let listRef: HTMLDivElement | null = $state(null);
  let searchRequestId = 0;

  let filteredIssues = $derived(
    (() => {
      let result = issues;

      const q = searchQuery.trim();
      if (q) {
        result = result.filter((i) =>
          issueMatchesSearchQuery(
            {
              number: i.number,
              title: i.title,
              labels: i.labels.map((label) => label.name),
              isSpec: i.labels.some(
                (label) => label.name.toLowerCase() === "gwt-spec",
              ),
            },
            q,
          ),
        );
      }

      if (labelFilter) {
        result = result.filter((i) =>
          i.labels.some((l) => l.name === labelFilter),
        );
      }

      return result;
    })(),
  );

  let searchActive = $derived(searchQuery.trim().length > 0);

  let displayedIssues = $derived(
    (() => {
      if (!searchActive) {
        return filteredIssues.map<SearchDisplayIssue>((issue) => ({
          ...issue,
          searchSource: "browse",
          semanticDistance: null,
        }));
      }

      const merged = new Map<number, SearchDisplayIssue>();
      for (const issue of filteredIssues) {
        merged.set(issue.number, {
          ...issue,
          searchSource: "browse",
        });
      }
      for (const issue of searchResults) {
        if (!merged.has(issue.number)) {
          merged.set(issue.number, issue);
        }
      }
      for (const item of semanticSearchResults) {
        if (merged.has(item.number)) continue;
        merged.set(item.number, {
          number: item.number,
          title: item.title,
          state: item.state === "closed" ? "closed" : "open",
          updatedAt: "",
          htmlUrl: item.url,
          labels: item.labels.map((label) => ({
            name: label,
            color: label.toLowerCase() === "gwt-spec" ? "0075ca" : "6c7086",
          })),
          assignees: [],
          commentsCount: 0,
          searchSource: "semantic",
          semanticDistance: item.distance,
        });
      }

      let result = Array.from(merged.values());
      if (labelFilter) {
        result = result.filter((issue) =>
          issue.labels.some((label) => label.name === labelFilter),
        );
      }
      return result;
    })(),
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
    })(),
  );

  let ghCliAvailable = $derived(
    ghCliStatus !== null && ghCliStatus.available && ghCliStatus.authenticated,
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
      if (Number.isNaN(date.getTime())) return "";
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

  function formatDistance(distance: number | null | undefined): string {
    if (distance == null) return "";
    const clamped = Math.max(0, Math.min(1, distance));
    return `${Math.round((1 - clamped) * 100)}%`;
  }

  function inferBranchPrefix(labels: GitHubLabel[]): string {
    const names = labels.map((l) => l.name.toLowerCase());
    if (names.includes("bug")) return "bugfix/";
    if (names.includes("enhancement") || names.includes("feature"))
      return "feature/";
    if (names.includes("hotfix")) return "hotfix/";
    return "feature/";
  }

  async function checkGhCli() {
    try {
      ghCliStatus = await invoke<GhCliStatus>("check_gh_cli_status", {
        projectPath,
      });
    } catch {
      ghCliStatus = { available: false, authenticated: false };
    }
  }

  async function handleUpdateSpecIndex() {
    indexUpdateLoading = true;
    indexUpdateError = null;
    indexUpdateStatus = null;
    try {
      const result: { issuesIndexed: number; durationMs: number } =
        await invoke("index_github_issues_cmd", { projectRoot: projectPath });
      indexUpdateStatus = `${result.issuesIndexed} specs indexed (${result.durationMs}ms)`;
    } catch (err) {
      indexUpdateError = toErrorMessage(err);
    } finally {
      indexUpdateLoading = false;
    }
  }

  async function fetchIssues(
    pageNum: number,
    append = false,
    forceRefresh = false,
  ) {
    if (append && loadingMore) return;
    const requestId = ++fetchRequestId;
    const lookupGeneration = append
      ? branchLookupGeneration
      : ++branchLookupGeneration;
    if (append) {
      loadingMore = true;
    } else {
      loading = true;
      // Force branch linkage revalidation on full reload (refresh/filter switch).
      issueBranchMap = new Map();
    }
    error = null;

    try {
      const resp = await invoke<FetchIssuesResponse>("fetch_github_issues", {
        projectPath,
        page: pageNum,
        perPage: 30,
        state: stateFilter,
        category: categoryTab,
        includeBody: false,
        forceRefresh,
      });
      if (requestId !== fetchRequestId) return;

      if (append) {
        issues = [...issues, ...resp.issues];
      } else {
        issues = resp.issues;
      }
      page = pageNum;
      hasNextPage = resp.hasNextPage;

      onIssueCountChange?.(issues.length);

      // Reset loading state immediately after issue data is received
      loading = false;
      loadingMore = false;

      // Check if sentinel is still visible after loading completes
      queueMicrotask(() => checkSentinelVisibility());

      // Fire-and-forget: don't block UI on branch link search
      void loadBranchLinks(resp.issues, lookupGeneration);
    } catch (err) {
      if (requestId !== fetchRequestId) return;
      error = toErrorMessage(err);
      loading = false;
      loadingMore = false;
    }
  }

  async function loadBranchLinks(
    loadedIssues: GitHubIssueInfo[],
    lookupGeneration: number,
  ) {
    if (lookupGeneration !== branchLookupGeneration) return;
    const issueNumbers = loadedIssues.map((issue) => issue.number);
    if (issueNumbers.length === 0) return;

    const baseline = new Map(issueBranchMap);
    for (const number of issueNumbers) {
      if (!baseline.has(number)) baseline.set(number, null);
    }
    if (lookupGeneration !== branchLookupGeneration) return;
    issueBranchMap = baseline;

    try {
      const matches = await invoke<IssueBranchMatch[]>(
        "find_existing_issue_branches_bulk",
        {
          projectPath,
          issueNumbers,
        },
      );
      if (lookupGeneration !== branchLookupGeneration) return;

      const next = new Map(issueBranchMap);
      for (const match of matches) {
        next.set(match.issueNumber, match.branchName);
      }
      if (lookupGeneration !== branchLookupGeneration) return;
      issueBranchMap = next;
    } catch {
      if (lookupGeneration !== branchLookupGeneration) return;
      const next = new Map(issueBranchMap);
      for (const issueNumber of issueNumbers) {
        next.set(issueNumber, ISSUE_BRANCH_LOOKUP_UNKNOWN);
      }
      if (lookupGeneration !== branchLookupGeneration) return;
      issueBranchMap = next;
    }
  }

  async function fetchIssueDetail(issue: GitHubIssueInfo) {
    detailLoading = true;
    detailError = null;

    try {
      const detail = await invoke<GitHubIssueInfo>(
        "fetch_github_issue_detail",
        {
          projectPath,
          issueNumber: issue.number,
        },
      );
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
    labelFilter = null;
    issues = [];
    page = 1;
    hasNextPage = false;
    void fetchIssues(1);
  }

  function handleCategoryTabChange(next: IssueCategory) {
    if (categoryTab === next) return;
    categoryTab = next;
    labelFilter = null;
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
    issueBranchMap = new Map();
    page = 1;
    hasNextPage = false;
    void fetchIssues(1, false, true);
  }

  async function handleOpenInGitHub(url: string) {
    await openExternalUrl(url);
  }

  function handleWorkOnIssue(issue: GitHubIssueInfo) {
    onWorkOnIssue(issue);
  }

  function handleSwitchToWorktree(branchName: string) {
    onSwitchToWorktree(branchName);
  }

  function isSpecIssue(issue: GitHubIssueInfo): boolean {
    return issue.labels.some((l) => l.name.toLowerCase() === "gwt-spec");
  }

  function resolveExistingBranch(issueNumber: number): string | null {
    const branchState = issueBranchMap.get(issueNumber);
    if (
      typeof branchState !== "string" ||
      branchState === ISSUE_BRANCH_LOOKUP_UNKNOWN
    ) {
      return null;
    }
    return branchState;
  }

  function isBranchLookupUnknown(issueNumber: number): boolean {
    return issueBranchMap.get(issueNumber) === ISSUE_BRANCH_LOOKUP_UNKNOWN;
  }

  function checkSentinelVisibility() {
    if (searchActive) return;
    if (!sentinelRef || !listRef || !hasNextPage || loading || loadingMore)
      return;
    const containerRect = listRef.getBoundingClientRect();
    const sentinelRect = sentinelRef.getBoundingClientRect();
    if (sentinelRect.top < containerRect.bottom) {
      void fetchIssues(page + 1, true);
    }
  }

  async function runUnifiedSearch(query: string, requestId: number) {
    searchLoading = true;
    searchError = null;
    semanticSearchError = null;

    const [catalogResult, semanticResult] = await Promise.allSettled([
      invoke<GitHubIssueInfo[]>("search_github_issue_catalog", {
        projectPath,
        query,
        state: stateFilter,
        limit: 30,
      }),
      invoke<GitHubIssueSearchResult[]>("search_github_issues_cmd", {
        projectRoot: projectPath,
        query,
        nResults: 20,
      }),
    ]);

    if (requestId !== searchRequestId) return;

    if (
      catalogResult.status === "fulfilled" &&
      Array.isArray(catalogResult.value)
    ) {
      searchResults = catalogResult.value.map((issue) => ({
        ...issue,
        searchSource: "catalog",
      }));
      searchError = null;
    } else {
      searchResults = [];
      searchError =
        catalogResult.status === "rejected"
          ? toErrorMessage(catalogResult.reason)
          : null;
    }

    if (
      semanticResult.status === "fulfilled" &&
      Array.isArray(semanticResult.value)
    ) {
      semanticSearchResults = semanticResult.value;
      semanticSearchError = null;
    } else {
      semanticSearchResults = [];
      semanticSearchError =
        semanticResult.status === "rejected"
          ? toErrorMessage(semanticResult.reason)
          : null;
    }

    searchLoading = false;
  }

  // Setup IntersectionObserver for infinite scroll
  $effect(() => {
    if (!sentinelRef) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (searchActive) return;
        if (
          entries[0]?.isIntersecting &&
          hasNextPage &&
          !loading &&
          !loadingMore
        ) {
          void fetchIssues(page + 1, true);
        }
      },
      { root: listRef, threshold: 0.1 },
    );
    observer.observe(sentinelRef);
    return () => observer.disconnect();
  });

  $effect(() => {
    if (!ghCliAvailable) {
      searchResults = [];
      semanticSearchResults = [];
      searchLoading = false;
      searchError = null;
      semanticSearchError = null;
      return;
    }

    const query = searchQuery.trim();
    if (!query) {
      searchResults = [];
      semanticSearchResults = [];
      searchLoading = false;
      searchError = null;
      semanticSearchError = null;
      return;
    }

    const requestId = ++searchRequestId;
    const timer = window.setTimeout(() => {
      void runUnifiedSearch(query, requestId);
    }, 150);

    return () => window.clearTimeout(timer);
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
        <h2 class="ilp-title">
          {categoryTab === "specs" ? "Specs" : "Issues"}
        </h2>
        <div class="ilp-header-actions">
          <div class="ilp-category-toggle">
            <button
              class="state-btn"
              class:active={categoryTab === "issues"}
              disabled={searchActive}
              onclick={() => handleCategoryTabChange("issues")}
            >
              Issues
            </button>
            <button
              class="state-btn"
              class:active={categoryTab === "specs"}
              disabled={searchActive}
              onclick={() => handleCategoryTabChange("specs")}
            >
              Specs
            </button>
          </div>
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
          <button
            class="ilp-refresh-btn"
            onclick={handleRefresh}
            title="Refresh"
          >
            &#x21BB;
          </button>
        </div>
      </div>

      <div class="ilp-search-row">
        <input
          type="text"
          class="ilp-search-input"
          placeholder="Search issues, specs, or #1234..."
          bind:value={searchQuery}
        />
        <button
          class="ilp-index-btn"
          onclick={handleUpdateSpecIndex}
          disabled={indexUpdateLoading}
          title="Refresh the semantic spec search index"
        >
          {indexUpdateLoading ? "Updating..." : "Update Spec Index"}
        </button>
      </div>

      {#if searchActive}
        <div class="ilp-search-hint">
          Searching across issues and specs. Semantic matches currently come
          from the spec index.
        </div>
      {/if}

      {#if indexUpdateStatus}
        <div class="ilp-status">{indexUpdateStatus}</div>
      {/if}

      {#if indexUpdateError}
        <div class="ilp-error">{indexUpdateError}</div>
      {/if}

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
        GitHub CLI (gh) is not available. Install and authenticate gh to view
        issues.
      </div>
    {:else if error}
      <div class="ilp-error">{error}</div>
    {:else if loading && issues.length === 0 && !searchActive}
      <div class="ilp-loading">Loading issues...</div>
    {:else if searchActive && searchLoading && displayedIssues.length === 0}
      <div class="ilp-loading">Searching issues and specs...</div>
    {:else if searchActive && searchError}
      <div class="ilp-error">{searchError}</div>
    {:else if displayedIssues.length === 0}
      <div class="ilp-empty">No issues found.</div>
    {:else}
      <!-- Issue List -->
      <div class="ilp-list" bind:this={listRef}>
        {#if searchActive && semanticSearchError}
          <div class="ilp-search-note">
            Semantic spec search unavailable: {semanticSearchError}
          </div>
        {/if}

        {#each displayedIssues as issue (issue.number)}
          {@const existingBranch = resolveExistingBranch(issue.number)}
          {@const lookupUnknown = isBranchLookupUnknown(issue.number)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="ilp-issue-row" onclick={() => openDetail(issue)}>
            <div class="ilp-issue-main">
              <span class="ilp-issue-number">#{issue.number}</span>
              <span class="ilp-issue-title">{issue.title}</span>
            </div>
            <div class="ilp-issue-meta">
              {#if issue.labels.length > 0}
                <span class="ilp-issue-labels">
                  {#each issue.labels as lbl (lbl.name)}
                    <span class="ilp-issue-label" style={labelStyle(lbl.color)}>
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
                <span
                  class="ilp-issue-comments"
                  title="{issue.commentsCount} comments"
                >
                  {issue.commentsCount}
                </span>
              {/if}
              {#if issue.semanticDistance != null}
                <span class="ilp-search-score"
                  >{formatDistance(issue.semanticDistance)}</span
                >
              {/if}
              {#if issue.updatedAt}
                <span class="ilp-issue-updated" title={issue.updatedAt}>
                  {relativeTime(issue.updatedAt)}
                </span>
              {/if}
              {#if existingBranch}
                <button
                  class="ilp-worktree-btn"
                  title="Switch to worktree: {existingBranch}"
                  onclick={(e) => {
                    e.stopPropagation();
                    handleSwitchToWorktree(existingBranch);
                  }}
                >
                  WT
                </button>
              {:else if lookupUnknown}
                <button
                  class="ilp-worktree-btn"
                  disabled
                  title="Branch lookup failed. Refresh to retry."
                  onclick={(e) => {
                    e.stopPropagation();
                  }}
                >
                  WT
                </button>
              {/if}
            </div>
          </div>
        {/each}

        <!-- Infinite scroll sentinel -->
        {#if hasNextPage && !searchActive}
          <div class="ilp-sentinel" bind:this={sentinelRef}></div>
        {/if}

        {#if loadingMore && !searchActive}
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
            <span
              class="ilp-detail-state"
              class:closed={detailIssue.state === "closed"}
            >
              {detailIssue.state}
            </span>
          </div>

          <div class="ilp-detail-meta">
            {#if detailIssue.labels.length > 0}
              <span class="ilp-issue-labels">
                {#each detailIssue.labels as lbl (lbl.name)}
                  <span class="ilp-issue-label" style={labelStyle(lbl.color)}>
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

          <!-- Actions -->
          <div class="ilp-detail-actions">
            {#if resolveExistingBranch(detailIssue.number)}
              <button
                class="ilp-action-btn ilp-action-switch"
                onclick={() =>
                  handleSwitchToWorktree(
                    resolveExistingBranch(detailIssue!.number)!,
                  )}
              >
                Switch to Worktree
              </button>
            {:else if isBranchLookupUnknown(detailIssue.number)}
              <button
                class="ilp-action-btn ilp-action-switch"
                disabled
                title="Branch lookup failed. Refresh to retry."
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

          {#if detailError}
            <div class="ilp-error">{detailError}</div>
          {/if}

          <!-- Body -->
          <div class="ilp-detail-body">
            {#if isSpecIssue(detailIssue)}
              <IssueSpecPanel {projectPath} issueNumber={detailIssue.number} />
            {:else if detailIssue.body}
              <MarkdownRenderer text={detailIssue.body} />
            {:else}
              <p class="ilp-empty">No description provided.</p>
            {/if}
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
    gap: var(--space-lg);
    padding-bottom: var(--space-lg);
    border-bottom: 1px solid var(--border-color);
    margin-bottom: var(--space-lg);
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
    gap: var(--space-md);
  }

  .ilp-state-toggle {
    display: flex;
    gap: var(--space-sm);
  }

  .ilp-category-toggle {
    display: flex;
    gap: var(--space-sm);
  }

  .state-btn {
    padding: var(--space-sm) var(--space-lg);
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    font-size: var(--ui-font-sm);
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    transition:
      border-color var(--transition-fast),
      background var(--transition-fast);
  }

  .state-btn:hover {
    border-color: var(--accent);
  }

  .state-btn.active {
    border-color: var(--accent);
    background: var(--bg-surface);
    color: var(--text-primary);
  }

  .state-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .ilp-refresh-btn {
    padding: var(--space-sm) var(--space-md);
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    font-size: var(--ui-font-lg);
    cursor: pointer;
    font-family: inherit;
    transition: border-color var(--transition-fast);
  }

  .ilp-refresh-btn:hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .ilp-search-row {
    display: flex;
    gap: var(--space-md);
  }

  .ilp-search-input {
    flex: 1;
    padding: var(--space-md) var(--space-lg);
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    color: var(--text-primary);
    font-size: var(--ui-font-md);
    font-family: inherit;
    outline: none;
    transition: border-color var(--transition-fast);
  }

  .ilp-search-input:focus {
    border-color: var(--accent);
  }

  .ilp-index-btn {
    padding: var(--space-md) var(--space-lg);
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    font-size: var(--ui-font-sm);
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    white-space: nowrap;
    transition:
      border-color var(--transition-fast),
      color var(--transition-fast);
  }

  .ilp-index-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .ilp-index-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .ilp-search-hint,
  .ilp-status,
  .ilp-search-note {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
  }

  .ilp-label-filters {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
    align-items: center;
  }

  .ilp-label-chip {
    padding: var(--space-xs) var(--space-md);
    border-radius: var(--radius-xl);
    font-size: var(--ui-font-xs);
    font-weight: 600;
    cursor: pointer;
    border: 2px solid transparent;
    font-family: inherit;
    transition:
      border-color var(--transition-fast),
      opacity var(--transition-fast);
  }

  .ilp-label-chip:hover {
    opacity: 0.85;
  }

  .ilp-label-chip.active {
    border-color: var(--accent);
  }

  .ilp-label-clear {
    padding: var(--space-xs) var(--space-md);
    border-radius: var(--radius-md);
    font-size: var(--ui-font-xs);
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    color: var(--text-muted);
    cursor: pointer;
    font-family: inherit;
  }

  .ilp-error {
    padding: var(--space-lg);
    border: 1px solid rgba(255, 0, 0, 0.3);
    background: rgba(255, 0, 0, 0.06);
    border-radius: var(--radius-lg);
    color: var(--text-primary);
    font-size: var(--ui-font-md);
  }

  .ilp-loading,
  .ilp-empty {
    padding: var(--space-2xl);
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
    gap: var(--space-sm);
    padding: var(--space-lg);
    border-bottom: 1px solid var(--border-color);
    cursor: pointer;
    transition: background var(--transition-fast);
  }

  .ilp-issue-row:hover {
    background: var(--bg-surface);
  }

  .ilp-issue-main {
    display: flex;
    align-items: baseline;
    gap: var(--space-md);
  }

  .ilp-issue-number {
    font-family: var(--font-mono);
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
    gap: var(--space-md);
    flex-wrap: wrap;
  }

  .ilp-issue-labels {
    display: flex;
    gap: var(--space-sm);
    flex-wrap: wrap;
  }

  .ilp-issue-label {
    font-size: var(--ui-font-xs);
    padding: 1px var(--space-md);
    border-radius: var(--radius-pill);
    font-weight: 600;
    white-space: nowrap;
  }

  .ilp-issue-assignees {
    display: flex;
    gap: var(--space-xs);
  }

  .ilp-avatar {
    border-radius: var(--radius-circle);
    border: 1px solid var(--border-color);
  }

  .ilp-issue-milestone {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    padding: 1px var(--space-md);
    border-radius: var(--radius-sm);
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
  }

  .ilp-issue-comments {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .ilp-search-score {
    font-size: var(--ui-font-xs);
    color: var(--accent);
    font-weight: 700;
  }

  .ilp-issue-updated {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    margin-left: auto;
  }

  .ilp-worktree-btn {
    padding: var(--space-xs) var(--space-md);
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    font-size: var(--ui-font-xs);
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    transition:
      border-color var(--transition-fast),
      color var(--transition-fast);
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
    padding: var(--space-lg);
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
    padding-bottom: var(--space-lg);
    border-bottom: 1px solid var(--border-color);
    margin-bottom: var(--space-lg);
  }

  .ilp-back-btn {
    padding: var(--space-md) var(--space-lg);
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    font-size: var(--ui-font-md);
    cursor: pointer;
    font-family: inherit;
    transition: border-color var(--transition-fast);
  }

  .ilp-back-btn:hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .ilp-detail-content {
    display: flex;
    flex-direction: column;
    gap: var(--space-xl);
  }

  .ilp-detail-title-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-lg);
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
    padding: 3px var(--space-lg);
    border-radius: var(--radius-pill);
    font-size: var(--ui-font-sm);
    font-weight: 700;
    text-transform: capitalize;
    background: rgba(46, 160, 67, 0.2);
    color: var(--green);
    border: 1px solid rgba(46, 160, 67, 0.3);
  }

  .ilp-detail-state.closed {
    background: rgba(139, 92, 246, 0.2);
    color: var(--magenta);
    border: 1px solid rgba(139, 92, 246, 0.3);
  }

  .ilp-detail-meta {
    display: flex;
    align-items: center;
    gap: var(--space-lg);
    flex-wrap: wrap;
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
  }

  .ilp-detail-milestone {
    padding: 1px var(--space-md);
    border-radius: var(--radius-sm);
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    font-size: var(--ui-font-xs);
  }

  .ilp-detail-updated,
  .ilp-detail-comments {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .ilp-detail-body {
    border: 1px solid var(--border-color);
    border-radius: var(--radius-lg);
    background: var(--bg-secondary);
    padding: var(--space-xl);
    box-shadow: var(--shadow-sm);
  }

  .ilp-detail-actions {
    display: flex;
    gap: var(--space-md);
    padding-bottom: var(--space-md);
  }

  .ilp-action-btn {
    padding: var(--space-md) var(--space-xl);
    border: none;
    border-radius: var(--radius-md);
    font-size: var(--ui-font-md);
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    transition: background var(--transition-fast);
  }

  .ilp-action-work {
    background: var(--accent);
    color: var(--bg-primary);
  }

  .ilp-action-work:hover {
    background: var(--accent-hover);
  }

  .ilp-action-switch {
    background: var(--green);
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
