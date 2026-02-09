<script lang="ts">
  let {
    projectPath,
    currentBranch = "",
    terminalCount = 0,
  }: {
    projectPath: string;
    currentBranch?: string;
    terminalCount?: number;
  } = $props();
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
    font-size: 11px;
    color: var(--text-muted);
    gap: 16px;
  }

  .branch-indicator {
    color: var(--green);
  }

  .terminal-count {
    color: var(--accent);
  }

  .spacer {
    flex: 1;
  }

  .path {
    font-family: monospace;
    font-size: 10px;
  }
</style>
