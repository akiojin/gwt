<script lang="ts">
  import type {
    PrListItem,
    FetchPrListResponse,
    GitHubUserResponse,
    GhCliStatus,
    PrStatusInfo,
  } from "../types";
  import { openExternalUrl } from "../openExternalUrl";
  import { workflowStatusIcon, workflowStatusClass } from "../prStatusHelpers";
  import MarkdownRenderer from "./MarkdownRenderer.svelte";
  import MergeDialog from "./MergeDialog.svelte";
  import ReviewDialog from "./ReviewDialog.svelte";

  let {
    projectPath,
    isActive = true,
    onSwitchToWorktree,
  }: {
    projectPath: string;
    isActive?: boolean;
    onSwitchToWorktree: (branchName: string) => void;
  } = $props();

  type StateFilter = "open" | "closed" | "merged";

  let stateFilter: StateFilter = $state("open");
  let searchQuery: string = $state("");
  let prList: PrListItem[] = $state([]);
  let loading: boolean = $state(true);
  let error: string | null = $state(null);
  let hasMore: boolean = $state(false);
  let loadingMore: boolean = $state(false);
  let currentUser: string | null = $state(null);
  let ghCliStatus: GhCliStatus | null = $state(null as GhCliStatus | null);
  let expandedPrNumber: number | null = $state(null);
  let prDetails: Map<number, PrStatusInfo> = $state(new Map());
  let loadedLimit: number = $state(30);
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let isWindowVisible: boolean = $state(
    typeof document === "undefined" ? true : !document.hidden,
  );

  // Dialog state
  let mergeDialogPr: PrListItem | null = $state(null);
  let reviewDialogPr: PrListItem | null = $state(null);
  let markingReady: Set<number> = $state(new Set());
  let updatingBranch: Set<number> = $state(new Set());

  let ghCliAvailable = $derived(
    ghCliStatus !== null && ghCliStatus.available && ghCliStatus.authenticated,
  );

  let filteredPrs = $derived(
    (() => {
      let result = prList;
      const q = searchQuery.trim().toLowerCase();
      if (q) {
        result = result.filter(
          (pr) =>
            pr.title.toLowerCase().includes(q) ||
            `#${pr.number}`.includes(q) ||
            pr.headRefName.toLowerCase().includes(q) ||
            pr.author.login.toLowerCase().includes(q),
        );
      }
      return result;
    })(),
  );

  let sortedPrs = $derived(
    (() => {
      if (!currentUser) return filteredPrs;
      return [...filteredPrs].sort((a, b) => {
        const aPriority = prSortPriority(a);
        const bPriority = prSortPriority(b);
        if (aPriority !== bPriority) return aPriority - bPriority;
        return new Date(b.updatedAt).getTime() - new Date(a.updatedAt).getTime();
      });
    })(),
  );

  function prSortPriority(pr: PrListItem): number {
    if (!currentUser) return 3;
    const isMine = isMyPr(pr);
    const detail = prDetails.get(pr.number);
    const hasActionRequired =
      isMine &&
      detail &&
      (detail.reviews.some((r) => r.state === "CHANGES_REQUESTED") ||
        detail.checkSuites.some(
          (c) => c.status === "completed" && c.conclusion === "failure",
        ));
    if (hasActionRequired) return 0;
    const isReviewRequested =
      !isMine &&
      pr.reviewRequests.some((r) => r.login === currentUser);
    if (isReviewRequested) return 1;
    if (isMine) return 2;
    return 3;
  }

  function isMyPr(pr: PrListItem): boolean {
    if (!currentUser) return false;
    return (
      pr.author.login === currentUser ||
      pr.assignees.some((a) => a.login === currentUser) ||
      pr.reviewRequests.some((r) => r.login === currentUser)
    );
  }

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
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

  function labelStyle(color: string): string {
    const hex = color.replace(/^#/, "");
    if (!/^[0-9a-fA-F]{6}$/.test(hex)) return "";
    const r = parseInt(hex.slice(0, 2), 16);
    const g = parseInt(hex.slice(2, 4), 16);
    const b = parseInt(hex.slice(4, 6), 16);
    const luma = (r * 299 + g * 587 + b * 114) / 1000;
    const textColor = luma > 128 ? "#1e1e2e" : "#cdd6f4";
    return `background-color: #${hex}; color: ${textColor};`;
  }

  function ciBadge(detail: PrStatusInfo | undefined): { icon: string; cls: string } {
    if (!detail || detail.checkSuites.length === 0) return { icon: "", cls: "" };
    const hasFailure = detail.checkSuites.some(
      (c) => c.status === "completed" && c.conclusion === "failure",
    );
    if (hasFailure) return { icon: "\u2717", cls: "fail" };
    const hasRunning = detail.checkSuites.some(
      (c) => c.status === "in_progress" || c.status === "queued",
    );
    if (hasRunning) return { icon: "\u25C9", cls: "running" };
    const allPass = detail.checkSuites.every(
      (c) => c.status === "completed" && c.conclusion === "success",
    );
    if (allPass) return { icon: "\u2713", cls: "pass" };
    return { icon: "\u25CB", cls: "neutral" };
  }

  function reviewBadge(detail: PrStatusInfo | undefined): { icon: string; cls: string } {
    if (!detail || detail.reviews.length === 0) return { icon: "", cls: "" };
    const hasChangesRequested = detail.reviews.some((r) => r.state === "CHANGES_REQUESTED");
    if (hasChangesRequested) return { icon: "\u2717", cls: "fail" };
    const hasApproved = detail.reviews.some((r) => r.state === "APPROVED");
    if (hasApproved) return { icon: "\u2713", cls: "pass" };
    return { icon: "\u25C6", cls: "neutral" };
  }

  function mergeableBadge(detail: PrStatusInfo | undefined): { label: string; cls: string } {
    if (!detail) return { label: "", cls: "" };
    switch (detail.mergeable) {
      case "MERGEABLE":
        return { label: "Mergeable", cls: "mergeable" };
      case "CONFLICTING":
        return { label: "Conflicts", cls: "conflicting" };
      default:
        return { label: "", cls: "" };
    }
  }

  async function checkGhCli() {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      ghCliStatus = await invoke<GhCliStatus>("check_gh_cli_status", {
        projectPath,
      });
    } catch {
      ghCliStatus = { available: false, authenticated: false };
    }
  }

  async function fetchCurrentUser() {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const resp = await invoke<GitHubUserResponse>("fetch_github_user", {
        projectPath,
      });
      currentUser = resp.login;
    } catch {
      currentUser = null;
    }
  }

  async function fetchPrs(append = false) {
    if (append) {
      loadingMore = true;
    } else {
      loading = true;
      prDetails = new Map();
    }
    error = null;

    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const limit = append ? loadedLimit + 30 : 30;
      const resp = await invoke<FetchPrListResponse>("fetch_pr_list", {
        projectPath,
        state: stateFilter,
        limit,
      });

      prList = resp.items;
      loadedLimit = limit;
      hasMore = resp.items.length >= limit;

      // Fetch details for loaded PRs
      for (const pr of resp.items) {
        if (!prDetails.has(pr.number)) {
          void fetchPrDetail(pr.number);
        }
      }
    } catch (err) {
      error = toErrorMessage(err);
    } finally {
      loading = false;
      loadingMore = false;
    }
  }

  async function fetchPrDetail(prNumber: number) {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const detail = await invoke<PrStatusInfo>("fetch_pr_detail", {
        projectPath,
        prNumber,
      });
      prDetails = new Map(prDetails).set(prNumber, detail);
    } catch {
      // Silently ignore detail fetch failures
    }
  }

  function handleStateFilterChange(filter: StateFilter) {
    if (stateFilter === filter) return;
    stateFilter = filter;
    prList = [];
    loadedLimit = 30;
    hasMore = false;
    void fetchPrs();
  }

  function handleRefresh() {
    prList = [];
    prDetails = new Map();
    loadedLimit = 30;
    hasMore = false;
    void fetchPrs();
  }

  function handleShowMore() {
    void fetchPrs(true);
  }

  function toggleExpanded(prNumber: number) {
    expandedPrNumber = expandedPrNumber === prNumber ? null : prNumber;
  }

  async function handleOpenInGitHub(url: string) {
    await openExternalUrl(url);
  }

  function handleSwitchToWorktree(branchName: string) {
    onSwitchToWorktree(branchName);
  }

  function openMergeDialog(pr: PrListItem) {
    mergeDialogPr = pr;
  }

  function openReviewDialog(pr: PrListItem) {
    reviewDialogPr = pr;
  }

  async function handleMarkReady(pr: PrListItem) {
    markingReady = new Set(markingReady).add(pr.number);
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      await invoke<string>("mark_pr_ready", {
        projectPath,
        prNumber: pr.number,
      });
      void handleRefresh();
    } catch (err) {
      error = toErrorMessage(err);
    } finally {
      const next = new Set(markingReady);
      next.delete(pr.number);
      markingReady = next;
    }
  }

  async function handleUpdateBranch(pr: PrListItem) {
    updatingBranch = new Set(updatingBranch).add(pr.number);
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      await invoke<string>("update_pr_branch", {
        projectPath,
        prNumber: pr.number,
      });
      void fetchPrDetail(pr.number);
    } catch (err) {
      error = toErrorMessage(err);
    } finally {
      const next = new Set(updatingBranch);
      next.delete(pr.number);
      updatingBranch = next;
    }
  }

  function handleMerged() {
    mergeDialogPr = null;
    void handleRefresh();
  }

  function handleReviewed() {
    reviewDialogPr = null;
    void handleRefresh();
  }

  // Visibility tracking for poll pause
  function handleVisibilityChange() {
    if (typeof document === "undefined") {
      isWindowVisible = true;
      return;
    }
    isWindowVisible = !document.hidden;
  }

  // Polling
  function startPolling() {
    stopPolling();
    pollTimer = setInterval(() => {
      if (!isActive || !isWindowVisible || !ghCliAvailable) return;
      void fetchPrs();
    }, 30000);
  }

  function stopPolling() {
    if (pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  // Initial data load
  $effect(() => {
    void projectPath;
    void checkGhCli().then(() => {
      if (ghCliAvailable) {
        void fetchCurrentUser();
        void fetchPrs();
      } else {
        loading = false;
      }
    });
  });

  // Start polling when mounted
  $effect(() => {
    const shouldPoll = ghCliAvailable && isActive && isWindowVisible;
    if (!shouldPoll) {
      stopPolling();
      return;
    }
    startPolling();
    return () => {
      stopPolling();
    };
  });

  $effect(() => {
    if (typeof document === "undefined") return;
    handleVisibilityChange();
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  });
</script>

<section class="pr-list-panel">
  <!-- Header -->
  <header class="plp-header">
    <div class="plp-header-top">
      <h2 class="plp-title">Pull Requests</h2>
      <div class="plp-header-actions">
        <div class="plp-state-toggle">
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
          <button
            class="state-btn"
            class:active={stateFilter === "merged"}
            onclick={() => handleStateFilterChange("merged")}
          >
            Merged
          </button>
        </div>
        <button class="plp-refresh-btn" onclick={handleRefresh} title="Refresh">
          &#x21BB;
        </button>
      </div>
    </div>

    <div class="plp-search-row">
      <input
        type="text"
        class="plp-search-input"
        placeholder="Search pull requests..."
        bind:value={searchQuery}
      />
    </div>
  </header>

  <!-- Error / not available -->
  {#if !ghCliAvailable && !loading}
    <div class="plp-error">
      GitHub CLI (gh) is not available or not authenticated. Install and authenticate gh to view pull requests.
    </div>
  {:else if error}
    <div class="plp-error">{error}</div>
  {:else if loading && prList.length === 0}
    <div class="plp-loading">Loading pull requests...</div>
  {:else if sortedPrs.length === 0}
    <div class="plp-empty">No pull requests found.</div>
  {:else}
    <!-- PR List -->
    <div class="plp-list">
      {#each sortedPrs as pr (pr.number)}
        {@const detail = prDetails.get(pr.number)}
        {@const ci = ciBadge(detail)}
        {@const review = reviewBadge(detail)}
        {@const merge = mergeableBadge(detail)}
        {@const mine = isMyPr(pr)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="plp-pr-row"
          class:my-pr={mine}
          onclick={() => toggleExpanded(pr.number)}
        >
          <div class="plp-pr-main">
            {#if pr.isDraft}
              <span class="plp-draft-badge">Draft</span>
            {/if}
            <button
              class="plp-pr-number"
              title="Open in GitHub"
              onclick={(e) => { e.stopPropagation(); handleOpenInGitHub(pr.url); }}
            >
              #{pr.number}
            </button>
            <span class="plp-pr-title">{pr.title}</span>
            <span class="plp-branch-info">
              <span class="plp-branch">{pr.headRefName}</span>
              <span class="plp-branch-arrow">&rarr;</span>
              <span class="plp-branch">{pr.baseRefName}</span>
            </span>
          </div>
          <div class="plp-pr-meta">
            <span class="plp-pr-author">{pr.author.login}</span>
            {#if ci.icon}
              <span class="plp-badge plp-ci-badge {ci.cls}" title="CI">{ci.icon}</span>
            {/if}
            {#if review.icon}
              <span class="plp-badge plp-review-badge {review.cls}" title="Review">{review.icon}</span>
            {/if}
            {#if merge.label}
              <span class="plp-merge-badge {merge.cls}">{merge.label}</span>
            {/if}
            {#if pr.labels.length > 0}
              <span class="plp-pr-labels">
                {#each pr.labels as lbl (lbl.name)}
                  <span class="plp-pr-label" style={labelStyle(lbl.color)}>{lbl.name}</span>
                {/each}
              </span>
            {/if}
            <span class="plp-pr-updated" title={pr.updatedAt}>
              {relativeTime(pr.updatedAt)}
            </span>
            <!-- Action buttons -->
            <span class="plp-pr-actions" onclick={(e) => e.stopPropagation()}>
              {#if pr.state === "OPEN" && !pr.isDraft}
                <button
                  class="plp-action-btn plp-action-merge"
                  title="Merge"
                  onclick={() => openMergeDialog(pr)}
                >
                  Merge
                </button>
              {/if}
              {#if pr.state === "OPEN"}
                <button
                  class="plp-action-btn plp-action-review"
                  title="Review"
                  onclick={() => openReviewDialog(pr)}
                >
                  Review
                </button>
              {/if}
              {#if pr.state === "OPEN" && detail?.mergeStateStatus === "BEHIND"}
                <button
                  class="plp-action-btn"
                  title="Update Branch"
                  disabled={updatingBranch.has(pr.number)}
                  onclick={() => handleUpdateBranch(pr)}
                >
                  {updatingBranch.has(pr.number) ? "Updating..." : "Update"}
                </button>
              {/if}
              {#if pr.isDraft && pr.state === "OPEN"}
                <button
                  class="plp-action-btn plp-action-ready"
                  title="Mark as Ready"
                  disabled={markingReady.has(pr.number)}
                  onclick={() => handleMarkReady(pr)}
                >
                  {markingReady.has(pr.number) ? "..." : "Ready"}
                </button>
              {/if}
              <button
                class="plp-action-btn plp-action-wt"
                title="Switch to worktree: {pr.headRefName}"
                onclick={() => handleSwitchToWorktree(pr.headRefName)}
              >
                WT
              </button>
            </span>
          </div>

          <!-- Expanded body -->
          {#if expandedPrNumber === pr.number}
            <div class="plp-pr-expanded" onclick={(e) => e.stopPropagation()}>
              {#if pr.body}
                <MarkdownRenderer text={pr.body} />
              {:else}
                <p class="plp-empty-body">No description provided.</p>
              {/if}
              {#if detail}
                <div class="plp-detail-section">
                  {#if detail.checkSuites.length > 0}
                    <div class="plp-checks">
                      <h4>Checks ({detail.checkSuites.length})</h4>
                      {#each detail.checkSuites as run}
                        <div class="plp-check-item">
                          <span class="plp-check-status {workflowStatusClass(run)}">
                            {workflowStatusIcon(run)}
                          </span>
                          <span class="plp-check-name">{run.workflowName}</span>
                        </div>
                      {/each}
                    </div>
                  {/if}
                  {#if detail.reviews.length > 0}
                    <div class="plp-reviews">
                      <h4>Reviews</h4>
                      {#each detail.reviews as rv}
                        <div class="plp-review-item">
                          <span class="plp-review-state {rv.state.toLowerCase()}">{rv.reviewer}</span>
                          <span class="plp-review-label">{rv.state}</span>
                        </div>
                      {/each}
                    </div>
                  {/if}
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/each}

      {#if hasMore}
        <div class="plp-show-more">
          <button
            class="plp-show-more-btn"
            disabled={loadingMore}
            onclick={handleShowMore}
          >
            {loadingMore ? "Loading..." : "Show More"}
          </button>
        </div>
      {/if}
    </div>
  {/if}
</section>

<!-- Merge Dialog -->
{#if mergeDialogPr}
  <MergeDialog
    {projectPath}
    prNumber={mergeDialogPr.number}
    prTitle={mergeDialogPr.title}
    onClose={() => (mergeDialogPr = null)}
    onMerged={handleMerged}
  />
{/if}

<!-- Review Dialog -->
{#if reviewDialogPr}
  <ReviewDialog
    {projectPath}
    prNumber={reviewDialogPr.number}
    prTitle={reviewDialogPr.title}
    onClose={() => (reviewDialogPr = null)}
    onReviewed={handleReviewed}
  />
{/if}

<style>
  .pr-list-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .plp-header {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--border-color);
    margin-bottom: 12px;
  }

  .plp-header-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .plp-title {
    font-size: var(--ui-font-xl);
    font-weight: 800;
    color: var(--text-primary);
    margin: 0;
  }

  .plp-header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .plp-state-toggle {
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

  .plp-refresh-btn {
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

  .plp-refresh-btn:hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .plp-search-row {
    display: flex;
  }

  .plp-search-input {
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

  .plp-search-input:focus {
    border-color: var(--accent);
  }

  .plp-error {
    padding: 12px;
    border: 1px solid rgba(255, 0, 0, 0.3);
    background: rgba(255, 0, 0, 0.06);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: var(--ui-font-md);
  }

  .plp-loading,
  .plp-empty {
    padding: 24px;
    text-align: center;
    color: var(--text-muted);
    font-size: var(--ui-font-md);
  }

  .plp-list {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }

  .plp-pr-row {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border-color);
    cursor: pointer;
    transition: background 0.15s;
  }

  .plp-pr-row:hover {
    background: var(--bg-surface);
  }

  .plp-pr-row.my-pr {
    background: rgba(136, 176, 255, 0.08);
  }

  .plp-pr-row.my-pr:hover {
    background: rgba(136, 176, 255, 0.14);
  }

  .plp-pr-main {
    display: flex;
    align-items: baseline;
    gap: 8px;
    flex-wrap: wrap;
  }

  .plp-draft-badge {
    padding: 1px 6px;
    border-radius: 10px;
    font-size: var(--ui-font-xs, 11px);
    font-weight: 700;
    background: rgba(128, 128, 128, 0.2);
    color: var(--text-muted);
  }

  .plp-pr-number {
    font-family: monospace;
    font-weight: 600;
    color: var(--accent);
    flex: 0 0 auto;
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
    font-size: inherit;
  }

  .plp-pr-number:hover {
    text-decoration: underline;
  }

  .plp-pr-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-primary);
    font-weight: 500;
  }

  .plp-branch-info {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--ui-font-xs, 11px);
    color: var(--text-muted);
    flex-shrink: 0;
  }

  .plp-branch {
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    font-family: monospace;
    white-space: nowrap;
  }

  .plp-branch-arrow {
    color: var(--text-muted);
  }

  .plp-pr-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .plp-pr-author {
    font-size: var(--ui-font-xs, 11px);
    color: var(--text-secondary);
    font-weight: 600;
  }

  .plp-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    font-size: 11px;
    font-weight: 700;
  }

  .plp-badge.pass {
    color: var(--green);
    background: rgba(63, 185, 80, 0.15);
  }

  .plp-badge.fail {
    color: var(--red);
    background: rgba(248, 81, 73, 0.15);
  }

  .plp-badge.running {
    color: var(--yellow, #e3b341);
    background: rgba(227, 179, 65, 0.15);
  }

  .plp-badge.neutral {
    color: var(--text-muted);
    background: rgba(128, 128, 128, 0.15);
  }

  .plp-merge-badge {
    padding: 1px 6px;
    border-radius: 999px;
    font-size: var(--ui-font-xs, 11px);
    font-weight: 600;
  }

  .plp-merge-badge.mergeable {
    background: rgba(63, 185, 80, 0.15);
    color: var(--green);
  }

  .plp-merge-badge.conflicting {
    background: rgba(248, 81, 73, 0.15);
    color: var(--red);
  }

  .plp-pr-labels {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }

  .plp-pr-label {
    font-size: var(--ui-font-xs, 11px);
    padding: 1px 6px;
    border-radius: 10px;
    font-weight: 600;
    white-space: nowrap;
  }

  .plp-pr-updated {
    font-size: var(--ui-font-xs, 11px);
    color: var(--text-muted);
    margin-left: auto;
  }

  .plp-pr-actions {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }

  .plp-action-btn {
    padding: 2px 8px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 4px;
    color: var(--text-secondary);
    font-size: var(--ui-font-xs, 11px);
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s;
  }

  .plp-action-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .plp-action-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .plp-action-merge {
    border-color: var(--green);
    color: var(--green);
  }

  .plp-action-review {
    border-color: var(--cyan);
    color: var(--cyan);
  }

  .plp-action-ready {
    border-color: var(--yellow, #e3b341);
    color: var(--yellow, #e3b341);
  }

  .plp-action-wt {
    font-family: monospace;
  }

  .plp-pr-expanded {
    margin-top: 8px;
    padding: 12px;
    border: 1px solid var(--border-color);
    border-radius: 8px;
    background: var(--bg-secondary);
  }

  .plp-empty-body {
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
    margin: 0;
  }

  .plp-detail-section {
    margin-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .plp-checks h4,
  .plp-reviews h4 {
    font-size: var(--ui-font-sm);
    font-weight: 700;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin: 0 0 6px 0;
  }

  .plp-check-item {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--ui-font-sm);
    padding: 2px 0;
  }

  .plp-check-status.pass {
    color: var(--green);
  }
  .plp-check-status.fail {
    color: var(--red);
  }
  .plp-check-status.running {
    color: var(--yellow, #e3b341);
  }
  .plp-check-status.pending,
  .plp-check-status.neutral {
    color: var(--text-muted);
  }

  .plp-review-item {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--ui-font-sm);
    padding: 2px 0;
  }

  .plp-review-state {
    font-weight: 600;
    color: var(--text-primary);
  }

  .plp-review-label {
    font-size: var(--ui-font-xs, 11px);
    color: var(--text-muted);
    text-transform: uppercase;
  }

  .plp-review-state.approved {
    color: var(--green);
  }
  .plp-review-state.changes_requested {
    color: var(--red);
  }
  .plp-review-state.commented {
    color: var(--cyan);
  }
  .plp-review-state.pending,
  .plp-review-state.dismissed {
    color: var(--text-muted);
  }

  .plp-show-more {
    padding: 12px;
    text-align: center;
  }

  .plp-show-more-btn {
    padding: 6px 16px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-secondary);
    font-size: var(--ui-font-sm);
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s;
  }

  .plp-show-more-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .plp-show-more-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
