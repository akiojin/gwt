<script lang="ts">
  let { projectPath }: { projectPath: string } = $props();
</script>

<footer class="statusbar">
  <span class="status-item">
    <span class="branch-indicator">*</span> main
  </span>
  <span class="status-item">gwt v6.28.1</span>
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

  .spacer {
    flex: 1;
  }

  .path {
    font-family: monospace;
    font-size: 10px;
  }
</style>
