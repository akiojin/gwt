<script lang="ts">
  import type {
    BranchPrPreflight,
    MergeUiState,
    PrStatusInfo,
    WorkflowRunInfo,
  } from "../types";
  import { workflowStatusIcon, workflowStatusClass } from "../prStatusHelpers";

  let {
    prDetail = null,
    loading = false,
    error = null,
    preflight = null,
    preflightLoading = false,
    preflightError = null,
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
    preflight?: BranchPrPreflight | null;
    preflightLoading?: boolean;
    preflightError?: string | null;
    updateError?: string | null;
    onOpenCiLog?: (run: WorkflowRunInfo) => void;
    onUpdateBranch?: () => Promise<void>;
    updatingBranch?: boolean;
    onMerge?: () => void;
    merging?: boolean;
    retrying?: boolean;
  } = $props();

  let checksExpanded = $state(false);
  const resolvedMergeUiState = $derived.by(() => {
    if (!prDetail) return "checking" as MergeUiState;
    return resolveMergeUiState(prDetail, retrying);
  });
  const checksWarning = $derived.by(() => {
    if (!prDetail) return false;
    return shouldShowChecksWarning(prDetail);
  });
  const shouldShowPreflightBanner = $derived.by(() => {
    if (prDetail || !preflight || !preflight.blockingReason) return false;
    return preflight.status === "behind" || preflight.status === "diverged";
  });

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

  function isFailureConclusion(
    conclusion: WorkflowRunInfo["conclusion"] | string | null | undefined
  ): boolean {
    return [
      "failure",
      "cancelled",
      "timed_out",
      "action_required",
      "startup_failure",
    ].includes((conclusion ?? "").toString());
  }

  function hasRequiredCheckFailure(checks: WorkflowRunInfo[]): boolean {
    return checks.some(
      (check) =>
        check.isRequired === true && isFailureConclusion(check.conclusion),
    );
  }

  function hasNonRequiredCheckFailure(checks: WorkflowRunInfo[]): boolean {
    return checks.some(
      (check) =>
        check.isRequired === false && isFailureConclusion(check.conclusion),
    );
  }

  function hasPendingRequiredCheck(checks: WorkflowRunInfo[]): boolean {
    return checks.some(
      (check) =>
        check.isRequired === true &&
        (check.status !== "completed" || check.conclusion == null),
    );
  }

  function hasChangesRequested(pr: PrStatusInfo): boolean {
    return pr.reviews.some((review) => review.state === "CHANGES_REQUESTED");
  }

  function asMergeUiState(value: string | null | undefined): MergeUiState | null {
    switch ((value ?? "").toString()) {
      case "merged":
      case "closed":
      case "checking":
      case "blocked":
      case "conflicting":
      case "mergeable":
        return value as MergeUiState;
      default:
        return null;
    }
  }

  function resolveMergeUiState(pr: PrStatusInfo, retryingNow: boolean): MergeUiState {
    if (pr.state === "MERGED") return "merged";
    if (pr.state === "CLOSED") return "closed";
    if (retryingNow) return "checking";
    const explicit = asMergeUiState(pr.mergeUiState ?? null);
    if (explicit) return explicit;
    if (hasRequiredCheckFailure(pr.checkSuites) || hasChangesRequested(pr)) {
      return "blocked";
    }
    if (pr.mergeStateStatus === "BLOCKED" && hasPendingRequiredCheck(pr.checkSuites)) {
      return "checking";
    }
    if (pr.mergeStateStatus === "BLOCKED") return "blocked";
    if (pr.mergeable === "UNKNOWN" || pr.mergeStateStatus === "UNKNOWN") {
      return "checking";
    }
    if (pr.mergeable === "CONFLICTING") return "conflicting";
    return "mergeable";
  }

  function mergeUiLabel(uiState: MergeUiState): string {
    switch (uiState) {
      case "merged":
        return "Merged";
      case "closed":
        return "Closed";
      case "checking":
        return "Checking merge status...";
      case "blocked":
        return "Blocked";
      case "conflicting":
        return "Conflicting";
      case "mergeable":
      default:
        return "Mergeable";
    }
  }

  function mergeUiClass(uiState: MergeUiState): string {
    return uiState;
  }

  function shouldShowChecksWarning(pr: PrStatusInfo): boolean {
    if (typeof pr.nonRequiredChecksWarning === "boolean") {
      return pr.nonRequiredChecksWarning;
    }
    return hasNonRequiredCheckFailure(pr.checkSuites) && !hasRequiredCheckFailure(pr.checkSuites);
  }

  function canShowMergeButton(pr: PrStatusInfo, uiState: MergeUiState): boolean {
    return uiState === "mergeable" && pr.state === "OPEN" && pr.mergeable === "MERGEABLE";
  }

  /**
   * States that should display an additional badge.
   * CLEAN, BLOCKED, HAS_HOOKS, UNKNOWN are hidden.
   * DIRTY + CONFLICTING is hidden to avoid duplicate "Conflicting/Conflicts" badges.
   */
  function shouldShowMergeStateBadge(
    state: "OPEN" | "CLOSED" | "MERGED",
    uiState: MergeUiState,
    mergeable: "MERGEABLE" | "CONFLICTING" | "UNKNOWN",
    s: string | null | undefined
  ): boolean {
    if (!s) return false;
    if (state === "MERGED" || state === "CLOSED") return false;
    if (uiState === "blocked") return false;
    if (s === "DIRTY" && mergeable === "CONFLICTING") return false;
    return ["BEHIND", "DIRTY", "DRAFT", "UNSTABLE"].includes(s);
  }

  function mergeStateLabel(s: string): string {
    switch (s) {
      case "BEHIND": return "Behind base";
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
        return "blocked";
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

  function commitCountLabel(count: number): string {
    return `${count} commit${count === 1 ? "" : "s"}`;
  }
</script>

<div class="pr-status-section">
{#if loading}
    <div class="pr-status-placeholder">Loading...</div>
  {:else if error}
    <div class="pr-status-error">{error}</div>
  {:else if !prDetail}
    {#if preflightError}
      <div class="pr-status-warning">{preflightError}</div>
    {/if}
    {#if shouldShowPreflightBanner}
      <div class="pr-status-warning pr-preflight-warning">
        <div>{preflight!.blockingReason}</div>
        <div class="pr-preflight-meta">
          Base: {preflight!.baseBranch}
          <span>Behind: {commitCountLabel(preflight!.behindBy)}</span>
          {#if preflight!.aheadBy > 0}
            <span>Ahead: {commitCountLabel(preflight!.aheadBy)}</span>
          {/if}
        </div>
      </div>
    {/if}
    <div class="pr-status-placeholder">
      {preflightLoading ? "Checking branch sync..." : "No PR"}
    </div>
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
          {#if canShowMergeButton(prDetail, resolvedMergeUiState) && onMerge}
            <button
              class="mergeable-badge-btn mergeable-badge {mergeUiClass(resolvedMergeUiState)}"
              disabled={merging || resolvedMergeUiState === "checking"}
              onclick={() => onMerge?.()}
            >
              {merging ? "Merging..." : mergeUiLabel(resolvedMergeUiState)}
            </button>
          {:else}
            <span
              class="mergeable-badge {mergeUiClass(resolvedMergeUiState)}{resolvedMergeUiState === 'checking' ? ' pulse' : ''}"
            >
              {mergeUiLabel(resolvedMergeUiState)}
            </span>
          {/if}
          {#if checksWarning}
            <span class="merge-warning-badge">Checks warning</span>
          {/if}
          {#if shouldShowMergeStateBadge(prDetail.state, resolvedMergeUiState, prDetail.mergeable, prDetail.mergeStateStatus)}
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
    gap: var(--space-3);
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
    gap: var(--space-2);
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
    padding: 2px var(--space-2);
    border-radius: var(--radius-full);
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

  .mergeable-badge.blocked {
    background: rgba(248, 81, 73, 0.15);
    color: var(--red);
  }

  .mergeable-badge.checking {
    background: rgba(128, 128, 128, 0.15);
    color: var(--text-muted);
  }

  .mergeable-badge.closed {
    background: rgba(248, 81, 73, 0.12);
    color: var(--red);
  }

  .merge-warning-badge {
    padding: 2px var(--space-2);
    border-radius: var(--radius-full);
    font-size: var(--ui-font-xs);
    font-weight: 600;
    background: rgba(227, 179, 65, 0.15);
    color: var(--yellow, #e3b341);
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
    padding: 1px var(--space-2);
    border-radius: var(--radius-full);
    font-size: var(--ui-font-xs);
    border: 1px solid var(--border-color);
    background: var(--bg-secondary);
    margin-right: var(--space-1);
  }

  .merge-meta-value {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .merge-state-badge {
    padding: 2px var(--space-2);
    border-radius: var(--radius-full);
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
    border-radius: var(--radius-sm);
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
    gap: var(--space-2);
    font-size: var(--ui-font-sm);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
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
    border-radius: var(--radius-full);
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
    margin-bottom: var(--space-2);
  }

  .review-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--ui-font-sm);
    padding: var(--space-1) 0;
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
    border-radius: var(--radius-md);
    padding: 10px;
    margin-bottom: var(--space-2);
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
    border-radius: var(--radius-sm);
    padding: var(--space-2);
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
    gap: var(--space-3);
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
    padding: var(--space-3) 0;
  }

  .pr-status-error {
    padding: 10px var(--space-3);
    border: 1px solid rgba(255, 0, 0, 0.35);
    background: rgba(255, 0, 0, 0.08);
    border-radius: var(--radius-md);
    color: var(--text-primary);
    font-size: var(--ui-font-sm);
    line-height: 1.4;
  }

  .pr-status-warning {
    padding: 10px var(--space-3);
    border: 1px solid rgba(255, 179, 0, 0.35);
    background: rgba(255, 179, 0, 0.12);
    border-radius: var(--radius-md);
    color: var(--text-primary);
    font-size: var(--ui-font-sm);
    line-height: 1.4;
  }

  .pr-preflight-warning {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .pr-preflight-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .pulse {
    animation: pulse 1.5s ease-in-out infinite;
  }
</style>
