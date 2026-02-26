<script lang="ts">
  import type { PrStatusInfo, WorkflowRunInfo } from "../types";
  import { workflowStatusIcon, workflowStatusClass } from "../prStatusHelpers";

  let {
    prDetail = null,
    loading = false,
    error = null,
    updateError = null,
    onOpenCiLog,
    onUpdateBranch,
    updatingBranch = false,
    onMerge,
    merging = false,
    retrying = false,
  }: {
    prDetail?: PrStatusInfo | null;
    loading?: boolean;
    error?: string | null;
    updateError?: string | null;
    onOpenCiLog?: (run: WorkflowRunInfo) => void;
    onUpdateBranch?: () => Promise<void>;
    updatingBranch?: boolean;
    onMerge?: () => void;
    merging?: boolean;
    retrying?: boolean;
  } = $props();

  let checksExpanded = $state(false);

  function reviewStateIcon(state: string): string {
    switch (state) {
      case "APPROVED":
        return "\u2713";
      case "CHANGES_REQUESTED":
        return "\u2717";
      case "COMMENTED":
        return "\u25C6";
      case "PENDING":
        return "\u25CB";
      case "DISMISSED":
        return "\u2014";
      default:
        return "?";
    }
  }

  function mergeableLabel(
    state: "OPEN" | "CLOSED" | "MERGED",
    m: "MERGEABLE" | "CONFLICTING" | "UNKNOWN"
  ): string {
    if (state === "MERGED") return "Merged";
    switch (m) {
      case "MERGEABLE":
        return "Mergeable";
      case "CONFLICTING":
        return "Conflicting";
      case "UNKNOWN":
        return "Unknown";
    }
  }

  function mergeableClass(
    state: "OPEN" | "CLOSED" | "MERGED",
    m: "MERGEABLE" | "CONFLICTING" | "UNKNOWN"
  ): string {
    if (state === "MERGED") return "merged";
    switch (m) {
      case "MERGEABLE":
        return "mergeable";
      case "CONFLICTING":
        return "conflicting";
      case "UNKNOWN":
        return "unknown";
    }
  }

  function isMergeClickable(
    state: "OPEN" | "CLOSED" | "MERGED",
    mergeable: "MERGEABLE" | "CONFLICTING" | "UNKNOWN"
  ): boolean {
    return state === "OPEN" && mergeable === "MERGEABLE";
  }

  function shouldShowMergeableBadge(
    state: "OPEN" | "CLOSED" | "MERGED",
    mergeStateStatus: string | null | undefined
  ): boolean {
    if (state === "MERGED") return true;
    return mergeStateStatus !== "BLOCKED";
  }

  /**
   * States that should display an additional badge.
   * CLEAN, HAS_HOOKS, UNKNOWN are hidden.
   * DIRTY + CONFLICTING is hidden to avoid duplicate "Conflicting/Conflicts" badges.
   */
  function shouldShowMergeStateBadge(
    state: "OPEN" | "CLOSED" | "MERGED",
    mergeable: "MERGEABLE" | "CONFLICTING" | "UNKNOWN",
    s: string | null | undefined
  ): boolean {
    if (!s) return false;
    if (state === "MERGED") return false;
    if (s === "DIRTY" && mergeable === "CONFLICTING") return false;
    return ["BEHIND", "BLOCKED", "DIRTY", "DRAFT", "UNSTABLE"].includes(s);
  }

  function mergeStateLabel(s: string): string {
    switch (s) {
      case "BEHIND": return "Behind base";
      case "BLOCKED": return "Blocked";
      case "DIRTY": return "Conflicts";
      case "DRAFT": return "Draft";
      case "UNSTABLE": return "Unstable";
      default: return s;
    }
  }

  function mergeStateClass(s: string): string {
    switch (s) {
      case "BEHIND": return "behind";
      case "DIRTY":
      case "BLOCKED": return "blocked";
      case "UNSTABLE": return "unstable";
      default: return "neutral";
    }
  }

  function workflowStatusText(run: WorkflowRunInfo): string {
    if (run.status === "in_progress") return "Running";
    if (run.status === "queued") return "Queued";
    if (run.status !== "completed") return run.status;
    switch (run.conclusion) {
      case "success": return "Success";
      case "failure": return "Failure";
      case "neutral": return "Neutral";
      case "skipped": return "Skipped";
      default: return "Completed";
    }
  }

  function handleCheckClick(run: WorkflowRunInfo) {
    if (onOpenCiLog) {
      onOpenCiLog(run);
    } else if (prDetail?.url) {
      // Fallback: open GitHub Actions URL in browser
      const repoUrl = prDetail.url.replace(/\/pull\/\d+$/, "");
      window.open(`${repoUrl}/actions/runs/${run.runId}`, "_blank");
    }
  }
