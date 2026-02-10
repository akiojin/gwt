<script lang="ts">
  import type { LaunchFinishedPayload, LaunchProgressPayload } from "../types";

  let {
    open,
    jobId,
    onSuccess,
    onClose,
  }: {
    open: boolean;
    jobId: string;
    onSuccess: (paneId: string) => void;
    onClose: () => void;
  } = $props();

  type StepId = "fetch" | "validate" | "paths" | "conflicts" | "create" | "deps";
  const STEPS: { id: StepId; label: string }[] = [
    { id: "fetch", label: "Fetching agent info" },
    { id: "validate", label: "Validating request" },
    { id: "paths", label: "Resolving paths" },
    { id: "conflicts", label: "Checking conflicts" },
    { id: "create", label: "Creating worktree" },
    { id: "deps", label: "Preparing runtime" },
  ];

  let step: StepId = $state("fetch");
  let detail: string = $state("");
  let startedAtMs: number = $state(0);
  let stepStartedAtMs: number = $state(0);
  let nowMs: number = $state(Date.now());
  let status: "running" | "ok" | "error" = $state("running");
  let error: string | null = $state(null);
  let finishedPaneId: string | null = $state(null);

  function reset() {
    step = "fetch";
    detail = "";
    startedAtMs = Date.now();
    stepStartedAtMs = startedAtMs;
    nowMs = startedAtMs;
    status = "running";
    error = null;
    finishedPaneId = null;
  }

  $effect(() => {
    void open;
    void jobId;
    if (!open) return;
    reset();
  });

  $effect(() => {
    let timer: number | null = null;
    if (!open) return;
    timer = window.setInterval(() => {
      nowMs = Date.now();
    }, 200);
    return () => {
      if (timer) window.clearInterval(timer);
    };
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

  async function cancel() {
    if (!jobId) {
      onClose();
      return;
    }
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("cancel_launch_job", { jobId });
    } catch {
      // Ignore: not available outside Tauri runtime.
    } finally {
      onClose();
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      void cancel();
    }
  }

  $effect(() => {
    let unlistenProgress: null | (() => void) = null;
    let unlistenFinished: null | (() => void) = null;
    let cancelled = false;
    if (!open) return;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");

        unlistenProgress = await listen<LaunchProgressPayload>(
          "launch-progress",
          (event) => {
            if (!jobId || event.payload.jobId !== jobId) return;
            if (status !== "running") return;
            const next = event.payload.step as StepId;
            if (stepIndex(next) >= 0 && next !== step) {
              step = next;
              stepStartedAtMs = Date.now();
            }
            detail = (event.payload.detail ?? "").toString();
          }
        );

        unlistenFinished = await listen<LaunchFinishedPayload>(
          "launch-finished",
          (event) => {
            if (!jobId || event.payload.jobId !== jobId) return;

            if (event.payload.status === "cancelled") {
              onClose();
              return;
            }

            if (event.payload.status === "ok" && event.payload.paneId) {
              status = "ok";
              finishedPaneId = event.payload.paneId;
              onSuccess(event.payload.paneId);
              window.setTimeout(() => onClose(), 2000);
              return;
            }

            status = "error";
            error = event.payload.error || "Launch failed.";
          }
        );
      } catch {
        if (cancelled) return;
        status = "error";
        error = "Failed to subscribe to launch events.";
      }
    })();

    return () => {
      cancelled = true;
      if (unlistenProgress) unlistenProgress();
      if (unlistenFinished) unlistenFinished();
    };
  });
</script>

{#if open}
  <div
    class="overlay"
    role="dialog"
    aria-modal="true"
    aria-label="Preparing Launch"
    tabindex="0"
    onkeydown={handleKeydown}
  >
    <div class="dialog">
      <div class="header">
        <h2>Preparing Launch</h2>
        <button class="close-btn" onclick={cancel} disabled={status !== "running"}>[x]</button>
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
          <button class="secondary" onclick={cancel}>Cancel (Esc)</button>
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
    z-index: 2000;
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
    border: 1px solid var(--border-color);
    background: none;
    color: var(--text-muted);
    border-radius: 10px;
    padding: 6px 10px;
    cursor: pointer;
    font-size: 12px;
    font-weight: 700;
  }

  .close-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
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
