<script lang="ts">
  import type { BranchBrowserPanelConfig } from "../types";
  import Sidebar from "./Sidebar.svelte";

  let { config }: { config: BranchBrowserPanelConfig } = $props();
</script>

<div class="branch-browser-panel">
  <Sidebar
    projectPath={config.projectPath}
    refreshKey={config.refreshKey}
    widthPx={config.widthPx}
    minWidthPx={config.minWidthPx}
    maxWidthPx={config.maxWidthPx}
    mode={config.mode}
    onModeChange={config.onModeChange}
    selectedBranch={config.selectedBranch}
    currentBranch={config.currentBranch}
    agentTabBranches={config.agentTabBranches}
    activeAgentTabBranch={config.activeAgentTabBranch}
    appLanguage={config.appLanguage}
    onResize={config.onResize}
    onBranchSelect={config.onBranchSelect}
    onBranchActivate={config.onBranchActivate}
    onCleanupRequest={config.onCleanupRequest}
    onLaunchAgent={config.onLaunchAgent}
    onQuickLaunch={config.onQuickLaunch}
    onNewTerminal={config.onNewTerminal}
    onOpenDocsEditor={config.onOpenDocsEditor}
    onOpenCiLog={config.onOpenCiLog}
    onDisplayNameChanged={config.onDisplayNameChanged}
    embedded={true}
  />
</div>

<style>
  .branch-browser-panel {
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
</style>
