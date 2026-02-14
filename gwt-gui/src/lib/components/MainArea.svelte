<script lang="ts">
  import type { LaunchAgentRequest, Tab } from "../types";
  import type { TabDropPosition } from "../appTabs";
  import TerminalView from "../terminal/TerminalView.svelte";
  import AgentModePanel from "./AgentModePanel.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";
  import VersionHistoryPanel from "./VersionHistoryPanel.svelte";

  function isAgentTabWithPaneId(tab: Tab): tab is Tab & { paneId: string } {
    return (
      tab.type === "agent" &&
      typeof tab.paneId === "string" &&
      tab.paneId.length > 0
    );
  }

  function isTerminalTabWithPaneId(tab: Tab): tab is Tab & { paneId: string } {
    return (
      tab.type === "terminal" &&
      typeof tab.paneId === "string" &&
      tab.paneId.length > 0
    );
  }

  let {
    tabs,
    activeTabId,
    selectedBranch: _selectedBranch,
    projectPath,
    onLaunchAgent: _onLaunchAgent,
    onQuickLaunch: _onQuickLaunch,
    onTabSelect,
    onTabClose,
    onTabReorder,
  }: {
    tabs: Tab[];
    activeTabId: string;
    selectedBranch?: unknown;
    projectPath: string;
    onLaunchAgent?: () => void;
    onQuickLaunch?: (request: LaunchAgentRequest) => Promise<void>;
    onTabSelect: (tabId: string) => void;
    onTabClose: (tabId: string) => void;
    onTabReorder: (
      dragTabId: string,
      overTabId: string,
      position: TabDropPosition,
    ) => void;
  } = $props();

  let activeTab = $derived(tabs.find((t) => t.id === activeTabId));
  let agentTabs = $derived(tabs.filter(isAgentTabWithPaneId));
  let terminalTabs = $derived(tabs.filter(isTerminalTabWithPaneId));
  let showTerminalLayer = $derived(
    activeTab?.type === "agent" || activeTab?.type === "terminal",
  );
  let isPinnedTab = (tabType?: Tab["type"]) => tabType === "agentMode";
  let draggingTabId: string | null = $state(null);
  let pointerDrag:
    | {
        tabId: string;
        pointerId: number;
        startX: number;
        startY: number;
      }
    | null = null;
  let lastReorderSignature = "";

  function readDraggedTabId(event: DragEvent): string {
    if (draggingTabId) return draggingTabId;

    const appData =
      event.dataTransfer?.getData("application/x-gwt-tab-id") ?? "";
    if (appData.trim()) return appData.trim();
    const textData = event.dataTransfer?.getData("text/plain") ?? "";
    return textData.trim();
  }

  function resetDragState() {
    removeGlobalPointerListeners();
    draggingTabId = null;
    pointerDrag = null;
    lastReorderSignature = "";
  }

  function removeGlobalPointerListeners() {
    if (typeof window === "undefined") return;
    window.removeEventListener("pointermove", handleGlobalPointerMove);
    window.removeEventListener("pointerup", handleGlobalPointerUp);
    window.removeEventListener("pointercancel", handleGlobalPointerCancel);
  }

  function addGlobalPointerListeners() {
    if (typeof window === "undefined") return;
    removeGlobalPointerListeners();
    window.addEventListener("pointermove", handleGlobalPointerMove);
    window.addEventListener("pointerup", handleGlobalPointerUp);
    window.addEventListener("pointercancel", handleGlobalPointerCancel);
  }

  function handleTabDragStart(event: DragEvent, tabId: string) {
    draggingTabId = tabId;
    lastReorderSignature = "";
    if (!event.dataTransfer) return;
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("application/x-gwt-tab-id", tabId);
    event.dataTransfer.setData("text/plain", tabId);
  }

  function handleTabDragOver(event: DragEvent, overTabId: string) {
    const dragTabId = readDraggedTabId(event);
    if (!dragTabId || dragTabId === overTabId) return;

    event.preventDefault();
    if (event.dataTransfer) {
      event.dataTransfer.dropEffect = "move";
    }

    const target = event.currentTarget;
    if (!(target instanceof HTMLElement)) return;

    const rect = target.getBoundingClientRect();
    const position: TabDropPosition =
      event.clientX <= rect.left + rect.width / 2 ? "before" : "after";

    const signature = `${dragTabId}:${overTabId}:${position}`;
    if (signature === lastReorderSignature) return;
    lastReorderSignature = signature;
    onTabReorder(dragTabId, overTabId, position);
  }

  function handleTabDrop(event: DragEvent) {
    event.preventDefault();
    lastReorderSignature = "";
  }

  function handleTabDragEnd() {
    resetDragState();
  }

  function handleTabPointerDown(event: PointerEvent, tabId: string) {
    if (event.button !== 0) return;
    if (event.target instanceof Element && event.target.closest(".tab-close")) {
      return;
    }

    draggingTabId = tabId;
    pointerDrag = {
      tabId,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
    };
    lastReorderSignature = "";
    addGlobalPointerListeners();
  }

  function handleGlobalPointerMove(event: PointerEvent) {
    if (!pointerDrag || event.pointerId !== pointerDrag.pointerId) return;
    // Ignore micro jitter so simple clicks do not trigger reordering.
    if (
      Math.abs(event.clientX - pointerDrag.startX) < 3 &&
      Math.abs(event.clientY - pointerDrag.startY) < 3
    ) {
      return;
    }

    const fromPoint =
      typeof document !== "undefined" &&
      typeof document.elementFromPoint === "function"
        ? document
            .elementFromPoint(event.clientX, event.clientY)
            ?.closest<HTMLElement>(".tab[data-tab-id]")
        : null;
    const fromTarget =
      event.target instanceof Element
        ? event.target.closest<HTMLElement>(".tab[data-tab-id]")
        : null;
    const overTab = fromPoint ?? fromTarget ?? null;
    if (!overTab) return;

    const overTabId = overTab.dataset.tabId ?? "";
    if (!overTabId || overTabId === pointerDrag.tabId) return;

    const rect = overTab.getBoundingClientRect();
    const position: TabDropPosition =
      event.clientX <= rect.left + rect.width / 2 ? "before" : "after";
    const signature = `${pointerDrag.tabId}:${overTabId}:${position}`;
    if (signature === lastReorderSignature) return;

    lastReorderSignature = signature;
    onTabReorder(pointerDrag.tabId, overTabId, position);
  }

  function handleGlobalPointerUp(event: PointerEvent) {
    if (!pointerDrag || event.pointerId !== pointerDrag.pointerId) return;
    resetDragState();
  }

  function handleGlobalPointerCancel(event: PointerEvent) {
    if (!pointerDrag || event.pointerId !== pointerDrag.pointerId) return;
    resetDragState();
  }

  $effect(() => {
    return () => {
      removeGlobalPointerListeners();
    };
  });
