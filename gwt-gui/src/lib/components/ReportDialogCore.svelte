<script lang="ts">
  import type { StructuredError } from "$lib/errorBus";
  import { maskSensitiveData } from "$lib/privacyMask";
  import { collectSystemInfo, collectRecentLogs } from "$lib/diagnostics";
  import { collectScreenText } from "$lib/screenCapture";
  import { invoke } from "$lib/tauriInvoke";
  import { openExternalUrl } from "$lib/openExternalUrl";
  import {
    buildBrowserIssueUrlRuntime,
    buildScreenCaptureTextRuntime,
    generateReportBodyRuntime,
    normalizeDetectedTargetsRuntime,
    type ReportTarget,
  } from "$lib/reportDialogRuntime";
  import ReportDialogBugForm from "./ReportDialogBugForm.svelte";
  import ReportDialogFeatureForm from "./ReportDialogFeatureForm.svelte";
  import ReportDialogStatus from "./ReportDialogStatus.svelte";

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
    onsuccess,
  }: {
    open: boolean;
    mode: "bug" | "feature";
    prefillError?: StructuredError;
    projectPath?: string;
    screenCaptureBranch?: string;
    screenCaptureActiveTab?: string;
    onclose: () => void;
    onsuccess: (result: { url: string; number: number }) => void;
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

  function resetDialogState() {
    // Bug report fields
    bugTitle = "";
    stepsToReproduce = "";
    expectedResult = "";
    actualResult = "";

    // Feature request fields
    featureTitle = "";
    featureDescription = "";
    useCase = "";
    expectedBenefit = "";

    // Diagnostic checkboxes
    includeSystemInfo = true;
    includeLogs = false;
    includeScreenCapture = false;

    // Collected diagnostic data
    systemInfoText = "";
    logsText = "";
    logsUnavailable = false;
    screenCaptureText = "";

    // Terminal text capture state
    terminalCaptureText = "";
    terminalCaptureLoading = false;
    terminalCaptureDone = false;

    // Preview state
    showPreview = false;
    previewMarkdown = "";

    // Error details toggle
    showErrorDetails = false;

    // Submit state
    submitting = false;
    submitMessage = "";
    submitSuccess = false;
    submitIssueUrl = "";
    submitError = false;
    submitBodyMarkdown = "";

    // Repository target
    targets = [GWT_TARGET];
    selectedTargetIndex = 0;
  }

  $effect(() => {
    if (!open) {
      wasOpen = false;
      return;
    }

    if (!wasOpen) {
      wasOpen = true;
      resetDialogState();
    }

    activeTab = mode;
    showPreview = false;
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
      targets = normalizeDetectedTargetsRuntime(detected, GWT_TARGET);
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
    return buildScreenCaptureTextRuntime({
      includeScreenCapture,
      screenCaptureText,
      terminalCaptureDone,
      terminalCaptureText,
    });
  }

  function generateBody(): string {
    return generateReportBodyRuntime({
      activeTab,
      bugTitle,
      stepsToReproduce,
      expectedResult,
      actualResult,
      includeSystemInfo,
      systemInfoText,
      includeLogs,
      logsText,
      screenCaptureText: buildScreenCaptureText(),
      prefillError,
      featureTitle,
      featureDescription,
      useCase,
      expectedBenefit,
    });
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
      onsuccess({ url: result.url, number: result.number });
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
    const url = buildBrowserIssueUrlRuntime({
      target,
      activeTab,
      title,
    });
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
    class="modal-overlay report-overlay modal-overlay-report"
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
      <ReportDialogBugForm
        bind:bugTitle
        bind:stepsToReproduce
        bind:expectedResult
        bind:actualResult
        bind:includeSystemInfo
        bind:includeLogs
        {logsUnavailable}
        bind:includeScreenCapture
        {terminalCaptureLoading}
        {terminalCaptureDone}
        {terminalCaptureText}
        bind:showErrorDetails
        {prefillError}
        onCaptureTerminalText={handleCaptureTerminalText}
      />
    {:else}
      <ReportDialogFeatureForm
        bind:featureTitle
        bind:featureDescription
        bind:useCase
        bind:expectedBenefit
      />
    {/if}

    <ReportDialogStatus
      {showPreview}
      bind:previewMarkdown
      {submitMessage}
      {submitSuccess}
      {submitIssueUrl}
      {submitError}
      onOpenIssue={() => openExternalUrl(submitIssueUrl)}
      onCopy={handleCopyToClipboard}
      onOpenBrowser={handleOpenInBrowser}
    />
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
