<script lang="ts">
  import type { GitHubIssueInfo, LaunchAgentRequest, Tab } from "../types";
  import type {
    TabDropPosition,
    TabGroupState,
    TabLayoutDropTarget,
    TabLayoutNode,
    TabSplitDirection,
  } from "../tabLayout";
  import TabGroupPane from "./TabGroupPane.svelte";
  import SelfTabLayoutNode from "./TabLayoutNode.svelte";

  let {
    node,
    groups,
    tabsById,
    activeGroupId,
    projectPath,
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
    onSplitResize,
    onLaunchAgent,
    onQuickLaunch,
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
    node: TabLayoutNode;
    groups: Record<string, TabGroupState>;
    tabsById: Record<string, Tab>;
    activeGroupId: string;
    projectPath: string;
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
    onSplitResize: (splitId: string, primaryFraction: number) => void;
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

  let resizeContainer: HTMLDivElement | null = $state(null);

  function startResize(event: PointerEvent, splitId: string) {
    const container = resizeContainer;
    if (!container) return;
    const axis = node.type === "split" ? node.axis : "horizontal";

    const handlePointerMove = (moveEvent: PointerEvent) => {
      const rect = container.getBoundingClientRect();
      if (axis === "horizontal") {
        const primary = (moveEvent.clientX - rect.left) / rect.width;
        onSplitResize(splitId, primary);
      } else {
        const primary = (moveEvent.clientY - rect.top) / rect.height;
        onSplitResize(splitId, primary);
      }
    };

    const handlePointerUp = () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
      window.removeEventListener("pointercancel", handlePointerUp);
    };

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
    window.addEventListener("pointercancel", handlePointerUp);
    handlePointerMove(event);
  }
</script>

{#if node.type === "group"}
  {#if groups[node.groupId]}
    <TabGroupPane
      group={groups[node.groupId]}
      {tabsById}
      {activeGroupId}
      {projectPath}
      {draggedTabId}
      {dropTarget}
      {onGroupFocus}
      {onTabSelect}
      {onTabClose}
      {onTabSplitAction}
      {onTabDragStart}
      {onTabDragEnd}
      {onTabDragOver}
      {onTabDrop}
      {onGroupDragOver}
      {onGroupDrop}
      {onSplitDragOver}
      {onSplitDrop}
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
    />
  {/if}
{:else}
  <div
    bind:this={resizeContainer}
    class="split-node"
    class:vertical={node.axis === "vertical"}
  >
    <div class="split-child" style={`flex-basis: ${node.sizes[0] * 100}%`}>
      <SelfTabLayoutNode
        node={node.children[0]}
        {groups}
        {tabsById}
        {activeGroupId}
        {projectPath}
        {draggedTabId}
        {dropTarget}
        {onGroupFocus}
        {onTabSelect}
        {onTabClose}
        {onTabSplitAction}
        {onTabDragStart}
        {onTabDragEnd}
        {onTabDragOver}
        {onTabDrop}
        {onGroupDragOver}
        {onGroupDrop}
        {onSplitDragOver}
        {onSplitDrop}
        {onSplitResize}
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
      />
    </div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="split-divider"
      class:vertical={node.axis === "vertical"}
      onpointerdown={(event) => startResize(event, node.id)}
    ></div>
    <div class="split-child" style={`flex-basis: ${node.sizes[1] * 100}%`}>
      <SelfTabLayoutNode
        node={node.children[1]}
        {groups}
        {tabsById}
        {activeGroupId}
        {projectPath}
        {draggedTabId}
        {dropTarget}
        {onGroupFocus}
        {onTabSelect}
        {onTabClose}
        {onTabSplitAction}
        {onTabDragStart}
        {onTabDragEnd}
        {onTabDragOver}
        {onTabDrop}
        {onGroupDragOver}
        {onGroupDrop}
        {onSplitDragOver}
        {onSplitDrop}
        {onSplitResize}
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
      />
    </div>
  </div>
{/if}

<style>
  .split-node {
    display: flex;
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }

  .split-node.vertical {
    flex-direction: column;
  }

  .split-child {
    flex: 1 1 0;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }

  .split-divider {
    flex: 0 0 6px;
    background: var(--bg-secondary);
    cursor: col-resize;
    border-left: 1px solid var(--border-color);
    border-right: 1px solid var(--border-color);
  }

  .split-divider.vertical {
    cursor: row-resize;
    border-left: none;
    border-right: none;
    border-top: 1px solid var(--border-color);
    border-bottom: 1px solid var(--border-color);
  }
</style>
