<script lang="ts">
  let {
    open,
    step = "fetch",
    detail = "",
    status = "running",
    error = null,
    onCancel,
    onClose,
    onUseExisting = null,
  }: {
    open: boolean;
    step: string;
    detail: string;
    status: "running" | "ok" | "error" | "cancelled";
    error: string | null;
    onCancel: () => void;
    onClose: () => void;
    onUseExisting?: (() => void) | null;
  } = $props();

  const isBranchExistsError = $derived(
    status === "error" && error != null && error.includes("[E1004]") && onUseExisting != null,
  );

  type StepId = "fetch" | "validate" | "paths" | "conflicts" | "create" | "deps";
  const STEPS: { id: StepId; label: string }[] = [
    { id: "fetch", label: "Fetching agent info" },
    { id: "validate", label: "Validating request" },
    { id: "paths", label: "Resolving paths" },
    { id: "conflicts", label: "Checking conflicts" },
    { id: "create", label: "Creating worktree" },
    { id: "deps", label: "Preparing runtime" },
  ];

  let startedAtMs: number = $state(0);
  let stepStartedAtMs: number = $state(0);
  let nowMs: number = $state(Date.now());

  // Reset timers when the modal opens.
  $effect(() => {
    if (!open) return;
    startedAtMs = Date.now();
    stepStartedAtMs = startedAtMs;
    nowMs = startedAtMs;
  });

  // Reset step timer when the step changes.
  $effect(() => {
    void step;
    if (!open) return;
    stepStartedAtMs = Date.now();
  });

  // Tick elapsed time while running.
  $effect(() => {
    if (!open || status !== "running") return;
    const timer = window.setInterval(() => {
      nowMs = Date.now();
    }, 200);
    return () => window.clearInterval(timer);
  });

  function stepIndex(id: string): number {
    return STEPS.findIndex((s) => s.id === id);
  }

  function markerFor(idx: number): string {
    if (status === "error") return idx === stepIndex(step) ? "[!]" : idx < stepIndex(step) ? "[x]" : "[ ]";
    if (status === "ok") return "[x]";
    return idx < stepIndex(step) ? "[x]" : idx === stepIndex(step) ? "[>]" : "[ ]";
  }

  function elapsedSeconds(ms: number): string {
    return (ms / 1000).toFixed(1);
  }

  function stepElapsedLabel(): string {
    if (status !== "running") return "";
    const ms = nowMs - stepStartedAtMs;
    if (ms < 3000) return "";
    return `${elapsedSeconds(ms)}s`;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      if (status === "running") {
        onCancel();
        return;
      }
      onClose();
    }
  }
</script>

{#if open}
  <div
    class="overlay modal-overlay modal-overlay-stacked"
    role="dialog"
    aria-modal="true"
    aria-label="Preparing Launch"
    tabindex="0"
    onkeydown={handleKeydown}
  >
    <div class="dialog modal-dialog-shell">
      <div class="header">
        <h2>Preparing Launch</h2>
        <button class="close-btn" onclick={onCancel} disabled={status !== "running"} aria-label="Close">&times;</button>
      </div>

      <div class="body mono">
        <div class="steps">
          {#each STEPS as s, idx (s.id)}
            <div class="row">
              <span class="mark">{markerFor(idx)}</span>
              <span class="text">{s.label}</span>
              {#if status === "running" && s.id === step && stepElapsedLabel()}
                <span class="elapsed">{stepElapsedLabel()}</span>
              {/if}
            </div>
          {/each}
        </div>

        {#if detail && status === "running"}
          <div class="detail">{detail}</div>
        {/if}

        {#if status === "ok"}
          <div class="summary">
            Completed in {elapsedSeconds(nowMs - startedAtMs)}s
          </div>
        {/if}

        {#if status === "error" && error}
          <div class="error">{error}</div>
        {/if}
      </div>

      <div class="footer">
        {#if status === "running"}
          <button class="secondary" onclick={onCancel}>Cancel (Esc)</button>
        {:else if isBranchExistsError}
          <button class="secondary" onclick={onClose}>Close</button>
          <button class="primary" onclick={onUseExisting}>Use Existing Branch</button>
        {:else}
          <button class="primary" onclick={onClose}>Close</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.6);
    z-index: var(--z-modal-stacked);
  }

  .dialog {
    width: min(640px, calc(100vw - 36px));
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 12px;
    overflow: hidden;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.55);
  }

  .header {
    padding: 12px 14px;
    border-bottom: 1px solid var(--border-color);
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: 0.2px;
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    border-radius: 4px;
    padding: 4px 8px;
    cursor: pointer;
    font-size: 20px;
    line-height: 1;
  }

  .close-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .close-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .close-btn:disabled:hover {
    color: var(--text-muted);
    background: none;
  }

  .body {
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono",
      "Courier New", monospace;
  }

  .steps {
    border: 1px solid var(--border-color);
    background: rgba(0, 0, 0, 0.14);
    border-radius: 10px;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .row {
    display: flex;
    gap: 10px;
    align-items: baseline;
  }

  .mark {
    width: 28px;
    color: var(--text-muted);
  }

  .text {
    flex: 1;
  }

  .elapsed {
    color: var(--text-muted);
    font-size: 11px;
  }

  .detail {
    font-size: 11px;
    color: var(--text-muted);
    white-space: pre-wrap;
  }

  .summary {
    font-size: 12px;
    color: var(--text-secondary);
  }

  .error {
    padding: 10px 12px;
    border: 1px solid rgba(255, 90, 90, 0.35);
    background: rgba(255, 90, 90, 0.08);
    color: rgb(255, 160, 160);
    border-radius: 8px;
    font-size: 11px;
    line-height: 1.4;
    white-space: pre-wrap;
  }

  .footer {
    padding: 12px 14px;
    border-top: 1px solid var(--border-color);
    display: flex;
    justify-content: flex-end;
    gap: 10px;
  }

  button {
    padding: 10px 12px;
    border-radius: 10px;
    border: 1px solid var(--border-color);
    background: none;
    color: var(--text-primary);
    font-size: 12px;
    font-weight: 700;
    cursor: pointer;
  }

  button.primary {
    background: var(--accent);
    color: var(--bg-primary);
    border-color: transparent;
  }

  button.secondary:hover:not(:disabled) {
    border-color: var(--accent);
    background-color: var(--bg-surface);
  }

  button.primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }
</style>
