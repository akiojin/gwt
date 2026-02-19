<script lang="ts">
  import type { DashboardIssue, ProjectTask, DeveloperState } from "../types";

  let {
    issues = [],
    onTaskClick,
  }: {
    issues: DashboardIssue[];
    onTaskClick?: (taskId: string) => void;
  } = $props();

  // Track local toggle overrides: id -> toggled-expanded-state
  let toggleOverrides: Record<string, boolean> = $state({});

  function toggleExpand(id: string) {
    const current = id in toggleOverrides
      ? toggleOverrides[id]
      : issues.find((i) => i.id === id)?.expanded ?? false;
    toggleOverrides = { ...toggleOverrides, [id]: !current };
  }

  function isExpanded(id: string): boolean {
    if (id in toggleOverrides) return toggleOverrides[id];
    return issues.find((i) => i.id === id)?.expanded ?? false;
  }

  function statusColor(
    status: string,
  ): string {
    switch (status) {
      case "pending":
        return "#94a3b8";
      case "planned":
      case "ready":
        return "#60a5fa";
      case "in_progress":
      case "running":
      case "starting":
      case "restarting":
        return "#3b82f6";
      case "completed":
      case "passed":
        return "#22c55e";
      case "failed":
      case "error":
      case "crashed":
        return "#ef4444";
      case "ci_fail":
        return "#f97316";
      case "cancelled":
        return "#94a3b8";
      default:
        return "#94a3b8";
    }
  }

  function statusPulse(status: string): boolean {
    return status === "in_progress" || status === "running" || status === "starting" || status === "restarting";
  }

  function agentTypeLabel(agentType: DeveloperState["agentType"]): string {
    return agentType;
  }

  function handleTaskClick(taskId: string) {
    onTaskClick?.(taskId);
  }
</script>

<div class="dashboard">
  {#if issues.length === 0}
    <div class="empty-state">No issues yet</div>
  {:else}
    <div class="issue-list">
      {#each issues as issue}
        <div class="issue-item" data-testid="issue-row-{issue.id}">
          <button
            type="button"
            class="issue-header"
            onclick={() => toggleExpand(issue.id)}
          >
            <span
              class="status-dot"
              class:pulse={statusPulse(issue.status)}
              style="background-color: {statusColor(issue.status)}"
              data-testid="issue-status-{issue.id}"
              data-status={issue.status}
            ></span>
            <span class="issue-title">{issue.title}</span>
            <span class="issue-task-count">{issue.taskCompletedCount}/{issue.taskTotalCount} tasks</span>
            <span class="chevron" class:expanded={isExpanded(issue.id)}>&#9654;</span>
          </button>

          {#if isExpanded(issue.id)}
            <div class="issue-body">
              {#if issue.coordinator}
                <div class="coordinator-status">
                  <span class="coordinator-label">Coordinator:</span>
                  <span
                    class="coordinator-value"
                    style="color: {statusColor(issue.coordinator.status)}"
                  >{issue.coordinator.status}</span>
                </div>
              {/if}

              {#if issue.tasks.length === 0}
                <div class="no-tasks">No tasks</div>
              {:else}
                <div class="task-list">
                  {#each issue.tasks as task}
                    <button
                      type="button"
                      class="task-row"
                      data-testid="task-row-{task.id}"
                      onclick={() => handleTaskClick(task.id)}
                    >
                      <span
                        class="status-dot small"
                        class:pulse={statusPulse(task.status)}
                        style="background-color: {statusColor(task.status)}"
                        data-testid="task-status-{task.id}"
                        data-status={task.status}
                      ></span>
                      <span class="task-name">{task.name}</span>
                      {#if task.developers.length > 0}
                        <span class="dev-count">{task.developers.length}</span>
                      {/if}
                    </button>

                    {#if task.developers.length > 0}
                      <div class="developer-list">
                        {#each task.developers as dev}
                          <div class="developer-item">
                            <span class="dev-agent-type">{agentTypeLabel(dev.agentType)}</span>
                            <span
                              class="dev-status"
                              style="color: {statusColor(dev.status)}"
                            >{dev.status}</span>
                            <span class="dev-branch mono">{dev.worktree.branchName}</span>
                          </div>
                        {/each}
                      </div>
                    {/if}
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .dashboard {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    padding: 8px;
  }

  .empty-state {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: 1;
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
  }

  .issue-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .issue-item {
    border: 1px solid var(--border-color);
    border-radius: 8px;
    background: var(--bg-surface);
    overflow: hidden;
  }

  .issue-header {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px;
    border: none;
    background: none;
    color: var(--text-primary);
    cursor: pointer;
    text-align: left;
    font-size: var(--ui-font-sm);
  }

  .issue-header:hover {
    background: var(--bg-secondary);
  }

  .status-dot {
    width: 8px;
    height: 8px;
    min-width: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .status-dot.small {
    width: 6px;
    height: 6px;
    min-width: 6px;
  }

  .status-dot.pulse {
    animation: pulse 2s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .issue-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .issue-task-count {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .chevron {
    font-size: 10px;
    color: var(--text-muted);
    transition: transform 0.15s ease;
    flex-shrink: 0;
  }

  .chevron.expanded {
    transform: rotate(90deg);
  }

  .issue-body {
    padding: 0 10px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .coordinator-status {
    font-size: var(--ui-font-xs);
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    background: var(--bg-secondary);
    border-radius: 4px;
  }

  .coordinator-label {
    color: var(--text-muted);
  }

  .coordinator-value {
    font-weight: 600;
  }

  .no-tasks {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .task-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .task-row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border: 1px solid var(--border-color);
    border-radius: 6px;
    background: var(--bg-secondary);
    color: var(--text-primary);
    cursor: pointer;
    text-align: left;
    font-size: var(--ui-font-xs);
  }

  .task-row:hover {
    border-color: rgba(137, 180, 250, 0.4);
  }

  .task-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dev-count {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    background: var(--bg-surface);
    padding: 1px 5px;
    border-radius: 999px;
    flex-shrink: 0;
  }

  .developer-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-left: 22px;
  }

  .developer-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 8px;
    font-size: var(--ui-font-xs);
    color: var(--text-secondary);
  }

  .dev-agent-type {
    font-weight: 600;
    min-width: 48px;
  }

  .dev-status {
    min-width: 64px;
  }

  .dev-branch {
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mono {
    font-family: monospace;
  }
</style>
