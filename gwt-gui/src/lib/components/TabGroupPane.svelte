<script lang="ts">
  import type {
    BranchBrowserPanelConfig,
    BranchBrowserPanelState,
    GitHubIssueInfo,
    LaunchAgentRequest,
    Tab,
    WorktreeInfo,
  } from "../types";
  import type {
    AgentCanvasCardLayout,
    AgentCanvasViewport,
  } from "../agentCanvas";
  import type {
    TabDropPosition,
    TabGroupState,
    TabLayoutDropTarget,
    TabSplitDirection,
  } from "../tabLayout";
  import TerminalView from "../terminal/TerminalView.svelte";
  import IssueListPanel from "./IssueListPanel.svelte";
  import IssueSpecPanel from "./IssueSpecPanel.svelte";
  import PrListPanel from "./PrListPanel.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";
  import VersionHistoryPanel from "./VersionHistoryPanel.svelte";
  import ProjectIndexPanel from "./ProjectIndexPanel.svelte";
  import AssistantPanel from "./AssistantPanel.svelte";
  import AgentCanvasPanel from "./AgentCanvasPanel.svelte";
  import BranchBrowserPanel from "./BranchBrowserPanel.svelte";

  function isAgentOrTerminal(tab: Tab | null | undefined): boolean {
    return tab?.type === "agent" || tab?.type === "terminal";
  }

  let {
    group,
    tabsById,
    activeGroupId,
    projectPath,
    branchBrowserConfig = undefined,
    currentBranch = "",
    flatShell = false,
    selectedCanvasSessionTabId = null,
    selectedCanvasCardId = null,
    canvasViewport = undefined,
    canvasCardLayouts = undefined,
    canvasWorktrees = [],
    selectedCanvasWorktreeBranch = null,
    onCanvasWorktreeSelect = () => {},
    branchBrowserState = undefined,
    disableSplit = false,
    onCanvasSessionSelect = () => {},
    onCanvasViewportChange = () => {},
    onCanvasCardLayoutsChange = () => {},
    onCanvasSelectedCardChange = () => {},
    draggedTabId = null,
    dropTarget = null,
    onGroupFocus,
    onTabSelect,
    onTabClose,
    onTabSplitAction,
    onTabDragStart,
    onTabDragEnd,
    onTabDragOver,
    onTabDrop,
    onGroupDragOver,
    onGroupDrop,
    onSplitDragOver,
    onSplitDrop,
    onSplitResize = () => {},
    onLaunchAgent: _onLaunchAgent,
    onQuickLaunch: _onQuickLaunch,
    onWorkOnIssue,
    onSwitchToWorktree,
    onIssueCountChange,
    onOpenSettings,
    voiceInputEnabled = false,
    voiceInputListening = false,
    voiceInputPreparing = false,
    voiceInputSupported = true,
    voiceInputAvailable = false,
    voiceInputAvailabilityReason = null,
    voiceInputError = null,
  }: {
    group: TabGroupState;
    tabsById: Record<string, Tab>;
    activeGroupId: string;
    projectPath: string;
    branchBrowserConfig?: BranchBrowserPanelConfig | undefined;
    currentBranch?: string;
    flatShell?: boolean;
    selectedCanvasSessionTabId?: string | null;
    selectedCanvasCardId?: string | null;
    canvasViewport?: AgentCanvasViewport | undefined;
    canvasCardLayouts?: Record<string, AgentCanvasCardLayout> | undefined;
    canvasWorktrees?: WorktreeInfo[];
    selectedCanvasWorktreeBranch?: string | null;
    onCanvasWorktreeSelect?: (branchName: string) => void;
    branchBrowserState?: BranchBrowserPanelState | undefined;
    disableSplit?: boolean;
    onCanvasSessionSelect?: (tabId: string) => void;
    onCanvasViewportChange?: (viewport: AgentCanvasViewport) => void;
    onCanvasCardLayoutsChange?: (
      layouts: Record<string, AgentCanvasCardLayout>,
    ) => void;
    onCanvasSelectedCardChange?: (cardId: string | null) => void;
    draggedTabId?: string | null;
    dropTarget?: TabLayoutDropTarget | null;
    onGroupFocus: (groupId: string) => void;
    onTabSelect: (groupId: string, tabId: string) => void;
    onTabClose: (tabId: string) => void;
    onTabSplitAction: (
      tabId: string,
      groupId: string,
      direction: TabSplitDirection,
    ) => void;
    onTabDragStart: (tabId: string, event: DragEvent) => void;
    onTabDragEnd: () => void;
    onTabDragOver: (
      groupId: string,
      tabId: string,
      position: TabDropPosition,
      event: DragEvent,
    ) => void;
    onTabDrop: (
      groupId: string,
      tabId: string,
      position: TabDropPosition,
      event: DragEvent,
    ) => void;
    onGroupDragOver: (groupId: string, event: DragEvent) => void;
    onGroupDrop: (groupId: string, event: DragEvent) => void;
    onSplitDragOver: (
      groupId: string,
      direction: TabSplitDirection,
      event: DragEvent,
    ) => void;
    onSplitDrop: (
      groupId: string,
      direction: TabSplitDirection,
      event: DragEvent,
    ) => void;
    onSplitResize?: (splitId: string, primaryFraction: number) => void;
    onLaunchAgent?: () => void;
    onQuickLaunch?: (request: LaunchAgentRequest) => Promise<void>;
    onWorkOnIssue?: (issue: GitHubIssueInfo) => void;
    onSwitchToWorktree?: (branchName: string) => void;
    onIssueCountChange?: (count: number) => void;
    onOpenSettings?: () => void;
    voiceInputEnabled?: boolean;
    voiceInputListening?: boolean;
    voiceInputPreparing?: boolean;
    voiceInputSupported?: boolean;
    voiceInputAvailable?: boolean;
    voiceInputAvailabilityReason?: string | null;
    voiceInputError?: string | null;
  } = $props();

  let groupTabs = $derived(
    group.tabIds.map((tabId) => tabsById[tabId]).filter(Boolean),
  );
  let visibleGroupTabs = $derived(
    groupTabs.filter(
      (tab) =>
        tab.type !== "assistant" &&
        tab.type !== "agent" &&
        tab.type !== "terminal",
    ),
  );
  let canvasSessionTabs = $derived(
    Object.values(tabsById).filter(
      (tab) => tab.type === "agent" || tab.type === "terminal",
    ),
  );
  let activeTab = $derived(
    group.activeTabId ? tabsById[group.activeTabId] : null,
  );
  let isActiveGroup = $derived(activeGroupId === group.id);
  let showDetachedTerminalPlaceholder = $derived(
    isAgentOrTerminal(activeTab) &&
      (!activeTab?.paneId || activeTab.paneId.length === 0),
  );

  function isPinnedTab(tab: Tab | null | undefined) {
    return tab?.type === "agentCanvas" || tab?.type === "branchBrowser";
  }

  function canSplitCurrentGroup(): boolean {
    return !disableSplit && group.tabIds.length > 1;
  }

  function handleSplitAction(
    menuEl: HTMLDetailsElement,
    tabId: string,
    direction: TabSplitDirection,
  ) {
    if (!canSplitCurrentGroup()) return;
    onTabSplitAction(tabId, group.id, direction);
    menuEl.open = false;
  }

  function isTabDropTarget(
    tabId: string,
    position: TabDropPosition,
  ): boolean {
    return (
      dropTarget?.kind === "tab" &&
      dropTarget.groupId === group.id &&
      dropTarget.tabId === tabId &&
      dropTarget.position === position
    );
  }

  function isGroupDropTarget(): boolean {
    return dropTarget?.kind === "group" && dropTarget.groupId === group.id;
  }

  function isSplitTarget(direction: TabSplitDirection): boolean {
    return (
      dropTarget?.kind === "split" &&
      dropTarget.groupId === group.id &&
      dropTarget.direction === direction
    );
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="group-pane"
  class:flat-shell={flatShell}
  role="group"
  class:active-group={isActiveGroup}
  onmousedown={() => onGroupFocus(group.id)}
>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="tab-bar"
    role="tablist"
    tabindex="-1"
    class:drop-target={isGroupDropTarget()}
    ondragover={(event) => onGroupDragOver(group.id, event)}
    ondrop={(event) => onGroupDrop(group.id, event)}
  >
    {#each visibleGroupTabs as tab (tab.id)}
      <div
        class="tab"
        role="tab"
        tabindex="0"
        aria-selected={group.activeTabId === tab.id}
        data-tab-id={tab.id}
        class:active={group.activeTabId === tab.id}
        class:dragging={draggedTabId === tab.id}
        class:drop-before={isTabDropTarget(tab.id, "before")}
        class:drop-after={isTabDropTarget(tab.id, "after")}
        draggable={!disableSplit}
        title={tab.type === "terminal" ? tab.cwd || "" : ""}
        onclick={() => onTabSelect(group.id, tab.id)}
        onkeydown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            onTabSelect(group.id, tab.id);
          }
        }}
        ondragstart={(event) => onTabDragStart(tab.id, event)}
        ondragend={onTabDragEnd}
        ondragover={(event) => {
          const target = event.currentTarget;
          if (!(target instanceof HTMLElement)) return;
          const rect = target.getBoundingClientRect();
          const position: TabDropPosition =
            event.clientX <= rect.left + rect.width / 2 ? "before" : "after";
          onTabDragOver(group.id, tab.id, position, event);
        }}
        ondrop={(event) => {
          const target = event.currentTarget;
          if (!(target instanceof HTMLElement)) return;
          const rect = target.getBoundingClientRect();
          const position: TabDropPosition =
            event.clientX <= rect.left + rect.width / 2 ? "before" : "after";
          onTabDrop(group.id, tab.id, position, event);
        }}
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
        {#if tab.type === "agent"}
          <span class="tab-label tab-label-scroll" aria-label={tab.label}>
            <span class="tab-label-track">
              <span>{tab.label}</span>
              <span aria-hidden="true">{tab.label}</span>
            </span>
          </span>
        {:else}
          <span class="tab-label">{tab.label}</span>
        {/if}
        {#if !disableSplit}
          <details class="tab-actions">
            <summary
              class="tab-actions-toggle"
              onpointerdown={(event) => event.stopPropagation()}
            >
              ⋯
            </summary>
            <div class="tab-actions-menu">
              <button
                type="button"
                disabled={!canSplitCurrentGroup()}
                onclick={(event) => {
                  event.stopPropagation();
                  handleSplitAction(event.currentTarget.closest("details") as HTMLDetailsElement, tab.id, "left");
                }}
              >
                Split Left
              </button>
              <button
                type="button"
                disabled={!canSplitCurrentGroup()}
                onclick={(event) => {
                  event.stopPropagation();
                  handleSplitAction(event.currentTarget.closest("details") as HTMLDetailsElement, tab.id, "right");
                }}
              >
                Split Right
              </button>
              <button
                type="button"
                disabled={!canSplitCurrentGroup()}
                onclick={(event) => {
                  event.stopPropagation();
                  handleSplitAction(event.currentTarget.closest("details") as HTMLDetailsElement, tab.id, "up");
                }}
              >
                Split Up
              </button>
              <button
                type="button"
                disabled={!canSplitCurrentGroup()}
                onclick={(event) => {
                  event.stopPropagation();
                  handleSplitAction(event.currentTarget.closest("details") as HTMLDetailsElement, tab.id, "down");
                }}
              >
                Split Down
              </button>
            </div>
          </details>
        {/if}
        {#if !isPinnedTab(tab)}
          <button
            class="tab-close"
            type="button"
            onpointerdown={(event) => event.stopPropagation()}
            onclick={(event) => {
              event.stopPropagation();
              onTabClose(tab.id);
            }}
          >
            x
          </button>
        {/if}
      </div>
    {/each}
  </div>

  <div class="group-content" class:drag-active={draggedTabId !== null}>
    {#if !disableSplit}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="split-target split-target-top"
        role="presentation"
        class:active={isSplitTarget("up")}
        ondragover={(event) => onSplitDragOver(group.id, "up", event)}
        ondrop={(event) => onSplitDrop(group.id, "up", event)}
      ></div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="split-target split-target-right"
        role="presentation"
        class:active={isSplitTarget("right")}
        ondragover={(event) => onSplitDragOver(group.id, "right", event)}
        ondrop={(event) => onSplitDrop(group.id, "right", event)}
      ></div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="split-target split-target-bottom"
        role="presentation"
        class:active={isSplitTarget("down")}
        ondragover={(event) => onSplitDragOver(group.id, "down", event)}
        ondrop={(event) => onSplitDrop(group.id, "down", event)}
      ></div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="split-target split-target-left"
        role="presentation"
        class:active={isSplitTarget("left")}
        ondragover={(event) => onSplitDragOver(group.id, "left", event)}
        ondrop={(event) => onSplitDrop(group.id, "left", event)}
      ></div>
    {/if}

    {#if visibleGroupTabs.length === 0}
      <div class="placeholder">
        <h2>Select a tab</h2>
      </div>
    {:else if showDetachedTerminalPlaceholder}
      <div class="placeholder panel-fallback">
        <h2>Agent starting...</h2>
        <p>Waiting for the backend terminal pane to attach.</p>
      </div>
    {:else}
      {#each groupTabs as tab (tab.id)}
        {#if !isAgentOrTerminal(tab)}
          <div class="panel-wrapper" class:active={group.activeTabId === tab.id}>
            {#if tab.type === "settings"}
              <SettingsPanel {projectPath} onClose={() => onTabClose(tab.id)} />
            {:else if tab.type === "versionHistory"}
              <VersionHistoryPanel {projectPath} />
            {:else if tab.type === "issueSpec"}
              <IssueSpecPanel
                projectPath={projectPath}
                issueNumber={tab.issueNumber ?? 0}
              />
            {:else if tab.type === "issues"}
              <IssueListPanel
                {projectPath}
                onWorkOnIssue={onWorkOnIssue ?? (() => {})}
                onSwitchToWorktree={onSwitchToWorktree ?? (() => {})}
                {onIssueCountChange}
              />
            {:else if tab.type === "prs"}
              <PrListPanel
                {projectPath}
                isActive={group.activeTabId === tab.id}
                onSwitchToWorktree={onSwitchToWorktree ?? (() => {})}
              />
            {:else if tab.type === "projectIndex"}
              <ProjectIndexPanel {projectPath} />
            {:else if tab.type === "agentCanvas"}
              <AgentCanvasPanel
                {projectPath}
                {currentBranch}
                tabs={canvasSessionTabs}
                worktrees={canvasWorktrees}
                selectedWorktreeBranch={selectedCanvasWorktreeBranch}
                onWorktreeSelect={onCanvasWorktreeSelect}
                selectedSessionTabId={selectedCanvasSessionTabId}
                persistedSelectedCardId={selectedCanvasCardId}
                persistedViewport={canvasViewport}
                persistedCardLayouts={canvasCardLayouts}
                onSessionSelect={onCanvasSessionSelect}
                onViewportChange={onCanvasViewportChange}
                onCardLayoutsChange={onCanvasCardLayoutsChange}
                onSelectedCardChange={onCanvasSelectedCardChange}
                onOpenSettings={onOpenSettings ?? (() => {})}
                {voiceInputEnabled}
                {voiceInputListening}
                {voiceInputPreparing}
                {voiceInputSupported}
                {voiceInputAvailable}
                {voiceInputAvailabilityReason}
                {voiceInputError}
              />
            {:else if tab.type === "branchBrowser" && branchBrowserConfig}
              <BranchBrowserPanel config={{
                ...branchBrowserConfig,
                initialFilter: branchBrowserState?.filter,
                initialQuery: branchBrowserState?.query,
                selectedBranchName: branchBrowserState?.selectedBranchName ?? null,
              }} />
            {:else if tab.type === "assistant"}
              <AssistantPanel
                isActive={group.activeTabId === tab.id}
                {projectPath}
                onOpenSettings={onOpenSettings ?? (() => {})}
              />
            {:else}
              <div class="placeholder">
                <h2>Select a tab</h2>
              </div>
            {/if}
          </div>
        {:else if group.activeTabId === tab.id && tab.paneId}
          <div class="terminal-wrapper" class:active={group.activeTabId === tab.id}>
            <TerminalView
              paneId={tab.paneId ?? ""}
              active={group.activeTabId === tab.id}
              agentId={tab.type === "agent" ? tab.agentId ?? null : null}
              {voiceInputEnabled}
              {voiceInputListening}
              {voiceInputPreparing}
              {voiceInputSupported}
              {voiceInputAvailable}
              {voiceInputAvailabilityReason}
              {voiceInputError}
            />
          </div>
        {/if}
      {/each}
    {/if}
  </div>
</div>

<style>
  .group-pane {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    background: var(--bg-primary);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--border-color) 72%, transparent);
  }

  .group-pane.active-group {
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 36%, var(--border-color));
  }

  .group-pane.flat-shell,
  .group-pane.flat-shell.active-group {
    flex: 1 1 auto;
    width: 100%;
    align-self: stretch;
    box-shadow: none;
  }

  .tab-bar {
    display: flex;
    min-height: var(--tab-height);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border-color);
    overflow-x: auto;
    user-select: none;
  }

  .tab-bar.drop-target {
    background: color-mix(in srgb, var(--accent) 16%, var(--bg-secondary));
  }

  .tab {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    box-sizing: border-box;
    flex: 0 0 180px;
    min-width: 180px;
    max-width: 180px;
    padding: 0 14px;
    border-right: 1px solid var(--border-color);
    cursor: pointer;
    white-space: nowrap;
    user-select: none;
    background: transparent;
    color: var(--text-secondary);
  }

  .tab.active {
    background: var(--bg-primary);
    color: var(--text-primary);
    border-bottom: 2px solid var(--accent);
  }

  .tab.dragging {
    opacity: 0.45;
  }

  .tab.drop-before::before,
  .tab.drop-after::after {
    content: "";
    position: absolute;
    top: 4px;
    bottom: 4px;
    width: 3px;
    background: var(--accent);
    border-radius: 999px;
  }

  .tab.drop-before::before {
    left: 0;
  }

  .tab.drop-after::after {
    right: 0;
  }

  .tab-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--green);
  }

  .tab-dot.claude {
    background: var(--yellow);
  }

  .tab-dot.codex {
    background: var(--cyan);
  }

  .tab-dot.gemini {
    background: var(--magenta);
  }

  .tab-dot.opencode {
    background: var(--green);
  }

  .tab-dot.terminal {
    background: var(--text-muted);
  }

  .tab-label {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tab-label-scroll {
    display: inline-flex;
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }

  .tab-label-track {
    display: inline-flex;
    gap: 24px;
    min-width: max-content;
    animation: tab-label-marquee 12s linear infinite;
  }

  @keyframes tab-label-marquee {
    from {
      transform: translateX(0);
    }

    to {
      transform: translateX(calc(-50% - 12px));
    }
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

  .tab-actions {
    position: relative;
    flex-shrink: 0;
  }

  .tab-actions-toggle {
    list-style: none;
    cursor: pointer;
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
  }

  .tab-actions-toggle::-webkit-details-marker {
    display: none;
  }

  .tab-actions-menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    display: flex;
    flex-direction: column;
    min-width: 132px;
    padding: 6px;
    gap: 4px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    z-index: 20;
  }

  .tab-actions-menu button {
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    color: var(--text-primary);
    padding: 6px 8px;
    text-align: left;
    cursor: pointer;
  }

  .tab-actions-menu button:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .tab-close:hover {
    color: var(--red);
  }

  .group-content {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
    background: var(--bg-primary);
  }

  .panel-wrapper,
  .terminal-wrapper {
    position: absolute;
    inset: 0;
    visibility: hidden;
    pointer-events: none;
    overflow: auto;
    background: var(--bg-primary);
  }

  .panel-wrapper {
    padding: 0;
  }

  .panel-wrapper.active,
  .terminal-wrapper.active {
    visibility: visible;
    pointer-events: auto;
  }

  .split-target {
    position: absolute;
    z-index: 5;
    opacity: 0;
    pointer-events: none;
  }

  .group-content.drag-active .split-target {
    pointer-events: auto;
  }

  .split-target.active {
    opacity: 1;
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    outline: 2px solid var(--accent);
  }

  .split-target-top,
  .split-target-bottom {
    left: 18%;
    right: 18%;
    height: 22%;
  }

  .split-target-top {
    top: 0;
  }

  .split-target-bottom {
    bottom: 0;
  }

  .split-target-left,
  .split-target-right {
    top: 18%;
    bottom: 18%;
    width: 22%;
  }

  .split-target-left {
    left: 0;
  }

  .split-target-right {
    right: 0;
  }

  .placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-muted);
  }

  .panel-fallback {
    position: absolute;
    inset: 24px;
  }
</style>
