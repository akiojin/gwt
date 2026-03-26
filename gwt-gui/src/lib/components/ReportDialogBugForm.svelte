<script lang="ts">
  import type { StructuredError } from "$lib/errorBus";

  let {
    bugTitle = $bindable(""),
    stepsToReproduce = $bindable(""),
    expectedResult = $bindable(""),
    actualResult = $bindable(""),
    includeSystemInfo = $bindable(true),
    includeLogs = $bindable(false),
    logsUnavailable = false,
    includeScreenCapture = $bindable(false),
    terminalCaptureLoading = false,
    terminalCaptureDone = false,
    terminalCaptureText = "",
    showErrorDetails = $bindable(false),
    prefillError,
    onCaptureTerminalText,
  }: {
    bugTitle?: string;
    stepsToReproduce?: string;
    expectedResult?: string;
    actualResult?: string;
    includeSystemInfo?: boolean;
    includeLogs?: boolean;
    logsUnavailable?: boolean;
    includeScreenCapture?: boolean;
    terminalCaptureLoading?: boolean;
    terminalCaptureDone?: boolean;
    terminalCaptureText?: string;
    showErrorDetails?: boolean;
    prefillError?: StructuredError;
    onCaptureTerminalText: () => void;
  } = $props();
</script>

<div class="form-section">
  <label class="form-label" for="bug-title">Title</label>
  <input id="bug-title" class="form-input" type="text" bind:value={bugTitle} placeholder="Brief description of the bug" />
</div>

<div class="form-section">
  <label class="form-label" for="steps">Steps to Reproduce</label>
  <textarea id="steps" class="form-textarea" bind:value={stepsToReproduce} placeholder="1. Open the application&#10;2. Navigate to...&#10;3. Click on..." rows="3"></textarea>
</div>

<div class="form-section">
  <label class="form-label" for="expected">Expected Result</label>
  <textarea id="expected" class="form-textarea" bind:value={expectedResult} placeholder="What should have happened?" rows="2"></textarea>
</div>

<div class="form-section">
  <label class="form-label" for="actual">Actual Result</label>
  <textarea id="actual" class="form-textarea" bind:value={actualResult} placeholder="What actually happened?" rows="2"></textarea>
</div>

{#if prefillError}
  <div class="form-section">
    <button class="collapsible-header" onclick={() => (showErrorDetails = !showErrorDetails)}>
      <span class="collapse-arrow" class:expanded={showErrorDetails}>&#9654;</span>
      Error Details
      <span class="error-code">{prefillError.code}</span>
    </button>
    {#if showErrorDetails}
      <div class="error-details">
        <div class="error-row"><span class="error-key">Code:</span> {prefillError.code}</div>
        <div class="error-row"><span class="error-key">Severity:</span> {prefillError.severity}</div>
        <div class="error-row"><span class="error-key">Command:</span> {prefillError.command}</div>
        <div class="error-row"><span class="error-key">Category:</span> {prefillError.category}</div>
        <div class="error-row"><span class="error-key">Message:</span> {prefillError.message}</div>
        {#if prefillError.suggestions.length > 0}
          <div class="error-row"><span class="error-key">Suggestions:</span> {prefillError.suggestions.join(", ")}</div>
        {/if}
        <div class="error-row"><span class="error-key">Timestamp:</span> {prefillError.timestamp}</div>
      </div>
    {/if}
  </div>
{/if}

<fieldset class="diagnostics-section">
  <legend>Diagnostic Information</legend>
  <label class="checkbox-label">
    <input type="checkbox" bind:checked={includeSystemInfo} />
    System Info
  </label>
  <label class="checkbox-label" class:disabled-label={logsUnavailable}>
    <input type="checkbox" bind:checked={includeLogs} disabled={logsUnavailable} />
    Application Logs{logsUnavailable ? " (No logs available)" : ""}
  </label>
  <label class="checkbox-label">
    <input type="checkbox" bind:checked={includeScreenCapture} />
    Screen Capture (text)
  </label>
  <div class="capture-terminal-row">
    <button class="btn btn-capture" onclick={onCaptureTerminalText} disabled={terminalCaptureLoading}>
      {#if terminalCaptureLoading}
        Capturing...
      {:else if terminalCaptureDone}
        Recapture Terminal Text
      {:else}
        Capture Terminal Text
      {/if}
    </button>
    {#if terminalCaptureDone}
      <span class="capture-status">Captured ({terminalCaptureText.length} chars)</span>
    {/if}
  </div>
</fieldset>

<style>
  .form-section { display: flex; flex-direction: column; gap: 6px; }
  .form-label { font-size: var(--ui-font-base, 14px); color: var(--text-primary); font-weight: 600; line-height: 1.4; }
  .form-input, .form-textarea {
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 8px 12px;
    color: var(--text-primary);
    font-family: inherit;
    font-size: var(--ui-font-lg, 14px);
    line-height: 1.5;
  }
  .form-textarea { resize: vertical; min-height: 52px; }
  .form-input::placeholder, .form-textarea::placeholder { color: var(--text-muted); opacity: 0.9; }
  .form-input:focus, .form-textarea:focus { outline: none; border-color: var(--accent); }
  .collapsible-header { display:flex; align-items:center; gap:6px; background:none; border:none; color:var(--text-primary); font:inherit; font-weight:600; cursor:pointer; padding:4px 0; }
  .collapse-arrow { font-size: 10px; transition: transform 0.15s ease; display:inline-block; }
  .collapse-arrow.expanded { transform: rotate(90deg); }
  .error-code { font-family: monospace; font-size: var(--ui-font-sm,13px); color: var(--text-secondary); background: var(--bg-primary); padding:2px 7px; border-radius:4px; margin-left:auto; }
  .error-details { background: var(--bg-primary); border: 1px solid var(--border-color); border-radius: 6px; padding: 10px 12px; margin-top:4px; display:flex; flex-direction:column; gap:6px; font-size: var(--ui-font-base,14px); }
  .error-row { color: var(--text-primary); line-height: 1.45; word-break: break-word; }
  .error-key { color: var(--text-secondary); font-weight: 600; }
  .diagnostics-section { border: 1px solid var(--border-color); border-radius: 6px; padding: 12px 14px; display:flex; flex-direction:column; gap:10px; background: var(--bg-primary); }
  .diagnostics-section legend { font-size: var(--ui-font-base,14px); color: var(--text-secondary); font-weight: 600; padding:0 4px; }
  .checkbox-label { display:flex; align-items:center; gap:8px; font-size: var(--ui-font-base,14px); color: var(--text-primary); }
  .checkbox-label input[type="checkbox"] { accent-color: var(--accent); }
  .capture-terminal-row { display:flex; align-items:center; gap:10px; margin-top:4px; }
  .btn-capture { padding: 4px 12px; border-radius:5px; font: inherit; font-size: var(--ui-font-xs,12px); cursor:pointer; border:1px solid var(--border-color); background: var(--bg-surface); color: var(--text-primary); }
  .btn-capture:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-capture:disabled { opacity: .6; cursor: default; }
  .capture-status { font-size: var(--ui-font-sm,13px); color: var(--text-secondary); }
  .disabled-label { opacity: .5; cursor: default; }
</style>
