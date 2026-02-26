<script lang="ts">
  let {
    projectPath,
    prNumber,
    prTitle,
    onClose,
    onReviewed,
  }: {
    projectPath: string;
    prNumber: number;
    prTitle: string;
    onClose: () => void;
    onReviewed: () => void;
  } = $props();

  type ReviewAction = "approve" | "request-changes" | "comment";

  let action: ReviewAction = $state("approve");
  let body: string = $state("");
  let submitting: boolean = $state(false);
  let error: string | null = $state(null);

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  async function handleSubmit() {
    submitting = true;
    error = null;
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      await invoke<string>("review_pr", {
        projectPath,
        prNumber,
        action,
        body: body.trim() || undefined,
      });
      onReviewed();
    } catch (err) {
      error = toErrorMessage(err);
    } finally {
      submitting = false;
    }
  }

  function handleBackdropClick(e: MouseEvent) {
    if (e.target === e.currentTarget) {
      onClose();
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="dialog-backdrop" onclick={handleBackdropClick}>
  <div class="dialog">
    <h3 class="dialog-title">Review PR #{prNumber}</h3>
    <p class="dialog-subtitle">{prTitle}</p>

    {#if error}
      <div class="dialog-error">{error}</div>
    {/if}

    <div class="dialog-field">
      <!-- svelte-ignore a11y_label_has_associated_control -->
      <label class="dialog-label">Review Action</label>
      <div class="dialog-radio-group">
        <label class="dialog-radio">
          <input type="radio" bind:group={action} value="approve" />
          Approve
        </label>
        <label class="dialog-radio">
          <input type="radio" bind:group={action} value="request-changes" />
          Request Changes
        </label>
        <label class="dialog-radio">
          <input type="radio" bind:group={action} value="comment" />
          Comment
        </label>
      </div>
    </div>

    <div class="dialog-field">
      <label class="dialog-label" for="review-body">Comment</label>
      <textarea
        id="review-body"
        class="dialog-textarea"
        bind:value={body}
        rows="4"
        placeholder="Leave a comment..."
      ></textarea>
    </div>

    <div class="dialog-actions">
      <button class="dialog-btn dialog-btn-cancel" onclick={onClose} disabled={submitting}>
        Cancel
      </button>
      <button class="dialog-btn dialog-btn-primary" onclick={handleSubmit} disabled={submitting}>
        {submitting ? "Submitting..." : "Submit Review"}
      </button>
    </div>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.5);
  }

  .dialog {
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 12px;
    padding: 24px;
    min-width: 400px;
    max-width: 520px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .dialog-title {
    font-size: var(--ui-font-lg);
    font-weight: 700;
    color: var(--text-primary);
    margin: 0;
  }

  .dialog-subtitle {
    font-size: var(--ui-font-sm);
    color: var(--text-secondary);
    margin: 0;
  }

  .dialog-error {
    padding: 8px 12px;
    border: 1px solid rgba(255, 0, 0, 0.3);
    background: rgba(255, 0, 0, 0.06);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-sm);
  }

  .dialog-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .dialog-label {
    font-size: var(--ui-font-sm);
    font-weight: 600;
    color: var(--text-secondary);
  }

  .dialog-radio-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .dialog-radio {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--ui-font-sm);
    color: var(--text-primary);
    cursor: pointer;
  }

  .dialog-textarea {
    padding: 8px 10px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-sm);
    font-family: inherit;
    resize: vertical;
    outline: none;
  }

  .dialog-textarea:focus {
    border-color: var(--accent);
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .dialog-btn {
    padding: 8px 16px;
    border-radius: 6px;
    font-size: var(--ui-font-sm);
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    border: none;
    transition: background 0.15s;
  }

  .dialog-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .dialog-btn-cancel {
    background: var(--bg-surface);
    color: var(--text-secondary);
    border: 1px solid var(--border-color);
  }

  .dialog-btn-cancel:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .dialog-btn-primary {
    background: var(--accent);
    color: var(--bg-primary);
  }

  .dialog-btn-primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }
</style>
