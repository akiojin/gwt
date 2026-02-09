<script lang="ts">
  import type { Tab, BranchInfo } from "../types";
  import TerminalView from "../terminal/TerminalView.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";

  function isAgentTabWithPaneId(tab: Tab): tab is Tab & { paneId: string } {
    return tab.type === "agent" && typeof tab.paneId === "string" && tab.paneId.length > 0;
  }

  let {
    tabs,
    activeTabId,
    selectedBranch,
    showSettings = false,
    onLaunchAgent,
    onTabSelect,
    onTabClose,
    onSettingsClose,
  }: {
    tabs: Tab[];
    activeTabId: string;
    selectedBranch: BranchInfo | null;
    showSettings?: boolean;
    onLaunchAgent?: () => void;
    onTabSelect: (tabId: string) => void;
    onTabClose: (tabId: string) => void;
    onSettingsClose: () => void;
  } = $props();

  let activeTab = $derived(tabs.find((t) => t.id === activeTabId));
  let agentTabs = $derived(tabs.filter(isAgentTabWithPaneId));
  let showTerminalLayer = $derived(!showSettings && activeTab?.type === "agent");
</script>

<main class="main-area">
  <div class="tab-bar">
    {#each tabs as tab}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="tab"
        class:active={activeTabId === tab.id}
        onclick={() => onTabSelect(tab.id)}
      >
        {#if tab.type === "agent"}
          <span class="tab-dot"></span>
        {/if}
        <span class="tab-label">{tab.label}</span>
        {#if tab.type === "agent"}
          <button
            class="tab-close"
            onclick={(e) => {
              e.stopPropagation();
              onTabClose(tab.id);
            }}
          >
            x
          </button>
        {/if}
      </div>
    {/each}
  </div>
  <div class="tab-content">
    <div class="panel-layer" class:hidden={showTerminalLayer}>
      {#if showSettings}
        <SettingsPanel onClose={onSettingsClose} />
      {:else if activeTabId === "summary"}
        <div class="summary-content">
          {#if selectedBranch}
            <div class="branch-detail">
              <div class="branch-header">
                <h2>{selectedBranch.name}</h2>
                <button class="launch-btn" onclick={() => onLaunchAgent?.()}>
                  Launch Agent...
                </button>
              </div>
              <div class="detail-grid">
                <div class="detail-item">
                  <span class="detail-label">Commit</span>
                  <span class="detail-value mono">{selectedBranch.commit}</span>
                </div>
                <div class="detail-item">
                  <span class="detail-label">Status</span>
                  <span class="detail-value">
                    {selectedBranch.divergence_status}
                    {#if selectedBranch.ahead > 0}
                      (+{selectedBranch.ahead})
                    {/if}
                    {#if selectedBranch.behind > 0}
                      (-{selectedBranch.behind})
                    {/if}
                  </span>
                </div>
                <div class="detail-item">
                  <span class="detail-label">Current</span>
                  <span class="detail-value">
                    {selectedBranch.is_current ? "Yes" : "No"}
                  </span>
                </div>
              </div>
            </div>
          {:else}
            <div class="placeholder">
              <h2>Session Summary</h2>
              <p>Select a branch to view details.</p>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <div class="terminal-layer" class:hidden={!showTerminalLayer}>
      {#each agentTabs as tab (tab.id)}
        <div class="terminal-wrapper" class:active={activeTabId === tab.id}>
          <TerminalView paneId={tab.paneId} />
        </div>
      {/each}
    </div>
  </div>
</main>

<style>
  .main-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background-color: var(--bg-primary);
  }

  .tab-bar {
    display: flex;
    height: var(--tab-height);
    background-color: var(--bg-secondary);
    border-bottom: 1px solid var(--border-color);
    overflow-x: auto;
    scrollbar-width: none;
  }

  .tab-bar::-webkit-scrollbar {
    display: none;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 16px;
    background: none;
    border: none;
    border-right: 1px solid var(--border-color);
    color: var(--text-secondary);
    font-size: 12px;
    cursor: pointer;
    white-space: nowrap;
    font-family: inherit;
  }

  .tab:hover {
    color: var(--text-primary);
    background-color: var(--bg-hover);
  }

  .tab.active {
    color: var(--text-primary);
    background-color: var(--bg-primary);
    border-bottom: 2px solid var(--accent);
  }

  .tab-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background-color: var(--green);
    flex-shrink: 0;
  }

  .tab-label {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tab-close {
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: 11px;
    font-family: monospace;
    cursor: pointer;
    padding: 0 2px;
    line-height: 1;
    flex-shrink: 0;
  }

  .tab-close:hover {
    color: var(--red);
  }

  .tab-content {
    flex: 1;
    position: relative;
    overflow: hidden;
  }

  .panel-layer {
    position: absolute;
    inset: 0;
    overflow: auto;
    padding: 24px;
    z-index: 2;
  }

  .terminal-layer {
    position: absolute;
    inset: 0;
    overflow: hidden;
    z-index: 1;
  }

  .panel-layer.hidden,
  .terminal-layer.hidden {
    visibility: hidden;
    pointer-events: none;
  }

  .terminal-wrapper {
    position: absolute;
    inset: 0;
    visibility: hidden;
    pointer-events: none;
  }

  .terminal-wrapper.active {
    visibility: visible;
    pointer-events: auto;
  }

  .summary-content {
    height: 100%;
  }

  .placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-muted);
  }

  .placeholder h2 {
    font-size: 18px;
    font-weight: 500;
    margin-bottom: 8px;
    color: var(--text-secondary);
  }

  .branch-detail {
    max-width: 600px;
  }

  .branch-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 20px;
  }

  .branch-detail h2 {
    font-size: 20px;
    font-weight: 600;
    color: var(--text-primary);
    font-family: monospace;
  }

  .launch-btn {
    background: var(--accent);
    color: var(--bg-primary);
    border: none;
    border-radius: 8px;
    padding: 8px 12px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    white-space: nowrap;
  }

  .launch-btn:hover {
    background: var(--accent-hover);
  }

  .detail-grid {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .detail-item {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }

  .detail-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    min-width: 80px;
  }

  .detail-value {
    font-size: 13px;
    color: var(--text-primary);
  }

  .detail-value.mono {
    font-family: monospace;
  }
</style>
