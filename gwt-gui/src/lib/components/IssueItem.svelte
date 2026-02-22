<script lang="ts">
  import type { ProjectIssue } from "../types";

  let {
    issue,
    onViewTerminal,
  }: {
    issue: ProjectIssue;
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
</script>

<div class="issue-detail" data-testid="issue-detail-{issue.id}">
  <header class="issue-detail-header">
    <span
      class="status-dot"
      class:pulse={statusPulse(issue.status)}
      style="background-color: {statusColor(issue.status)}"
      data-testid="issue-detail-status"
      data-status={issue.status}
    ></span>
    <span class="issue-detail-title">{issue.title}</span>
    <a
      class="issue-number"
      href={issue.githubIssueUrl}
      target="_blank"
      rel="noopener noreferrer"
    >#{issue.githubIssueNumber}</a>
  </header>

  {#if issue.coordinator}
    <div class="coordinator-section">
      <div class="coordinator-info">
        <span class="section-label">Coordinator</span>
        <span
          class="coordinator-status-value"
          style="color: {statusColor(issue.coordinator.status)}"
        >{issue.coordinator.status}</span>
      </div>
      <button
        type="button"
        class="view-terminal-btn"
        data-testid="view-terminal-btn"
        onclick={() => handleViewTerminal(issue.coordinator!.paneId)}
      >View Terminal</button>
    </div>
  {/if}

  <div class="tasks-section">
    {#if issue.tasks.length === 0}
      <div class="no-tasks">No tasks</div>
    {:else}
      <div class="task-list">
        {#each issue.tasks as task}
          <div class="task-item" data-testid="task-item-{task.id}">
            <div class="task-header">
              <span
                class="status-dot small"
                class:pulse={statusPulse(task.status)}
                style="background-color: {statusColor(task.status)}"
              ></span>
              <span class="task-name">{task.name}</span>
              {#if task.retryCount > 0}
                <span class="retry-badge">{task.retryCount} retries</span>
              {/if}
              {#if task.pullRequest}
                <span class="pr-badge">PR #{task.pullRequest.number}</span>
                {#if task.pullRequest.ciStatus}
                  <span
                    class="ci-badge"
                    data-testid="ci-status-{task.id}"
                    style="color: {statusColor(task.pullRequest.ciStatus)}"
                  >{task.pullRequest.ciStatus}</span>
                {/if}
              {/if}
            </div>

            {#if task.developers.length > 0}
              <div class="developer-list">
                {#each task.developers as dev}
                  <div class="developer-item">
                    <span class="dev-agent-type">{dev.agentType}</span>
                    <span
                      class="dev-status"
                      style="color: {statusColor(dev.status)}"
                    >{dev.status}</span>
                    <span class="dev-branch mono">{dev.worktree.branchName}</span>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .issue-detail {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 8px;
  }

  .issue-detail-header {
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

  .issue-detail-title {
    flex: 1;
    font-weight: 600;
    font-size: var(--ui-font-sm);
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .issue-number {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    text-decoration: none;
    flex-shrink: 0;
  }

  .issue-number:hover {
    color: var(--text-secondary);
    text-decoration: underline;
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

  .no-tasks {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .task-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .task-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 6px 8px;
    border: 1px solid var(--border-color);
    border-radius: 6px;
    background: var(--bg-secondary);
  }

  .task-header {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--ui-font-xs);
    color: var(--text-primary);
  }

  .task-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .retry-badge {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    background: var(--bg-surface);
    padding: 1px 6px;
    border-radius: 999px;
    flex-shrink: 0;
  }

  .pr-badge {
    font-size: var(--ui-font-xs);
    color: var(--text-secondary);
    background: var(--bg-surface);
    padding: 1px 6px;
    border-radius: 999px;
    flex-shrink: 0;
  }

  .ci-badge {
    font-size: var(--ui-font-xs);
    font-weight: 600;
    flex-shrink: 0;
  }

  .developer-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-left: 14px;
  }

  .developer-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 8px;
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
