<script lang="ts">
  import type { Tab } from "../types";
  import TerminalView from "../terminal/TerminalView.svelte";
  import AgentModePanel from "./AgentModePanel.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";
  import VersionHistoryPanel from "./VersionHistoryPanel.svelte";

  function isAgentTabWithPaneId(tab: Tab): tab is Tab & { paneId: string } {
    return tab.type === "agent" && typeof tab.paneId === "string" && tab.paneId.length > 0;
  }

  let {
    tabs,
    activeTabId,
    projectPath,
    onTabSelect,
    onTabClose,
  }: {
    tabs: Tab[];
    activeTabId: string;
    projectPath: string;
    onTabSelect: (tabId: string) => void;
    onTabClose: (tabId: string) => void;
  } = $props();

  let activeTab = $derived(tabs.find((t) => t.id === activeTabId));
  let agentTabs = $derived(tabs.filter(isAgentTabWithPaneId));
  let showTerminalLayer = $derived(activeTab?.type === "agent");
  let isPinnedTab = (tabType?: Tab["type"]) => tabType === "agentMode";
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
        {#if !isPinnedTab(tab.type)}
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
      {#if activeTab?.type === "settings"}
        <SettingsPanel onClose={() => onTabClose(activeTabId)} />
      {:else if activeTab?.type === "versionHistory"}
        <VersionHistoryPanel {projectPath} />
      {:else if activeTab?.type === "agentMode"}
        <AgentModePanel />
      {:else}
        <div class="placeholder">
          <h2>Select a tab</h2>
        </div>
      {/if}
    </div>

    <div class="terminal-layer" class:hidden={!showTerminalLayer}>
      {#each agentTabs as tab (tab.id)}
        <div class="terminal-wrapper" class:active={activeTabId === tab.id}>
          <TerminalView paneId={tab.paneId} active={activeTabId === tab.id} />
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
    font-size: var(--ui-font-md);
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
    font-size: var(--ui-font-sm);
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

  .placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-muted);
  }

  .placeholder h2 {
    font-size: var(--ui-font-2xl);
    font-weight: 500;
    color: var(--text-secondary);
  }
</style>
