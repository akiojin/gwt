<script lang="ts">
  import type { PrStatusInfo } from "../types";

  let {
    prDetail = null,
    loading = false,
    error = null,
  }: {
    prDetail?: PrStatusInfo | null;
    loading?: boolean;
    error?: string | null;
  } = $props();

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
    m: "MERGEABLE" | "CONFLICTING" | "UNKNOWN"
  ): string {
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
    m: "MERGEABLE" | "CONFLICTING" | "UNKNOWN"
  ): string {
    switch (m) {
      case "MERGEABLE":
        return "mergeable";
      case "CONFLICTING":
        return "conflicting";
      case "UNKNOWN":
        return "unknown";
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
        <span class="pr-meta-value">
          <span class="mergeable-badge {mergeableClass(prDetail.mergeable)}">
            {mergeableLabel(prDetail.mergeable)}
          </span>
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

  .mergeable-badge.conflicting {
    background: rgba(248, 81, 73, 0.15);
    color: var(--red);
  }

  .mergeable-badge.unknown {
    background: rgba(128, 128, 128, 0.15);
    color: var(--text-muted);
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
</style>
