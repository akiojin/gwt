<script lang="ts">
  import type {
    BranchBrowserPanelConfig,
    GitHubIssueInfo,
    LaunchAgentRequest,
    BranchBrowserPanelState,
    Tab,
    WorktreeInfo,
  } from "../types";
  import type {
    AgentCanvasTileLayout,
    AgentCanvasViewport,
  } from "../agentCanvas";
  import type {
    TabDropPosition,
    TabGroupState,
    TabLayoutDropTarget,
    TabLayoutNode,
    TabSplitDirection,
  } from "../tabLayout";
  import { createInitialTabLayout } from "../tabLayout";
  import TabLayoutNodeView from "./TabLayoutNode.svelte";
  import TabGroupPane from "./TabGroupPane.svelte";

  const TAB_ID_MIME = "application/x-gwt-tab-id";

  let {
    tabs,
    groups = undefined,
    layoutRoot = undefined,
    activeGroupId = undefined,
    activeTabId = undefined,
    selectedBranch: _selectedBranch = undefined,
    projectPath,
    branchBrowserConfig = undefined,
    currentBranch = "",
    selectedCanvasSessionTabId = null,
    selectedCanvasTileId = null,
    canvasViewport = undefined,
    canvasTileLayouts = undefined,
    canvasWorktrees = [],
    selectedCanvasWorktreeBranch = null,
    onCanvasWorktreeSelect = () => {},
    branchBrowserState = undefined,
    disableSplit = false,
    onCanvasSessionSelect = () => {},
    onCanvasViewportChange = () => {},
    onCanvasTileLayoutsChange = () => {},
    onCanvasSelectedTileChange = () => {},
    onLaunchAgent,
    onQuickLaunch,
    onTabSelect,
    onTabClose,
    onTabReorder,
    onTabMoveToGroup = () => {},
    onTabSplitToGroupEdge = () => {},
    onSplitResize = () => {},
    onGroupFocus = () => {},
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
    tabs: Tab[];
    groups?: Record<string, TabGroupState> | undefined;
    layoutRoot?: TabLayoutNode | undefined;
    activeGroupId?: string | undefined;
    activeTabId?: string | undefined;
    selectedBranch?: unknown;
    projectPath: string;
    branchBrowserConfig?: BranchBrowserPanelConfig | undefined;
    currentBranch?: string;
    selectedCanvasSessionTabId?: string | null;
    selectedCanvasTileId?: string | null;
    canvasViewport?: AgentCanvasViewport | undefined;
    canvasTileLayouts?: Record<string, AgentCanvasTileLayout> | undefined;
    canvasWorktrees?: WorktreeInfo[];
    selectedCanvasWorktreeBranch?: string | null;
    onCanvasWorktreeSelect?: (branchName: string) => void;
    branchBrowserState?: BranchBrowserPanelState | undefined;
    disableSplit?: boolean;
    onCanvasSessionSelect?: (tabId: string) => void;
    onCanvasViewportChange?: (viewport: AgentCanvasViewport) => void;
    onCanvasTileLayoutsChange?: (
      layouts: Record<string, AgentCanvasTileLayout>,
    ) => void;
    onCanvasSelectedTileChange?: (tileId: string | null) => void;
    onLaunchAgent?: () => void;
    onQuickLaunch?: (request: LaunchAgentRequest) => Promise<void>;
    onTabSelect:
      | ((groupId: string, tabId: string) => void)
      | ((tabId: string) => void);
    onTabClose: (tabId: string) => void;
    onTabReorder:
      | ((
          groupId: string,
          dragTabId: string,
          overTabId: string,
          position: TabDropPosition,
        ) => void)
      | ((dragTabId: string, overTabId: string, position: TabDropPosition) => void);
    onTabMoveToGroup?: (
      dragTabId: string,
      targetGroupId: string,
      overTabId?: string | null,
      position?: TabDropPosition,
    ) => void;
    onTabSplitToGroupEdge?: (
      dragTabId: string,
      targetGroupId: string,
      direction: TabSplitDirection,
    ) => void;
    onSplitResize?: (splitId: string, primaryFraction: number) => void;
    onGroupFocus?: (groupId: string) => void;
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

  let fallbackLayout = $derived(
    createInitialTabLayout(tabs, activeTabId ?? tabs[0]?.id ?? null),
  );
  let resolvedGroups = $derived(groups ?? fallbackLayout.groups);
  let resolvedLayoutRoot = $derived(layoutRoot ?? fallbackLayout.root);
  let resolvedActiveGroupId = $derived(
    activeGroupId ?? fallbackLayout.activeGroupId,
  );
  let draggedTabId: string | null = $state(null);
  let dropTarget: TabLayoutDropTarget | null = $state(null);
  let lastReorderSignature = "";
  let tabsById = $derived(
    Object.fromEntries(tabs.map((tab) => [tab.id, tab])),
  );
  let flatGroup = $derived.by(() => {
    const preferred = resolvedGroups[resolvedActiveGroupId];
    if (preferred) return preferred;
    return Object.values(resolvedGroups)[0] ?? null;
  });

  function readDraggedTabId(event: DragEvent): string {
    if (draggedTabId) return draggedTabId;
    const appData = event.dataTransfer?.getData(TAB_ID_MIME) ?? "";
    if (appData.trim()) return appData.trim();
    const textData = event.dataTransfer?.getData("text/plain") ?? "";
    return textData.trim();
  }

  function clearDragState() {
    draggedTabId = null;
    dropTarget = null;
    lastReorderSignature = "";
  }

  function handleTabDragStart(tabId: string, event: DragEvent) {
    draggedTabId = tabId;
    if (!event.dataTransfer) return;
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData(TAB_ID_MIME, tabId);
    event.dataTransfer.setData("text/plain", tabId);
  }

  function handleTabDragEnd() {
    clearDragState();
  }

  function handleTabSplitAction(
    tabId: string,
    groupId: string,
    direction: TabSplitDirection,
  ) {
    onTabSplitToGroupEdge(tabId, groupId, direction);
  }

  function handleTabSelectForward(groupId: string, tabId: string) {
    if (onTabSelect.length >= 2) {
      (onTabSelect as (groupId: string, tabId: string) => void)(groupId, tabId);
    } else {
      (onTabSelect as (tabId: string) => void)(tabId);
    }
  }

  function handleTabDragOver(
    groupId: string,
    tabId: string,
    position: TabDropPosition,
    event: DragEvent,
  ) {
    const dragTabId = readDraggedTabId(event);
    if (!dragTabId || dragTabId === tabId) return;
    event.preventDefault();
    if (event.dataTransfer) {
      event.dataTransfer.dropEffect = "move";
    }
    const sourceGroup = Object.values(resolvedGroups).find((group) =>
      group.tabIds.includes(dragTabId),
    );
    if (sourceGroup?.id === groupId) {
      const signature = `${dragTabId}:${tabId}:${position}`;
      if (signature !== lastReorderSignature) {
        lastReorderSignature = signature;
        if (onTabReorder.length >= 4) {
          (onTabReorder as (
            groupId: string,
            dragTabId: string,
            overTabId: string,
            position: TabDropPosition,
          ) => void)(groupId, dragTabId, tabId, position);
        } else {
          (onTabReorder as (
            dragTabId: string,
            overTabId: string,
            position: TabDropPosition,
          ) => void)(dragTabId, tabId, position);
        }
      }
    }
    dropTarget = {
      kind: "tab",
      groupId,
      tabId,
      position,
    };
  }

  function handleTabDrop(
    groupId: string,
    tabId: string,
    position: TabDropPosition,
    event: DragEvent,
  ) {
    const dragTabId = readDraggedTabId(event);
    if (!dragTabId || dragTabId === tabId) {
      clearDragState();
      return;
    }
    event.preventDefault();
    const currentGroups = resolvedGroups;
    const sourceGroup = Object.values(currentGroups).find((group) =>
      group.tabIds.includes(dragTabId),
    );
    if (sourceGroup?.id === groupId) {
      if (onTabReorder.length >= 4) {
        (onTabReorder as (
          groupId: string,
          dragTabId: string,
          overTabId: string,
          position: TabDropPosition,
        ) => void)(groupId, dragTabId, tabId, position);
      } else {
        (onTabReorder as (
          dragTabId: string,
          overTabId: string,
          position: TabDropPosition,
        ) => void)(dragTabId, tabId, position);
      }
    } else {
      onTabMoveToGroup(dragTabId, groupId, tabId, position);
    }
    clearDragState();
  }

  function handleGroupDragOver(groupId: string, event: DragEvent) {
    const dragTabId = readDraggedTabId(event);
    if (!dragTabId) return;
    event.preventDefault();
    if (event.dataTransfer) {
      event.dataTransfer.dropEffect = "move";
    }
    dropTarget = {
      kind: "group",
      groupId,
    };
  }

  function handleGroupDrop(groupId: string, event: DragEvent) {
    const dragTabId = readDraggedTabId(event);
    if (!dragTabId) {
      clearDragState();
      return;
    }
    event.preventDefault();
    onTabMoveToGroup(dragTabId, groupId, null, "after");
    clearDragState();
  }

  function handleSplitDragOver(
    groupId: string,
    direction: TabSplitDirection,
    event: DragEvent,
  ) {
    const dragTabId = readDraggedTabId(event);
    if (!dragTabId) return;
    event.preventDefault();
    if (event.dataTransfer) {
      event.dataTransfer.dropEffect = "move";
    }
    dropTarget = {
      kind: "split",
      groupId,
      direction,
    };
  }

  function handleSplitDrop(
    groupId: string,
    direction: TabSplitDirection,
    event: DragEvent,
  ) {
    const dragTabId = readDraggedTabId(event);
    if (!dragTabId) {
      clearDragState();
      return;
    }
    event.preventDefault();
    onTabSplitToGroupEdge(dragTabId, groupId, direction);
    clearDragState();
  }
</script>

<main class="main-area" class:drag-active={draggedTabId !== null}>
  {#if disableSplit && flatGroup}
    {#key `${resolvedActiveGroupId}:${activeTabId}:${tabs.length}`}
      <TabGroupPane
        flatShell={true}
        group={flatGroup}
        {tabsById}
        activeGroupId={resolvedActiveGroupId}
        {projectPath}
        {branchBrowserConfig}
        {currentBranch}
        {selectedCanvasSessionTabId}
        {selectedCanvasTileId}
        {canvasViewport}
        {canvasTileLayouts}
        {canvasWorktrees}
        {selectedCanvasWorktreeBranch}
        {onCanvasWorktreeSelect}
        {branchBrowserState}
        {disableSplit}
        {onCanvasSessionSelect}
        {onCanvasViewportChange}
        {onCanvasTileLayoutsChange}
        {onCanvasSelectedTileChange}
        {draggedTabId}
        {dropTarget}
        {onGroupFocus}
        {onLaunchAgent}
        {onQuickLaunch}
        {onWorkOnIssue}
        {onSwitchToWorktree}
        {onIssueCountChange}
        {onOpenSettings}
        {voiceInputEnabled}
        {voiceInputListening}
        {voiceInputPreparing}
        {voiceInputSupported}
        {voiceInputAvailable}
        {voiceInputAvailabilityReason}
        {voiceInputError}
        onTabSelect={handleTabSelectForward}
        onTabClose={onTabClose}
        onTabSplitAction={handleTabSplitAction}
        onTabDragStart={handleTabDragStart}
        onTabDragEnd={handleTabDragEnd}
        onTabDragOver={handleTabDragOver}
        onTabDrop={handleTabDrop}
        onGroupDragOver={handleGroupDragOver}
        onGroupDrop={handleGroupDrop}
        onSplitDragOver={handleSplitDragOver}
        onSplitDrop={handleSplitDrop}
        onSplitResize={onSplitResize}
      />
    {/key}
  {:else}
    <TabLayoutNodeView
      node={resolvedLayoutRoot}
      groups={resolvedGroups}
      {tabsById}
      activeGroupId={resolvedActiveGroupId}
      {projectPath}
      {branchBrowserConfig}
      {currentBranch}
      {selectedCanvasSessionTabId}
      {selectedCanvasTileId}
      {canvasViewport}
      {canvasTileLayouts}
      {canvasWorktrees}
      {selectedCanvasWorktreeBranch}
      {onCanvasWorktreeSelect}
      {branchBrowserState}
      {disableSplit}
      {onCanvasSessionSelect}
      {onCanvasViewportChange}
      {onCanvasTileLayoutsChange}
      {onCanvasSelectedTileChange}
      {draggedTabId}
      {dropTarget}
      {onGroupFocus}
      {onLaunchAgent}
      {onQuickLaunch}
      {onWorkOnIssue}
      {onSwitchToWorktree}
      {onIssueCountChange}
      {onOpenSettings}
      {voiceInputEnabled}
      {voiceInputListening}
      {voiceInputPreparing}
      {voiceInputSupported}
      {voiceInputAvailable}
      {voiceInputAvailabilityReason}
      {voiceInputError}
      onTabSelect={handleTabSelectForward}
      onTabClose={onTabClose}
      onTabSplitAction={handleTabSplitAction}
      onTabDragStart={handleTabDragStart}
      onTabDragEnd={handleTabDragEnd}
      onTabDragOver={handleTabDragOver}
      onTabDrop={handleTabDrop}
      onGroupDragOver={handleGroupDragOver}
      onGroupDrop={handleGroupDrop}
      onSplitDragOver={handleSplitDragOver}
      onSplitDrop={handleSplitDrop}
      onSplitResize={onSplitResize}
    />
  {/if}
</main>

<style>
  .main-area {
    flex: 1;
    display: flex;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    background: var(--bg-primary);
  }

  .main-area.drag-active {
    user-select: none;
  }
</style>