</script>

<div class="pr-status-section">
{#if loading}
    <div class="pr-status-placeholder">Loading...</div>
  {:else if error}
    <div class="pr-status-error">{error}</div>
  {:else if !prDetail}
    <div class="pr-status-placeholder">No PR</div>
  {:else}
    {#if updateError}
      <div class="pr-status-warning">{updateError}</div>
    {/if}
    <div class="pr-title">
      <a href={prDetail.url} target="_blank" rel="noopener noreferrer">
        #{prDetail.number} {prDetail.title}
      </a>
    </div>

    <div class="pr-meta">
      <div class="pr-meta-item">
        <span class="pr-meta-label">Author</span>
        <span class="pr-meta-value">{prDetail.author}</span>
      </div>
      <div class="pr-meta-item">
        <span class="pr-meta-label">Branch</span>
        <span class="pr-meta-value">{prDetail.baseBranch} ← {prDetail.headBranch}</span>
      </div>
      <div class="pr-meta-item">
        <span class="pr-meta-label">Merge</span>
        <span class="pr-meta-value merge-meta-value">
          {#if shouldShowMergeableBadge(prDetail.state, prDetail.mergeStateStatus)}
            {#if isMergeClickable(prDetail.state, prDetail.mergeable) && onMerge}
              <button
                class="mergeable-badge-btn mergeable-badge {mergeableClass(prDetail.state, prDetail.mergeable)}{retrying ? ' pulse' : ''}"
                disabled={merging || retrying}
                onclick={() => onMerge?.()}
              >
                {merging ? "Merging..." : retrying ? "Checking merge status..." : mergeableLabel(prDetail.state, prDetail.mergeable)}
              </button>
            {:else}
              <span class="mergeable-badge {mergeableClass(prDetail.state, prDetail.mergeable)}{retrying ? ' pulse' : ''}">
                {mergeableLabel(prDetail.state, prDetail.mergeable)}
              </span>
            {/if}
          {/if}
          {#if shouldShowMergeStateBadge(prDetail.state, prDetail.mergeable, prDetail.mergeStateStatus)}
            <span class="merge-state-badge {mergeStateClass(prDetail.mergeStateStatus!)}">
              {mergeStateLabel(prDetail.mergeStateStatus!)}
            </span>
          {/if}
          {#if prDetail.state !== "MERGED" && prDetail.mergeStateStatus === "BEHIND"}
            <button
              class="update-branch-btn"
              disabled={updatingBranch}
              onclick={() => onUpdateBranch?.()}
            >
              {updatingBranch ? "Updating..." : "Update Branch"}
            </button>
          {/if}
        </span>
      </div>
      {#if (prDetail.labels?.length ?? 0) > 0}
        <div class="pr-meta-item">
          <span class="pr-meta-label">Labels</span>
          <span class="pr-meta-value">
            {#each prDetail.labels as label}
              <span class="label-pill">{label}</span>
            {/each}
          </span>
        </div>
      {/if}
      {#if (prDetail.assignees?.length ?? 0) > 0}
        <div class="pr-meta-item">
          <span class="pr-meta-label">Assignees</span>
          <span class="pr-meta-value">{prDetail.assignees.join(", ")}</span>
        </div>
      {/if}
      {#if prDetail.milestone}
        <div class="pr-meta-item">
          <span class="pr-meta-label">Milestone</span>
          <span class="pr-meta-value">{prDetail.milestone}</span>
        </div>
      {/if}
      {#if (prDetail.linkedIssues?.length ?? 0) > 0}
        <div class="pr-meta-item">
          <span class="pr-meta-label">Issues</span>
          <span class="pr-meta-value">
            {prDetail.linkedIssues.map((n) => `#${n}`).join(", ")}
          </span>
        </div>
      {/if}
    </div>

    {#if (prDetail.checkSuites?.length ?? 0) > 0}
      <div class="checks-section">
        <button class="checks-toggle" onclick={() => checksExpanded = !checksExpanded}>
          <span class="checks-toggle-icon">{checksExpanded ? "\u25BC" : "\u25B6"}</span>
          <h4>Checks ({prDetail.checkSuites.length})</h4>
        </button>
        {#if checksExpanded}
          <div class="checks-list">
            {#each prDetail.checkSuites as run}
              <button class="check-item" onclick={() => handleCheckClick(run)}>
                <span class="check-status {workflowStatusClass(run)}">
                  {workflowStatusIcon(run)}
                </span>
                <span class="check-name">{run.workflowName}</span>
                <span class="check-conclusion">{workflowStatusText(run)}</span>
                {#if run.isRequired}
                  <span class="required-badge">required</span>
                {/if}
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {:else}
      <div class="checks-empty">No checks</div>
    {/if}

    {#if (prDetail.reviews?.length ?? 0) > 0}
      <div class="reviews-section">
        <h4>Reviews</h4>
        {#each prDetail.reviews as review}
          <div class="review-item">
            <span class="review-state {review.state.toLowerCase()}">
              {reviewStateIcon(review.state)}
            </span>
            <span class="reviewer-name">{review.reviewer}</span>
          </div>
        {/each}
      </div>
    {/if}

    {#if (prDetail.reviewComments?.length ?? 0) > 0}
      <div class="comments-section">
        <h4>Comments</h4>
        {#each prDetail.reviewComments as comment}
          <div class="comment-item">
            <div class="comment-header">
              <span class="comment-author">{comment.author}</span>
              {#if comment.filePath}
                <span class="comment-file">{comment.filePath}{comment.line ? `:${comment.line}` : ""}</span>
              {/if}
            </div>
            {#if comment.codeSnippet}
              <pre class="code-snippet"><code>{comment.codeSnippet}</code></pre>
            {/if}
            <div class="comment-body">{comment.body}</div>
          </div>
        {/each}
      </div>
    {/if}

    <div class="changes-section">
      <h4>Changes</h4>
      <div class="changes-stats">
        <span>{prDetail.changedFilesCount} files changed</span>
        <span class="additions">+{prDetail.additions}</span>
        <span class="deletions">-{prDetail.deletions}</span>
      </div>
    </div>
  {/if}
</div>

<style>
  .pr-status-section {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .pr-title {
    font-size: var(--ui-font-lg);
    font-weight: 700;
    color: var(--text-primary);
  }

  .pr-title a {
    color: var(--accent);
    text-decoration: none;
  }

  .pr-title a:hover {
    text-decoration: underline;
  }

  .pr-meta {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--ui-font-sm);
  }

  .pr-meta-item {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  .pr-meta-label {
    font-weight: 600;
    color: var(--text-muted);
    min-width: 88px;
    font-size: var(--ui-font-xs);
    text-transform: uppercase;
  }

  .pr-meta-value {
    color: var(--text-primary);
  }

  .mergeable-badge {
    padding: 2px 8px;
    border-radius: 999px;
    font-size: var(--ui-font-xs);
    font-weight: 600;
  }

  .mergeable-badge.mergeable {
    background: rgba(63, 185, 80, 0.15);
    color: var(--green);
  }

  .mergeable-badge.merged {
    background: rgba(63, 185, 80, 0.15);
    color: var(--green);
  }

  .mergeable-badge.conflicting {
    background: rgba(248, 81, 73, 0.15);
    color: var(--red);
  }

  .mergeable-badge.unknown {
    background: rgba(128, 128, 128, 0.15);
    color: var(--text-muted);
  }

  .mergeable-badge-btn {
    border: 1px solid transparent;
    cursor: pointer;
    transition: background 0.15s, border-color 0.15s;
  }

  .mergeable-badge-btn:hover:not(:disabled) {
    border-color: var(--green);
    background: rgba(63, 185, 80, 0.25);
  }

  .mergeable-badge-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .label-pill {
    display: inline-flex;
    padding: 1px 8px;
    border-radius: 999px;
    font-size: var(--ui-font-xs);
    border: 1px solid var(--border-color);
    background: var(--bg-secondary);
    margin-right: 4px;
  }

  .merge-meta-value {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .merge-state-badge {
    padding: 2px 8px;
    border-radius: 999px;
    font-size: var(--ui-font-xs);
    font-weight: 600;
  }

  .merge-state-badge.clean {
    background: rgba(63, 185, 80, 0.15);
    color: var(--green);
  }

  .merge-state-badge.behind {
    background: rgba(227, 179, 65, 0.15);
    color: var(--yellow, #e3b341);
  }

  .merge-state-badge.blocked {
    background: rgba(248, 81, 73, 0.15);
    color: var(--red);
  }

  .merge-state-badge.unstable {
    background: rgba(227, 179, 65, 0.15);
    color: var(--yellow, #e3b341);
  }

  .merge-state-badge.neutral {
    background: rgba(128, 128, 128, 0.15);
    color: var(--text-muted);
  }

  .update-branch-btn {
    padding: 2px 10px;
    border-radius: 6px;
    font-size: var(--ui-font-xs);
    font-weight: 600;
    border: 1px solid var(--border-color);
    background: var(--bg-secondary);
    color: var(--text-primary);
    cursor: pointer;
    transition: background 0.15s;
  }

  .update-branch-btn:hover:not(:disabled) {
    background: var(--bg-hover, var(--bg-primary));
  }

  .update-branch-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .checks-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .checks-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
    color: var(--text-secondary);
  }

  .checks-toggle h4 {
    font-size: var(--ui-font-sm);
    font-weight: 700;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin: 0;
  }

  .checks-toggle-icon {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .checks-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .check-item {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--ui-font-sm);
    padding: 4px 8px;
    border-radius: 6px;
    cursor: pointer;
    background: none;
    border: none;
    color: var(--text-primary);
    text-align: left;
    width: 100%;
  }

  .check-item:hover {
    background: var(--bg-secondary);
  }

  .check-status.pass {
    color: var(--green);
  }

  .check-status.fail {
    color: var(--red);
  }

  .check-status.running {
    color: var(--yellow, #e3b341);
  }

  .check-status.pending,
  .check-status.neutral {
    color: var(--text-muted);
  }

  .check-conclusion {
    color: var(--text-muted);
    font-size: var(--ui-font-xs);
    margin-left: auto;
  }

  .checks-empty {
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
  }

  .required-badge {
    padding: 1px 6px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 600;
    background: rgba(128, 128, 128, 0.15);
    color: var(--text-muted);
    text-transform: lowercase;
  }

  .reviews-section h4,
  .comments-section h4,
  .changes-section h4 {
    font-size: var(--ui-font-sm);
    font-weight: 700;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 8px;
  }

  .review-item {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--ui-font-sm);
    padding: 4px 0;
  }

  .review-state.approved {
    color: var(--green);
  }
  .review-state.changes_requested {
    color: var(--red);
  }
  .review-state.commented {
    color: var(--cyan);
  }
  .review-state.pending,
  .review-state.dismissed {
    color: var(--text-muted);
  }

  .comment-item {
    border: 1px solid var(--border-color);
    border-radius: 8px;
    padding: 10px;
    margin-bottom: 8px;
    background: var(--bg-secondary);
  }

  .comment-header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 6px;
    font-size: var(--ui-font-xs);
  }

  .comment-author {
    font-weight: 700;
    color: var(--text-primary);
  }

  .comment-file {
    color: var(--text-muted);
    font-family: monospace;
  }

  .code-snippet {
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 8px;
    font-size: var(--ui-font-xs);
    font-family: monospace;
    overflow-x: auto;
    margin-bottom: 6px;
  }

  .comment-body {
    font-size: var(--ui-font-sm);
    color: var(--text-primary);
    line-height: 1.5;
  }

  .changes-stats {
    display: flex;
    gap: 12px;
    font-size: var(--ui-font-sm);
  }

  .additions {
    color: var(--green);
    font-weight: 600;
  }
  .deletions {
    color: var(--red);
    font-weight: 600;
  }

  .pr-status-placeholder {
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
    padding: 12px 0;
  }

  .pr-status-error {
    padding: 10px 12px;
    border: 1px solid rgba(255, 0, 0, 0.35);
    background: rgba(255, 0, 0, 0.08);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: var(--ui-font-sm);
    line-height: 1.4;
  }

  .pr-status-warning {
    padding: 10px 12px;
    border: 1px solid rgba(255, 179, 0, 0.35);
    background: rgba(255, 179, 0, 0.12);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: var(--ui-font-sm);
    line-height: 1.4;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .pulse {
    animation: pulse 1.5s ease-in-out infinite;
  }
</style>
