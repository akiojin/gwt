<script lang="ts">
  import { isProfilingEnabled } from "$lib/profiling.svelte";

  let {
    projectPath,
    currentBranch = "",
    terminalCount = 0,
    osEnvReady = true,
    voiceInputEnabled = false,
    voiceInputListening = false,
    voiceInputPreparing = false,
    voiceInputSupported = true,
    voiceInputAvailable = false,
    voiceInputAvailabilityReason = null,
    voiceInputError = null,
  }: {
    projectPath: string;
    currentBranch?: string;
    terminalCount?: number;
    osEnvReady?: boolean;
    voiceInputEnabled?: boolean;
    voiceInputListening?: boolean;
    voiceInputPreparing?: boolean;
    voiceInputSupported?: boolean;
    voiceInputAvailable?: boolean;
    voiceInputAvailabilityReason?: string | null;
    voiceInputError?: string | null;
  } = $props();

  function voiceStatusClass(): string {
    if (!voiceInputSupported) return "bad";
    if (!voiceInputAvailable) return "bad";
    if (!voiceInputEnabled) return "muted";
    if (voiceInputError) return "warn";
    if (voiceInputPreparing) return "warn";
    if (voiceInputListening) return "ok";
    return "warn";
  }

  function voiceStatusText(): string {
    if (!voiceInputSupported) return "Voice: backend unavailable";
    if (!voiceInputAvailable) return "Voice: unavailable";
    if (!voiceInputEnabled) return "Voice: off";
    if (voiceInputError) return "Voice: error";
    if (voiceInputPreparing) return "Voice: preparing";
    if (voiceInputListening) return "Voice: listening";
    return "Voice: idle";
  }

  $effect(() => {
    void projectPath;
    void osEnvReady;
  });
</script>

<footer class="statusbar">
  <span class="status-item">
    <span class="branch-indicator">*</span>
    {currentBranch || "---"}
  </span>
  {#if terminalCount > 0}
    <span class="status-item terminal-count">
      [{terminalCount} terminal{terminalCount !== 1 ? "s" : ""}]
    </span>
  {/if}
  {#if isProfilingEnabled()}
    <span class="status-item profiling-badge">PROFILING</span>
  {/if}
  {#if !osEnvReady}
    <span class="status-loading">Loading environment...</span>
  {/if}
  <span
    class={`status-item voice ${voiceStatusClass()}`}
    title={voiceInputError ?? voiceInputAvailabilityReason ?? ""}
  >
    {voiceStatusText()}
  </span>
  <span class="spacer"></span>
  <span class="status-item path">{projectPath}</span>
</footer>

<style>
  .statusbar {
    display: flex;
    align-items: center;
    height: var(--statusbar-height);
    background-color: var(--bg-surface);
    border-top: 1px solid var(--border-color);
    padding: 0 12px;
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    gap: 16px;
  }

  .branch-indicator {
    color: var(--green);
  }

  .terminal-count {
    color: var(--accent);
  }

  .voice {
    font-size: 10px;
  }

  .voice.ok {
    color: var(--green);
  }

  .voice.warn {
    color: var(--yellow);
  }

  .voice.bad {
    color: var(--red);
  }

  .voice.muted {
    color: var(--text-muted);
  }

  .spacer {
    flex: 1;
  }

  .profiling-badge {
    background-color: var(--red, #e74c3c);
    color: #fff;
    padding: 0 6px;
    border-radius: 3px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.5px;
  }

  .status-loading {
    color: var(--text-muted);
    font-style: italic;
  }

  .path {
    font-family: monospace;
    font-size: var(--ui-font-xs);
  }
</style>
