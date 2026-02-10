<script lang="ts">
  import type { MigrationFinishedPayload, MigrationProgressPayload } from "../types";

  let {
    open,
    sourceRoot,
    onCompleted,
    onDismiss,
  }: {
    open: boolean;
    sourceRoot: string;
    onCompleted: (projectPath: string) => void;
    onDismiss?: () => void;
  } = $props();

  let jobId: string = $state("");
  let running: boolean = $state(false);
  let stage: string = $state("pending");
  let current: number | null = $state(null);
  let total: number | null = $state(null);
  let error: string | null = $state(null);

  const STEPS: { id: string; label: string }[] = [
    { id: "validating", label: "Validating prerequisites" },
    { id: "backingUp", label: "Creating backup" },
    { id: "creatingBareRepo", label: "Creating bare repository" },
    { id: "migratingWorktrees", label: "Migrating worktrees" },
    { id: "cleaningUp", label: "Cleaning up" },
    { id: "completed", label: "Completed" },
  ];

  function stepIndex(id: string): number {
    return STEPS.findIndex((s) => s.id === id);
  }

  function activeIndex(): number {
    const idx = stepIndex(stage);
    return idx >= 0 ? idx : 0;
  }

  function markerFor(idx: number): string {
    if (error) return idx < activeIndex() ? "[x]" : idx === activeIndex() ? "[!]" : "[ ]";
    if (running) return idx < activeIndex() ? "[x]" : idx === activeIndex() ? "[>]" : "[ ]";
    if (stage === "completed") return "[x]";
    return idx < activeIndex() ? "[x]" : "[ ]";
  }

  function labelFor(step: { id: string; label: string }): string {
    if (step.id === "migratingWorktrees" && current && total) {
      return `${step.label} (${current}/${total})`;
    }
    return step.label;
  }

  function reset() {
    jobId = "";
    running = false;
    stage = "pending";
    current = null;
    total = null;
    error = null;
  }

  $effect(() => {
    void open;
    if (!open) {
      reset();
    }
  });

  $effect(() => {
    let unlistenProgress: null | (() => void) = null;
    let unlistenFinished: null | (() => void) = null;
    let cancelled = false;

    if (!open) return;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");

        unlistenProgress = await listen<MigrationProgressPayload>(
          "migration-progress",
          (event) => {
            if (!jobId || event.payload.jobId !== jobId) return;
            stage = event.payload.state || stage;
            current =
              typeof event.payload.current === "number" ? event.payload.current : null;
            total = typeof event.payload.total === "number" ? event.payload.total : null;
          }
        );

        unlistenFinished = await listen<MigrationFinishedPayload>(
          "migration-finished",
          (event) => {
            if (!jobId || event.payload.jobId !== jobId) return;
            running = false;
            if (event.payload.ok && event.payload.projectPath) {
              stage = "completed";
              onCompleted(event.payload.projectPath);
              return;
            }
            error = event.payload.error || "Migration failed.";
          }
        );
      } catch (e) {
        if (cancelled) return;
        error = "Failed to subscribe to migration events.";
      }
    })();

    return () => {
      cancelled = true;
      if (unlistenProgress) unlistenProgress();
      if (unlistenFinished) unlistenFinished();
    };
  });

  async function startMigration() {
    if (!open || running) return;
    if (!sourceRoot.trim()) {
      error = "Repository path is required.";
      return;
    }

    error = null;
    stage = "pending";
    current = null;
    total = null;
    running = true;
    jobId = "";

    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const id = await invoke<string>("start_migration_job", { path: sourceRoot });
      jobId = id;
    } catch (e) {
      running = false;
      error = "Failed to start migration.";
    }
  }

  async function quitApp() {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("quit_app");
    } catch {
      // Ignore: not available outside Tauri runtime.
      onDismiss?.();
    }
  }
</script>

{#if open}
  <div class="overlay" role="dialog" aria-modal="true" aria-label="Migration Required">
    <div class="dialog">
      <div class="header">
        <h2>Migration Required</h2>
      </div>

      <div class="body">
        <p class="desc">
          This repository must be migrated to a bare gwt project to continue.
        </p>
        <div class="path mono">{sourceRoot}</div>

        <div class="steps mono">
          {#each STEPS as s (s.id)}
            <div class="step-row">
              <span class="step-mark">{markerFor(stepIndex(s.id))}</span>
              <span class="step-text">{labelFor(s)}</span>
            </div>
          {/each}
        </div>

        {#if error}
          <div class="error mono">{error}</div>
        {/if}
      </div>

      <div class="footer">
        <button class="secondary" onclick={quitApp} disabled={running}>
          Quit
        </button>
        <button class="primary" onclick={startMigration} disabled={running}>
          {running ? "Migrating..." : error ? "Retry Migration" : "Migrate"}
        </button>
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
    width: min(680px, calc(100vw - 36px));
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

  .body {
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .desc {
    margin: 0;
    color: var(--text-secondary);
    font-size: 12px;
    line-height: 1.5;
  }

  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono",
      "Courier New", monospace;
  }

  .path {
    font-size: 11px;
    color: var(--text-muted);
    padding: 8px 10px;
    border-radius: 8px;
    border: 1px solid var(--border-color);
    background: var(--bg-primary);
    word-break: break-all;
  }

  .steps {
    font-size: 12px;
    color: var(--text-secondary);
    padding: 10px;
    border-radius: 10px;
    border: 1px solid var(--border-color);
    background: rgba(0, 0, 0, 0.14);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .step-row {
    display: flex;
    gap: 10px;
    align-items: baseline;
  }

  .step-mark {
    width: 28px;
    color: var(--text-muted);
  }

  .step-text {
    flex: 1;
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
    background: var(--bg-surface);
  }

  button.primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  button:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }
</style>
