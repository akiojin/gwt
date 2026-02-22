<script lang="ts">
  import type { StructuredError } from "$lib/errorBus";
  import { maskSensitiveData } from "$lib/privacyMask";
  import { collectSystemInfo, collectRecentLogs } from "$lib/diagnostics";
  import { collectScreenText } from "$lib/screenCapture";
  import { invoke } from "$lib/tauriInvoke";
  import { openExternalUrl } from "$lib/openExternalUrl";
  import {
    generateBugReportBody,
    generateFeatureRequestBody,
    type BugReportData,
    type FeatureRequestData,
  } from "$lib/issueTemplate";

  interface ReportTarget {
    owner: string;
    repo: string;
    display: string;
  }

  interface CreateIssueResult {
    url: string;
    number: number;
  }

  let {
    open,
    mode,
    prefillError,
    projectPath = "",
    screenCaptureBranch = "",
    screenCaptureActiveTab = "",
    onclose,
  }: {
    open: boolean;
    mode: "bug" | "feature";
    prefillError?: StructuredError;
    projectPath?: string;
    screenCaptureBranch?: string;
    screenCaptureActiveTab?: string;
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
  let logsUnavailable = $state(false);
  let screenCaptureText = $state("");

  // Terminal text capture state
  let terminalCaptureText = $state("");
  let terminalCaptureLoading = $state(false);
  let terminalCaptureDone = $state(false);

  // Preview toggle
  let showPreview = $state(false);
  let previewMarkdown = $state("");

  // Error details toggle
  let showErrorDetails = $state(false);

  // Submit state
  let submitting = $state(false);
  let submitMessage = $state("");
  let submitSuccess = $state(false);
  let submitIssueUrl = $state("");
  let submitError = $state(false);
  let submitBodyMarkdown = $state("");

  // Repository target
  const GWT_TARGET: ReportTarget = { owner: "akiojin", repo: "gwt", display: "akiojin/gwt" };
  let targets = $state<ReportTarget[]>([GWT_TARGET]);
  let selectedTargetIndex = $state(0);

  let wasOpen = false;

  $effect(() => {
    if (!open) {
      wasOpen = false;
      return;
    }

    activeTab = mode;
    showPreview = false;
    if (wasOpen) return;

    wasOpen = true;
    submitMessage = "";
    submitSuccess = false;
    submitError = false;
    submitIssueUrl = "";
    submitBodyMarkdown = "";
  });

  $effect(() => {
    if (!open) return;
    void detectTarget(projectPath);
  });

  async function detectTarget(targetProjectPath: string) {
    if (!targetProjectPath) {
      targets = [GWT_TARGET];
      selectedTargetIndex = 0;
      return;
    }
    try {
      const detected = await invoke<ReportTarget>("detect_report_target", {
        projectPath: targetProjectPath,
      });
      if (detected.display === GWT_TARGET.display) {
        targets = [GWT_TARGET];
      } else {
        targets = [GWT_TARGET, detected];
      }
      selectedTargetIndex = 0;
    } catch {
      targets = [GWT_TARGET];
      selectedTargetIndex = 0;
    }
  }

  // Collect diagnostics when checkboxes change
  $effect(() => {
    if (open && includeSystemInfo && !systemInfoText) {
      collectSystemInfo().then((text) => {
        systemInfoText = text;
      });
    }
  });

  $effect(() => {
    if (open && includeLogs && !logsText && !logsUnavailable) {
      collectRecentLogs().then((text) => {
        if (text) {
          logsText = text;
        } else {
          logsUnavailable = true;
          includeLogs = false;
        }
      });
    }
  });

  $effect(() => {
    if (open && includeScreenCapture && !screenCaptureText) {
      const text = collectScreenText({
        branch: screenCaptureBranch,
        activeTab: screenCaptureActiveTab,
      });
      screenCaptureText = maskSensitiveData(text);
    }
  });

  function buildScreenCaptureText(): string | undefined {
    const parts: string[] = [];
    if (includeScreenCapture && screenCaptureText) {
      parts.push(screenCaptureText);
    }
    if (terminalCaptureDone && terminalCaptureText) {
      parts.push(terminalCaptureText);
    }
    return parts.length > 0 ? parts.join("\n\n") : undefined;
  }

  function generateBody(): string {
    if (activeTab === "bug") {
      const data: BugReportData = {
        title: bugTitle,
        stepsToReproduce,
        expectedResult,
        actualResult,
        systemInfo: includeSystemInfo ? systemInfoText : undefined,
        logs: includeLogs ? logsText : undefined,
        screenCapture: buildScreenCaptureText(),
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

  function generatePreview(): string {
    return generateBody();
  }

  async function handleCaptureTerminalText() {
    terminalCaptureLoading = true;
    terminalCaptureDone = false;
    try {
      const raw = await invoke<string>("capture_screen_text");
      terminalCaptureText = maskSensitiveData(raw);
      terminalCaptureDone = true;
    } catch {
      terminalCaptureText = "(Failed to capture terminal text)";
      terminalCaptureDone = true;
    } finally {
      terminalCaptureLoading = false;
    }
  }

  function handlePreviewToggle() {
    showPreview = !showPreview;
    if (showPreview) {
      previewMarkdown = generatePreview();
    }
  }

  async function handleSubmit() {
    const title = activeTab === "bug" ? bugTitle : featureTitle;
    if (!title.trim()) {
      submitMessage = "Please enter a title.";
      return;
    }

    const target = targets[selectedTargetIndex] ?? GWT_TARGET;
    const body = showPreview ? previewMarkdown : generateBody();
    const labels = activeTab === "bug" ? ["bug"] : ["enhancement"];

    submitting = true;
    submitMessage = "";
    submitSuccess = false;
    submitError = false;
    submitIssueUrl = "";
    submitBodyMarkdown = body;

    try {
      const result = await invoke<CreateIssueResult>("create_github_issue", {
        owner: target.owner,
        repo: target.repo,
        title,
        body,
        labels,
      });
      submitSuccess = true;
      submitIssueUrl = result.url;
      submitMessage = `Issue #${result.number} created successfully.`;
    } catch {
      submitError = true;
      submitMessage = "Failed to create issue via GitHub CLI. You can copy the report or open it in your browser instead.";
    } finally {
      submitting = false;
    }
  }

  async function handleCopyToClipboard() {
    try {
      await navigator.clipboard.writeText(submitBodyMarkdown);
      submitMessage = "Copied to clipboard.";
    } catch {
      submitMessage = "Failed to copy to clipboard.";
    }
  }

  function handleOpenInBrowser() {
    const target = targets[selectedTargetIndex] ?? GWT_TARGET;
    const title = activeTab === "bug" ? bugTitle : featureTitle;
    const labels = activeTab === "bug" ? "bug" : "enhancement";
    const url = `https://github.com/${target.owner}/${target.repo}/issues/new?title=${encodeURIComponent(title)}&labels=${encodeURIComponent(labels)}`;
    openExternalUrl(url);
  }

  function handleCancel() {
    onclose();
  }

  function handleOverlayClick(e: MouseEvent) {
    if (e.target !== e.currentTarget) return;
    onclose();
  }

  function handleOverlayKeydown(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    e.preventDefault();
    onclose();
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="modal-overlay report-overlay"
    role="dialog"
    aria-modal="true"
    aria-label="Report"
    tabindex="0"
    onclick={handleOverlayClick}
    onkeydown={handleOverlayKeydown}
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="report-dialog modal-dialog-shell" onclick={(e) => e.stopPropagation()}>
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
    <div class="form-section">
      <label class="form-label" for="report-target">Repository</label>
      <select
        id="report-target"
        class="form-input"
        bind:value={selectedTargetIndex}
      >
        {#each targets as target, i}
          <option value={i}>{target.display}</option>
        {/each}
      </select>
    </div>

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
        <label class="checkbox-label" class:disabled-label={logsUnavailable}>
          <input type="checkbox" bind:checked={includeLogs} disabled={logsUnavailable} />
          Application Logs{logsUnavailable ? " (No logs available)" : ""}
        </label>
        <label class="checkbox-label">
          <input type="checkbox" bind:checked={includeScreenCapture} />
          Screen Capture (text)
        </label>
        <div class="capture-terminal-row">
          <button
            class="btn btn-capture"
            onclick={handleCaptureTerminalText}
            disabled={terminalCaptureLoading}
          >
            {#if terminalCaptureLoading}
              Capturing...
            {:else if terminalCaptureDone}
              Recapture Terminal Text
            {:else}
              Capture Terminal Text
            {/if}
          </button>
          {#if terminalCaptureDone}
            <span class="capture-status">
              Captured ({terminalCaptureText.length} chars)
            </span>
          {/if}
        </div>
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
        <textarea class="preview-content" bind:value={previewMarkdown} rows="10"></textarea>
      </div>
    {/if}

    {#if submitMessage}
      <div class="submit-message" class:submit-success={submitSuccess} class:submit-error={submitError}>
        <span>{submitMessage}</span>
        {#if submitSuccess && submitIssueUrl}
          <button class="link-btn" onclick={() => openExternalUrl(submitIssueUrl)}>
            Open Issue
          </button>
        {/if}
      </div>
    {/if}

    {#if submitError}
      <div class="fallback-actions">
        <button class="btn btn-secondary" onclick={handleCopyToClipboard}>
          Copy to Clipboard
        </button>
        <button class="btn btn-secondary" onclick={handleOpenInBrowser}>
          Open in Browser
        </button>
      </div>
    {/if}
  </div>

  <div class="dialog-footer">
    <button class="btn btn-secondary" onclick={handleCancel}>Cancel</button>
    <button class="btn btn-secondary" onclick={handlePreviewToggle}>
      {showPreview ? "Hide Preview" : "Preview"}
    </button>
    <button class="btn btn-primary" onclick={handleSubmit} disabled={submitting || !(activeTab === "bug" ? bugTitle.trim() : featureTitle.trim())}>
      {submitting ? "Submitting..." : "Submit"}
    </button>
  </div>
    </div>
  </div>
{/if}

<style>
  .report-overlay {
    z-index: 1000;
  }

  .report-dialog {
    padding: 0;
    max-width: 680px;
    width: min(680px, 94vw);
    height: min(820px, 92vh);
    min-height: min(560px, 92vh);
    max-height: 92vh;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    color: var(--text-primary);
    font-family: inherit;
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
    font-size: var(--ui-font-2xl, 20px);
    font-weight: 700;
    color: var(--text-primary);
    margin: 0;
    line-height: 1.3;
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
    padding: 8px 16px;
    color: var(--text-secondary);
    cursor: pointer;
    font-family: inherit;
    font-size: var(--ui-font-base, 14px);
    font-weight: 600;
    line-height: 1.35;
  }

  .tab-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .tab-btn.active {
    color: var(--text-primary);
    border-color: var(--accent);
    border-bottom-color: var(--bg-secondary);
    background: rgba(137, 180, 250, 0.18);
    margin-bottom: -1px;
    padding-bottom: 9px;
  }

  .dialog-body {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .form-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .form-label {
    font-size: var(--ui-font-base, 14px);
    color: var(--text-primary);
    font-weight: 600;
    line-height: 1.4;
  }

  .form-input {
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 8px 12px;
    color: var(--text-primary);
    font-family: inherit;
    font-size: var(--ui-font-lg, 14px);
    line-height: 1.45;
  }

  .form-input::placeholder {
    color: var(--text-muted);
    opacity: 0.9;
  }

  .form-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .form-textarea {
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 8px 12px;
    color: var(--text-primary);
    font-family: inherit;
    font-size: var(--ui-font-lg, 14px);
    line-height: 1.5;
    resize: vertical;
    min-height: 52px;
  }

  .form-textarea::placeholder {
    color: var(--text-muted);
    opacity: 0.9;
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
    color: var(--text-primary);
    font-family: inherit;
    font-size: var(--ui-font-base, 14px);
    font-weight: 600;
    line-height: 1.4;
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
    font-size: var(--ui-font-sm, 13px);
    color: var(--text-secondary);
    background: var(--bg-primary);
    padding: 2px 7px;
    border-radius: 4px;
    margin-left: auto;
  }

  .error-details {
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 10px 12px;
    margin-top: 4px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--ui-font-base, 14px);
  }

  .error-row {
    color: var(--text-primary);
    line-height: 1.45;
    word-break: break-word;
  }

  .error-key {
    color: var(--text-secondary);
    font-weight: 600;
  }

  .diagnostics-section {
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 12px 14px;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
    background: var(--bg-primary);
  }

  .diagnostics-section legend {
    font-size: var(--ui-font-base, 14px);
    color: var(--text-secondary);
    font-weight: 600;
    padding: 0 4px;
  }

  .checkbox-label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--ui-font-base, 14px);
    color: var(--text-primary);
    line-height: 1.4;
    cursor: pointer;
  }

  .checkbox-label input[type="checkbox"] {
    accent-color: var(--accent);
  }

  .capture-terminal-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 4px;
  }

  .btn-capture {
    padding: 4px 12px;
    border-radius: 5px;
    font-family: inherit;
    font-size: var(--ui-font-xs, 12px);
    cursor: pointer;
    border: 1px solid var(--border-color);
    background: var(--bg-surface);
    color: var(--text-primary);
  }

  .btn-capture:hover:not(:disabled) {
    background: var(--bg-hover);
  }

  .btn-capture:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .capture-status {
    font-size: var(--ui-font-sm, 13px);
    color: var(--text-secondary);
  }

  .preview-section {
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 10px 14px;
    background: var(--bg-primary);
  }

  .preview-section h3 {
    font-size: var(--ui-font-base, 14px);
    color: var(--text-secondary);
    font-weight: 600;
    margin: 0 0 8px;
  }

  .preview-content {
    font-family: monospace;
    font-size: var(--ui-font-sm, 13px);
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

  .preview-content:focus {
    outline: none;
    border-color: var(--accent);
  }

  .disabled-label {
    opacity: 0.5;
    cursor: default;
  }

  .submit-message {
    font-size: var(--ui-font-base, 14px);
    color: var(--text-primary);
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 8px 12px;
    text-align: center;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
  }

  .submit-success {
    border-color: var(--color-success, #2ea043);
    color: var(--color-success, #2ea043);
  }

  .submit-error {
    border-color: var(--color-danger, #da3633);
    color: var(--color-danger, #da3633);
  }

  .link-btn {
    background: none;
    border: none;
    color: var(--accent);
    font-family: inherit;
    font-size: var(--ui-font-base, 14px);
    cursor: pointer;
    text-decoration: underline;
    padding: 0;
  }

  .link-btn:hover {
    filter: brightness(1.2);
  }

  .fallback-actions {
    display: flex;
    gap: 8px;
    justify-content: center;
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
    font-size: var(--ui-font-base, 14px);
    font-weight: 600;
    cursor: pointer;
    border: 1px solid var(--border-color);
  }

  @media (max-height: 760px) {
    .report-dialog {
      width: min(640px, 96vw);
      height: 94vh;
      min-height: 0;
      max-height: 94vh;
    }

    .dialog-header {
      padding: 12px 16px 0;
    }

    .tab-bar {
      padding: 8px 16px 0;
    }

    .dialog-body {
      padding: 12px 16px;
      gap: 12px;
    }

    .dialog-footer {
      padding: 10px 16px 12px;
    }
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-secondary {
    background: var(--bg-surface);
    color: var(--text-primary);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--bg-hover);
  }

  .btn-primary {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }

  .btn-primary:hover:not(:disabled) {
    filter: brightness(1.1);
  }
</style>
