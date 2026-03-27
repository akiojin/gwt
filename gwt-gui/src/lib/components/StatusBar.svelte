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

  function shortenPath(path: string): string {
    const home = "/Users/";
    if (path.startsWith(home)) {
      const rest = path.slice(home.length);
      const slashIdx = rest.indexOf("/");
      if (slashIdx > 0) return "~" + rest.slice(slashIdx);
    }
    return path;
  }

  $effect(() => {
    void projectPath;
    void osEnvReady;
  });
</script>

<footer class="statusbar" role="contentinfo">
  <!-- Left side -->
  <div class="status-left">
    <span class="status-chip branch-chip">
      <span class="branch-dot"></span>
      {currentBranch || "---"}
    </span>

    {#if terminalCount > 0}
      <span class="status-chip terminal-chip">
        {terminalCount} terminal{terminalCount !== 1 ? "s" : ""}
      </span>
    {/if}

    {#if isProfilingEnabled()}
      <span class="status-chip profiling-chip">PROFILING</span>
    {/if}

    {#if !osEnvReady}
      <span class="status-chip loading-chip">Loading env...</span>
    {/if}
  </div>

  <!-- Right side -->
  <div class="status-right">
    <span
      class={`status-chip voice-chip voice-${voiceStatusClass()}`}
      title={voiceInputError ?? voiceInputAvailabilityReason ?? ""}
    >
      {#if voiceInputListening}
        <span class="voice-dot pulse"></span>
      {/if}
      {voiceStatusText()}
    </span>
    <span class="status-path" title={projectPath}>{shortenPath(projectPath)}</span>
  </div>
</footer>

<style>
  .statusbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: var(--statusbar-height);
    background: var(--bg-secondary);
    border-top: 1px solid var(--border-subtle);
    padding: 0 var(--space-3);
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    gap: var(--space-4);
    user-select: none;
  }

  .status-left,
  .status-right {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
  }

  .status-right {
    justify-content: flex-end;
  }

  .status-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    white-space: nowrap;
  }

  .branch-chip {
    color: var(--text-secondary);
    font-weight: var(--font-weight-medium);
  }

  .branch-dot {
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--green);
  }

  .terminal-chip {
    color: var(--accent);
  }

  .profiling-chip {
    background: var(--red-muted);
    color: var(--red);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
    font-size: 9px;
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  .loading-chip {
    color: var(--text-muted);
    font-style: italic;
  }

  .voice-chip {
    font-size: var(--ui-font-xs);
  }

  .voice-ok {
    color: var(--green);
  }

  .voice-warn {
    color: var(--yellow);
  }

  .voice-bad {
    color: var(--red);
  }

  .voice-muted {
    color: var(--text-muted);
  }

  .voice-dot {
    display: inline-block;
    width: 5px;
    height: 5px;
    border-radius: var(--radius-full);
    background: currentColor;
  }

  .voice-dot.pulse {
    animation: pulse-dot 1.2s ease-in-out infinite;
  }

  .status-path {
    font-family: var(--font-mono);
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 300px;
  }
</style>