</script>

<main class="main-area">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="tab-bar">
    {#each tabs as tab}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="tab"
        data-tab-id={tab.id}
        class:active={activeTabId === tab.id}
        class:dragging={draggingTabId === tab.id}
        draggable="false"
        onclick={() => onTabSelect(tab.id)}
        title={tab.type === "terminal" ? tab.cwd || "" : ""}
        onpointerdown={(e) => handleTabPointerDown(e, tab.id)}
        ondragstart={(e) => handleTabDragStart(e, tab.id)}
        ondragover={(e) => handleTabDragOver(e, tab.id)}
        ondrop={handleTabDrop}
        ondragend={handleTabDragEnd}
      >
        {#if tab.type === "agent"}
          <span
            class="tab-dot"
            class:claude={tab.agentId === "claude"}
            class:codex={tab.agentId === "codex"}
            class:gemini={tab.agentId === "gemini"}
            class:opencode={tab.agentId === "opencode"}
          ></span>
        {:else if tab.type === "terminal"}
          <span class="tab-dot terminal"></span>
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
      {#each terminalTabs as tab (tab.id)}
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
    user-select: none;
  }

  .tab:hover {
    color: var(--text-primary);
    background-color: var(--bg-hover);
  }

  .tab.dragging {
    opacity: 0.6;
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

  .tab-dot.claude {
    background-color: var(--yellow);
  }

  .tab-dot.codex {
    background-color: var(--cyan);
  }

  .tab-dot.gemini {
    background-color: var(--magenta);
  }

  .tab-dot.opencode {
    background-color: var(--green);
  }

  .tab-dot.terminal {
    background-color: var(--text-muted);
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
