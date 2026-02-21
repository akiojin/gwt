<script lang="ts">
  import type { StructuredError } from "$lib/errorBus";
  import { maskSensitiveData } from "$lib/privacyMask";
  import { collectSystemInfo, collectRecentLogs } from "$lib/diagnostics";
  import { collectScreenText } from "$lib/screenCapture";
  import {
    generateBugReportBody,
    generateFeatureRequestBody,
    type BugReportData,
    type FeatureRequestData,
  } from "$lib/issueTemplate";

  let {
    open,
    mode,
    prefillError,
    onclose,
  }: {
    open: boolean;
    mode: "bug" | "feature";
    prefillError?: StructuredError;
    onclose: () => void;
  } = $props();

  let activeTab = $state<"bug" | "feature">("bug");

  // Bug report fields
  let bugTitle = $state("");
  let stepsToReproduce = $state("");
  let expectedResult = $state("");
  let actualResult = $state("");

  // Feature request fields
  let featureTitle = $state("");
  let featureDescription = $state("");
  let useCase = $state("");
  let expectedBenefit = $state("");

  // Diagnostic checkboxes
  let includeSystemInfo = $state(true);
  let includeLogs = $state(false);
  let includeScreenCapture = $state(false);

  // Collected diagnostic data
  let systemInfoText = $state("");
  let logsText = $state("");
  let screenCaptureText = $state("");

  // Preview toggle
  let showPreview = $state(false);
  let previewMarkdown = $state("");

  // Error details toggle
  let showErrorDetails = $state(false);

  // Submit state
  let submitMessage = $state("");

  let dialogRef: HTMLDialogElement | undefined = $state();

  $effect(() => {
    if (open && dialogRef) {
      activeTab = mode;
      showPreview = false;
      submitMessage = "";
      dialogRef.showModal();
    } else if (!open && dialogRef?.open) {
      dialogRef.close();
    }
  });

  // Collect diagnostics when checkboxes change
  $effect(() => {
    if (open && includeSystemInfo && !systemInfoText) {
      collectSystemInfo().then((text) => {
        systemInfoText = text;
      });
    }
  });

  $effect(() => {
    if (open && includeLogs && !logsText) {
      collectRecentLogs().then((text) => {
        logsText = text;
      });
    }
  });

  $effect(() => {
    if (open && includeScreenCapture && !screenCaptureText) {
      const text = collectScreenText({
        branch: "",
        activeTab: "",
      });
      screenCaptureText = maskSensitiveData(text);
    }
  });

  function generatePreview(): string {
    if (activeTab === "bug") {
      const data: BugReportData = {
        title: bugTitle,
        stepsToReproduce,
        expectedResult,
        actualResult,
        systemInfo: includeSystemInfo ? systemInfoText : undefined,
        logs: includeLogs ? logsText : undefined,
        screenCapture: includeScreenCapture ? screenCaptureText : undefined,
        error: prefillError,
      };
      return maskSensitiveData(generateBugReportBody(data));
    } else {
      const data: FeatureRequestData = {
        title: featureTitle,
        description: featureDescription,
        useCase,
        expectedBenefit,
      };
      return maskSensitiveData(generateFeatureRequestBody(data));
    }
  }

  function handlePreviewToggle() {
    showPreview = !showPreview;
    if (showPreview) {
      previewMarkdown = generatePreview();
    }
  }

  function handleSubmit() {
    submitMessage = "Submission is not yet available. This feature will be enabled in a future update.";
  }

  function handleCancel() {
    onclose();
  }

  function handleDialogClose() {
    if (open) {
      onclose();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialogRef}
  class="report-dialog"
  onclose={handleDialogClose}
  onkeydown={(e) => { if (e.key === "Escape") handleCancel(); }}
>
  <div class="dialog-header">
    <h2>Report</h2>
    <button class="close-btn" onclick={handleCancel} aria-label="Close">&times;</button>
  </div>

  <div class="tab-bar">
    <button
      class="tab-btn"
      class:active={activeTab === "bug"}
      onclick={() => { activeTab = "bug"; showPreview = false; }}
    >
      Bug Report
    </button>
    <button
      class="tab-btn"
      class:active={activeTab === "feature"}
      onclick={() => { activeTab = "feature"; showPreview = false; }}
    >
      Feature Request
    </button>
  </div>

  <div class="dialog-body">
    {#if activeTab === "bug"}
      <div class="form-section">
        <label class="form-label" for="bug-title">Title</label>
        <input
          id="bug-title"
          class="form-input"
          type="text"
          bind:value={bugTitle}
          placeholder="Brief description of the bug"
        />
      </div>

      <div class="form-section">
        <label class="form-label" for="steps">Steps to Reproduce</label>
        <textarea
          id="steps"
          class="form-textarea"
          bind:value={stepsToReproduce}
          placeholder="1. Open the application&#10;2. Navigate to...&#10;3. Click on..."
          rows="3"
        ></textarea>
      </div>

      <div class="form-section">
        <label class="form-label" for="expected">Expected Result</label>
        <textarea
          id="expected"
          class="form-textarea"
          bind:value={expectedResult}
          placeholder="What should have happened?"
          rows="2"
        ></textarea>
      </div>

      <div class="form-section">
        <label class="form-label" for="actual">Actual Result</label>
        <textarea
          id="actual"
          class="form-textarea"
          bind:value={actualResult}
          placeholder="What actually happened?"
          rows="2"
        ></textarea>
      </div>

      {#if prefillError}
        <div class="form-section">
          <button
            class="collapsible-header"
            onclick={() => (showErrorDetails = !showErrorDetails)}
          >
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
        <label class="checkbox-label">
          <input type="checkbox" bind:checked={includeLogs} />
          Application Logs
        </label>
        <label class="checkbox-label">
          <input type="checkbox" bind:checked={includeScreenCapture} />
          Screen Capture (text)
        </label>
      </fieldset>
    {:else}
      <div class="form-section">
        <label class="form-label" for="feature-title">Title</label>
        <input
          id="feature-title"
          class="form-input"
          type="text"
          bind:value={featureTitle}
          placeholder="Feature summary"
        />
      </div>

      <div class="form-section">
        <label class="form-label" for="feature-desc">Description</label>
        <textarea
          id="feature-desc"
          class="form-textarea"
          bind:value={featureDescription}
          placeholder="Describe the feature you'd like"
          rows="3"
        ></textarea>
      </div>

      <div class="form-section">
        <label class="form-label" for="feature-usecase">Use Case</label>
        <textarea
          id="feature-usecase"
          class="form-textarea"
          bind:value={useCase}
          placeholder="What problem does this solve?"
          rows="2"
        ></textarea>
      </div>

      <div class="form-section">
        <label class="form-label" for="feature-benefit">Expected Benefit</label>
        <textarea
          id="feature-benefit"
          class="form-textarea"
          bind:value={expectedBenefit}
          placeholder="How would this improve your workflow?"
          rows="2"
        ></textarea>
      </div>
    {/if}

    {#if showPreview}
      <div class="preview-section">
        <h3>Preview</h3>
        <pre class="preview-content">{previewMarkdown}</pre>
      </div>
    {/if}

    {#if submitMessage}
      <div class="submit-message">{submitMessage}</div>
    {/if}
  </div>

  <div class="dialog-footer">
    <button class="btn btn-secondary" onclick={handleCancel}>Cancel</button>
    <button class="btn btn-secondary" onclick={handlePreviewToggle}>
      {showPreview ? "Hide Preview" : "Preview"}
    </button>
    <button class="btn btn-primary" onclick={handleSubmit}>Submit</button>
  </div>
</dialog>

<style>
  .report-dialog {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 12px;
    padding: 0;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4);
    max-width: 640px;
    width: min(640px, 92vw);
    max-height: 85vh;
    display: flex;
    flex-direction: column;
    color: var(--text-primary);
    font-family: inherit;
  }

  .report-dialog::backdrop {
    background: rgba(0, 0, 0, 0.6);
  }

  .report-dialog[open] {
    animation: dialog-fade-in 0.15s ease-out;
  }

  @keyframes dialog-fade-in {
    from {
      opacity: 0;
      transform: translateY(-8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px 0;
  }

  .dialog-header h2 {
    font-size: var(--ui-font-xl, 18px);
    font-weight: 700;
    color: var(--text-primary);
    margin: 0;
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: 20px;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 4px;
    line-height: 1;
  }

  .close-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .tab-bar {
    display: flex;
    gap: 4px;
    padding: 12px 20px 0;
    border-bottom: 1px solid var(--border-color);
  }

  .tab-btn {
    background: none;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: 6px 6px 0 0;
    padding: 6px 14px;
    color: var(--text-muted);
    cursor: pointer;
    font-family: inherit;
    font-size: var(--ui-font-sm, 13px);
  }

  .tab-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .tab-btn.active {
    color: var(--accent);
    border-color: var(--border-color);
    border-bottom-color: var(--bg-secondary);
    background: var(--bg-secondary);
    margin-bottom: -1px;
    padding-bottom: 7px;
  }

  .dialog-body {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .form-section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .form-label {
    font-size: var(--ui-font-sm, 13px);
    color: var(--text-secondary);
    font-weight: 500;
  }

  .form-input {
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 6px 10px;
    color: var(--text-primary);
    font-family: inherit;
    font-size: var(--ui-font-base, 14px);
  }

  .form-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .form-textarea {
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 6px 10px;
    color: var(--text-primary);
    font-family: inherit;
    font-size: var(--ui-font-base, 14px);
    resize: vertical;
    min-height: 40px;
  }

  .form-textarea:focus {
    outline: none;
    border-color: var(--accent);
  }

  .collapsible-header {
    display: flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: none;
    color: var(--text-secondary);
    font-family: inherit;
    font-size: var(--ui-font-sm, 13px);
    font-weight: 500;
    cursor: pointer;
    padding: 4px 0;
  }

  .collapsible-header:hover {
    color: var(--text-primary);
  }

  .collapse-arrow {
    font-size: 10px;
    transition: transform 0.15s ease;
    display: inline-block;
  }

  .collapse-arrow.expanded {
    transform: rotate(90deg);
  }

  .error-code {
    font-family: monospace;
    font-size: var(--ui-font-xs, 12px);
    color: var(--text-muted);
    background: var(--bg-primary);
    padding: 1px 6px;
    border-radius: 4px;
    margin-left: auto;
  }

  .error-details {
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 8px 12px;
    margin-top: 4px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--ui-font-sm, 13px);
  }

  .error-row {
    color: var(--text-secondary);
    word-break: break-word;
  }

  .error-key {
    color: var(--text-muted);
    font-weight: 600;
  }

  .diagnostics-section {
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 10px 14px;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .diagnostics-section legend {
    font-size: var(--ui-font-sm, 13px);
    color: var(--text-muted);
    font-weight: 500;
    padding: 0 4px;
  }

  .checkbox-label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--ui-font-sm, 13px);
    color: var(--text-secondary);
    cursor: pointer;
  }

  .checkbox-label input[type="checkbox"] {
    accent-color: var(--accent);
  }

  .preview-section {
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 10px 14px;
    background: var(--bg-primary);
  }

  .preview-section h3 {
    font-size: var(--ui-font-sm, 13px);
    color: var(--text-muted);
    font-weight: 600;
    margin: 0 0 8px;
  }

  .preview-content {
    font-family: monospace;
    font-size: var(--ui-font-xs, 12px);
    color: var(--text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 200px;
    overflow-y: auto;
    margin: 0;
    line-height: 1.5;
  }

  .submit-message {
    font-size: var(--ui-font-sm, 13px);
    color: var(--text-muted);
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 8px 12px;
    text-align: center;
  }

  .dialog-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 20px 16px;
    border-top: 1px solid var(--border-color);
  }

  .btn {
    padding: 6px 16px;
    border-radius: 6px;
    font-family: inherit;
    font-size: var(--ui-font-sm, 13px);
    cursor: pointer;
    border: 1px solid var(--border-color);
  }

  .btn-secondary {
    background: var(--bg-surface);
    color: var(--text-primary);
  }

  .btn-secondary:hover {
    background: var(--bg-hover);
  }

  .btn-primary {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }

  .btn-primary:hover {
    filter: brightness(1.1);
  }
</style>
