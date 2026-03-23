<script lang="ts">
  let {
    showPreview,
    previewMarkdown = $bindable(""),
    submitMessage = "",
    submitSuccess = false,
    submitIssueUrl = "",
    submitError = false,
    onOpenIssue,
    onCopy,
    onOpenBrowser,
  }: {
    showPreview: boolean;
    previewMarkdown?: string;
    submitMessage?: string;
    submitSuccess?: boolean;
    submitIssueUrl?: string;
    submitError?: boolean;
    onOpenIssue: () => void;
    onCopy: () => void;
    onOpenBrowser: () => void;
  } = $props();
</script>

{#if showPreview}
  <div class="preview-section">
    <h3>Preview</h3>
    <textarea class="preview-content" bind:value={previewMarkdown} rows="10"></textarea>
  </div>
{/if}

{#if submitMessage}
  <div class="submit-message" class:submit-success={submitSuccess} class:submit-error={submitError}>
    <span>{submitMessage}</span>
    {#if submitSuccess && submitIssueUrl}
      <button class="link-btn" onclick={onOpenIssue}>Open Issue</button>
    {/if}
  </div>
{/if}

{#if submitError}
  <div class="fallback-actions">
    <button class="btn btn-secondary" onclick={onCopy}>Copy to Clipboard</button>
    <button class="btn btn-secondary" onclick={onOpenBrowser}>Open in Browser</button>
  </div>
{/if}

<style>
  .preview-section { border: 1px solid var(--border-color); border-radius: 6px; padding: 10px 14px; background: var(--bg-primary); }
  .preview-section h3 { font-size: var(--ui-font-base,14px); color: var(--text-secondary); font-weight: 600; margin: 0 0 8px; }
  .preview-content {
    font-family: monospace;
    font-size: var(--ui-font-sm,13px);
    color: var(--text-primary);
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 200px;
    overflow-y: auto;
    margin: 0;
    line-height: 1.55;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 6px 10px;
    resize: vertical;
    width: 100%;
    box-sizing: border-box;
  }
  .preview-content:focus { outline: none; border-color: var(--accent); }
  .submit-message { font-size: var(--ui-font-base,14px); color: var(--text-primary); background: var(--bg-primary); border: 1px solid var(--border-color); border-radius: 6px; padding: 8px 12px; text-align: center; display:flex; align-items:center; justify-content:center; gap:8px; }
  .submit-success { border-color: var(--color-success, #2ea043); color: var(--color-success, #2ea043); }
  .submit-error { border-color: var(--color-danger, #da3633); color: var(--color-danger, #da3633); }
  .link-btn { background:none; border:none; color: var(--accent); font: inherit; cursor:pointer; text-decoration: underline; padding:0; }
  .fallback-actions { display:flex; gap:8px; justify-content:center; }
  .btn { padding: 6px 16px; border-radius: 6px; font: inherit; font-weight: 600; cursor: pointer; border: 1px solid var(--border-color); }
  .btn-secondary { background: var(--bg-primary); color: var(--text-primary); }
</style>
