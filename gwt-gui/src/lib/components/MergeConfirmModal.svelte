<script lang="ts">
  import type { PrStatusInfo, WorkflowRunInfo, ReviewInfo } from "../types";
  import { workflowStatusIcon, workflowStatusClass } from "../prStatusHelpers";

  let {
    open = false,
    prDetail = null,
    merging = false,
    onClose,
    onConfirm,
  }: {
    open: boolean;
    prDetail?: PrStatusInfo | null;
    merging?: boolean;
    onClose: () => void;
    onConfirm: () => void;
  } = $props();

  function reviewStateIcon(state: string): string {
    switch (state) {
      case "APPROVED": return "\u2713";
      case "CHANGES_REQUESTED": return "\u2717";
      case "COMMENTED": return "\u25C6";
      case "PENDING": return "\u25CB";
      case "DISMISSED": return "\u2014";
      default: return "?";
    }
  }

  function reviewStateClass(state: string): string {
    return state.toLowerCase();
  }

  function handleWindowKeydown(e: KeyboardEvent) {
    if (!open || !prDetail) return;
    if (e.key !== "Escape") return;
    e.preventDefault();
    onClose();
  }
</script>

<svelte:window onkeydown={handleWindowKeydown} />

{#if open && prDetail}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="overlay modal-overlay"
    onclick={onClose}
    role="dialog"
    aria-modal="true"
    aria-label="Merge Pull Request"
    tabindex="-1"
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="dialog merge-dialog" onclick={(e) => e.stopPropagation()}>
      <div class="dialog-header">
        <h2>Merge Pull Request</h2>
        <button class="close-btn" onclick={onClose} aria-label="Close">&times;</button>
      </div>

      <div class="dialog-body">
        <div class="pr-info">
          <div class="pr-info-title">#{prDetail.number} {prDetail.title}</div>
          <div class="pr-info-branch">
            {prDetail.headBranch} → {prDetail.baseBranch}
          </div>
        </div>

        {#if (prDetail.checkSuites?.length ?? 0) > 0}
          <div class="merge-checks">
            <h4>Checks</h4>
            {#each prDetail.checkSuites as run}
              <div class="merge-check-item">
                <span class="check-status {workflowStatusClass(run)}">
                  {workflowStatusIcon(run)}
                </span>
                <span class="check-name">{run.workflowName}</span>
              </div>
            {/each}
          </div>
        {/if}

        {#if (prDetail.reviews?.length ?? 0) > 0}
          <div class="merge-reviews">
            <h4>Reviews</h4>
            {#each prDetail.reviews as review}
              <div class="merge-review-item">
                <span class="review-state {reviewStateClass(review.state)}">
                  {reviewStateIcon(review.state)}
                </span>
                <span>{review.reviewer}</span>
              </div>
            {/each}
          </div>
        {/if}
      </div>

      <div class="dialog-footer">
        <button class="btn btn-cancel" onclick={onClose}>Cancel</button>
        <button
          class="btn btn-merge"
          disabled={merging}
          onclick={onConfirm}
        >
          {merging ? "Merging..." : "Merge"}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: var(--z-modal-base);
  }

  .dialog {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-lg);
    max-width: 480px;
    width: 90vw;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: var(--shadow-xl);
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4) var(--space-5);
    border-bottom: 1px solid var(--border-color);
  }

  .dialog-header h2 {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 20px;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    line-height: 1;
  }

  .close-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .dialog-body {
    padding: var(--space-4) var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .pr-info-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: var(--space-1);
  }

  .pr-info-branch {
    font-size: 12px;
    font-family: monospace;
    color: var(--text-secondary);
  }

  .merge-checks h4,
  .merge-reviews h4 {
    font-size: var(--ui-font-xs, 11px);
    font-weight: 700;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 6px;
  }

  .merge-check-item,
  .merge-review-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 12px;
    padding: 2px 0;
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

  .dialog-footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-4) var(--space-5);
    border-top: 1px solid var(--border-color);
  }

  .btn {
    padding: var(--space-2) var(--space-4);
    border: none;
    border-radius: var(--radius-sm);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    font-family: inherit;
  }

  .btn-cancel {
    background: var(--bg-surface);
    color: var(--text-secondary);
  }

  .btn-cancel:hover {
    background: var(--bg-hover);
  }

  .btn-merge {
    background: var(--green, #3fb950);
    color: var(--bg-primary);
  }

  .btn-merge:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  .btn-merge:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
</style>
