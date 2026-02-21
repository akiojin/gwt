<script lang="ts">
  import type { CoordinatorState, DeveloperState, ProjectTask } from "../types";

  let {
    coordinator,
    issueTitle,
    developers = [],
    tasks = [],
    onViewTerminal,
  }: {
    coordinator: CoordinatorState;
    issueTitle: string;
    developers: DeveloperState[];
    tasks: ProjectTask[];
    onViewTerminal?: (paneId: string) => void;
  } = $props();

  function statusColor(status: string): string {
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
      case "not_run":
        return "#94a3b8";
      default:
        return "#94a3b8";
    }
  }

  function statusPulse(status: string): boolean {
    return (
      status === "in_progress" ||
      status === "running" ||
      status === "starting" ||
      status === "restarting"
    );
  }

  function handleViewTerminal(paneId: string) {
    onViewTerminal?.(paneId);
  }

  // Find tasks assigned to a specific developer
  function tasksForDeveloper(devId: string): ProjectTask[] {
    return tasks.filter((t) =>
      t.developers.some((d) => d.id === devId),
    );
  }
</script>

<div class="coordinator-detail" data-testid="coordinator-detail">
  <!-- Header: issue title + coordinator status -->
  <header class="coordinator-detail-header">
    <span
      class="status-dot"
      class:pulse={statusPulse(coordinator.status)}
      style="background-color: {statusColor(coordinator.status)}"
      data-testid="coordinator-status"
      data-status={coordinator.status}
    ></span>
    <span class="coordinator-detail-title">{issueTitle}</span>
  </header>

  <!-- Coordinator terminal link -->
  <div class="coordinator-section">
    <div class="coordinator-info">
      <span class="section-label">Coordinator</span>
      <span
        class="coordinator-status-value"
        style="color: {statusColor(coordinator.status)}"
      >{coordinator.status}</span>
    </div>
    <button
      type="button"
      class="view-terminal-btn"
      data-testid="view-terminal-coordinator"
      onclick={() => handleViewTerminal(coordinator.paneId)}
    >View Terminal</button>
  </div>

  <!-- Tasks section -->
  {#if tasks.length > 0}
    <div class="tasks-section">
      <span class="section-label">Tasks</span>
      <div class="task-list">
        {#each tasks as task}
          <div class="task-item" data-testid="task-item-{task.id}">
            <span
              class="status-dot small"
              class:pulse={statusPulse(task.status)}
              style="background-color: {statusColor(task.status)}"
            ></span>
            <span class="task-name">{task.name}</span>
          </div>
        {/each}
      </div>
    </div>
  {/if}

  <!-- Developer list -->
  <div class="developers-section">
    <span class="section-label">Developers</span>
    {#if developers.length === 0}
      <div class="no-developers">No developers assigned</div>
    {:else}
      <div class="developer-list">
        {#each developers as dev}
          <div class="developer-item" data-testid="developer-item-{dev.id}">
            <div class="developer-info">
              <span class="dev-agent-type">{dev.agentType}</span>
              <span
                class="dev-status"
                style="color: {statusColor(dev.status)}"
              >{dev.status}</span>
              <span class="dev-branch mono">{dev.worktree.branchName}</span>
            </div>
            <button
              type="button"
              class="view-terminal-btn"
              data-testid="view-terminal-{dev.id}"
              onclick={() => handleViewTerminal(dev.paneId)}
            >View Terminal</button>
          </div>

          <!-- Show tasks assigned to this developer -->
          {#if tasksForDeveloper(dev.id).length > 0}
            <div class="dev-tasks">
              {#each tasksForDeveloper(dev.id) as task}
                <div class="dev-task-item">
                  <span
                    class="status-dot tiny"
                    style="background-color: {statusColor(task.status)}"
                  ></span>
                  <span class="dev-task-name">{task.name}</span>
                </div>
              {/each}
            </div>
          {/if}
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .coordinator-detail {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 8px;
  }

  .coordinator-detail-header {
    display: flex;
    align-items: center;
    gap: 8px;
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

  .status-dot.tiny {
    width: 5px;
    height: 5px;
    min-width: 5px;
  }

  .status-dot.pulse {
    animation: pulse 2s ease-in-out infinite;
  }

  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }

  .coordinator-detail-title {
    flex: 1;
    font-weight: 600;
    font-size: var(--ui-font-sm);
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .coordinator-section {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 10px;
    background: var(--bg-secondary);
    border-radius: 6px;
    font-size: var(--ui-font-xs);
  }

  .coordinator-info {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .section-label {
    color: var(--text-muted);
    font-size: var(--ui-font-xs);
  }

  .coordinator-status-value {
    font-weight: 600;
  }

  .view-terminal-btn {
    padding: 3px 10px;
    font-size: var(--ui-font-xs);
    border: 1px solid var(--border-color);
    border-radius: 4px;
    background: var(--bg-surface);
    color: var(--text-secondary);
    cursor: pointer;
  }

  .view-terminal-btn:hover {
    background: var(--bg-primary);
    color: var(--text-primary);
  }

  .tasks-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .task-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .task-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    font-size: var(--ui-font-xs);
    color: var(--text-primary);
    background: var(--bg-secondary);
    border-radius: 4px;
  }

  .task-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .developers-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .no-developers {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .developer-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .developer-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 8px;
    border: 1px solid var(--border-color);
    border-radius: 6px;
    background: var(--bg-secondary);
    font-size: var(--ui-font-xs);
  }

  .developer-info {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    flex: 1;
  }

  .dev-agent-type {
    font-weight: 600;
    min-width: 48px;
    color: var(--text-primary);
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

  .dev-tasks {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-left: 16px;
    margin-top: -2px;
  }

  .dev-task-item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 6px;
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .dev-task-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
