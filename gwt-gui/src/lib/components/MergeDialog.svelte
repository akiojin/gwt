<script lang="ts">
  let {
    projectPath,
    prNumber,
    prTitle,
    onClose,
    onMerged,
  }: {
    projectPath: string;
    prNumber: number;
    prTitle: string;
    onClose: () => void;
    onMerged: () => void;
  } = $props();

  type MergeMethod = "merge" | "squash" | "rebase";

  let method: MergeMethod = $state("squash");
  let deleteBranch: boolean = $state(true);
  let commitMsg: string = $state("");
  let merging: boolean = $state(false);
  let error: string | null = $state(null);
  let lastSyncedPrTitle = "";

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  async function handleMerge() {
    merging = true;
    error = null;
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      await invoke<string>("merge_pr", {
        projectPath,
        prNumber,
        method,
        deleteBranch,
        commitMsg: commitMsg.trim() || undefined,
      });
      onMerged();
    } catch (err) {
      error = toErrorMessage(err);
    } finally {
      merging = false;
    }
  }

  function handleBackdropClick(e: MouseEvent) {
    if (e.target === e.currentTarget) {
      onClose();
    }
  }

  $effect(() => {
    if (prTitle === lastSyncedPrTitle) return;
    commitMsg = prTitle;
    lastSyncedPrTitle = prTitle;
  });
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="dialog-backdrop" onclick={handleBackdropClick}>
  <div class="dialog">
    <h3 class="dialog-title">Merge PR #{prNumber}</h3>
    <p class="dialog-subtitle">{prTitle}</p>

    {#if error}
      <div class="dialog-error">{error}</div>
    {/if}

    <div class="dialog-field">
      <!-- svelte-ignore a11y_label_has_associated_control -->
      <label class="dialog-label">Merge Method</label>
      <div class="dialog-radio-group">
        <label class="dialog-radio">
          <input type="radio" bind:group={method} value="merge" />
          Merge commit
        </label>
        <label class="dialog-radio">
          <input type="radio" bind:group={method} value="squash" />
          Squash and merge
        </label>
        <label class="dialog-radio">
          <input type="radio" bind:group={method} value="rebase" />
          Rebase and merge
        </label>
      </div>
    </div>

    <div class="dialog-field">
      <label class="dialog-checkbox">
        <input type="checkbox" bind:checked={deleteBranch} />
        Delete branch after merge
      </label>
    </div>

    <div class="dialog-field">
      <label class="dialog-label" for="merge-commit-msg">Commit Message</label>
      <textarea
        id="merge-commit-msg"
        class="dialog-textarea"
        bind:value={commitMsg}
        rows="3"
      ></textarea>
    </div>

    <div class="dialog-actions">
      <button
        class="dialog-btn dialog-btn-cancel"
        onclick={onClose}
        disabled={merging}
      >
        Cancel
      </button>
      <button
        class="dialog-btn dialog-btn-primary"
        onclick={handleMerge}
        disabled={merging}
      >
        {merging ? "Merging..." : "Merge"}
      </button>
    </div>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-modal-base);
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

  .dialog-checkbox {
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
    background: var(--green, #a6e3a1);
    color: var(--bg-primary);
  }

  .dialog-btn-primary:hover:not(:disabled) {
    opacity: 0.9;
  }
</style>
