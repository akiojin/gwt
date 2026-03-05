<script lang="ts">
  import type { ProjectIssue } from '$lib/types';
  import AgentAvatar from './AgentAvatar.svelte';
  import TaskBar from './TaskBar.svelte';
  import PixelSprite from './PixelSprite.svelte';
  import { getBuildingSprite } from './sprites';

  interface Props {
    issue: ProjectIssue;
    expanded?: boolean;
    onToggle?: () => void;
    onAgentClick?: (paneId: string) => void;
  }

  let { issue, expanded = false, onToggle, onAgentClick }: Props = $props();

  let buildingSprite = $derived(getBuildingSprite(issue.status));

  let completedTasks = $derived(issue.tasks.filter(t => t.status === 'completed').length);
  let totalTasks = $derived(issue.tasks.length);
  let progressPercent = $derived(totalTasks > 0 ? (completedTasks / totalTasks) * 100 : 0);
  let allDevelopers = $derived(issue.tasks.flatMap(t => t.developers));

  const statusLabels: Record<string, string> = {
    pending: 'Pending',
    planned: 'Planned',
    in_progress: 'In Progress',
    ci_fail: 'CI Failed',
    completed: 'Completed',
    failed: 'Failed',
  };
</script>

<div
  class="issue-plot"
  class:pending={issue.status === 'pending'}
  class:planned={issue.status === 'planned'}
  class:in_progress={issue.status === 'in_progress'}
  class:ci_fail={issue.status === 'ci_fail'}
  class:completed={issue.status === 'completed'}
  class:failed={issue.status === 'failed'}
  role="region"
  aria-label={`Issue: ${issue.title}`}
>
  <button class="plot-header" onclick={onToggle} type="button" aria-expanded={expanded}>
    <PixelSprite sprite={buildingSprite} scale={1} class="plot-building" />
    <span class="plot-title" title={issue.title}>
      <span class="issue-num">#{issue.githubIssueNumber}</span>
      {issue.title}
    </span>
    <span class="plot-status">{statusLabels[issue.status] ?? issue.status}</span>
  </button>

  <div class="plot-body">
    {#if issue.status !== 'pending'}
      <div class="progress-row">
        <div class="progress-track">
          <div class="progress-fill" style:width="{progressPercent}%"></div>
        </div>
        <span class="progress-text">{completedTasks}/{totalTasks}</span>
      </div>
    {/if}

    {#if allDevelopers.length > 0}
      <div class="agents-row">
        {#each allDevelopers as dev (dev.id)}
          <AgentAvatar
            agentType={dev.agentType}
            status={dev.status}
            name={dev.id}
            onclick={() => onAgentClick?.(dev.paneId)}
          />
        {/each}
      </div>
    {/if}
  </div>

  {#if expanded}
    <div class="plot-details">
      {#each issue.tasks as task (task.id)}
        <div class="task-row">
          <TaskBar
            name={task.name}
            status={task.status}
            testStatus={task.testStatus}
            retryCount={task.retryCount}
          />
          {#if task.developers.length > 0}
            <div class="task-agents">
              {#each task.developers as dev (dev.id)}
                <AgentAvatar
                  agentType={dev.agentType}
                  status={dev.status}
                  name={dev.id}
                  onclick={() => onAgentClick?.(dev.paneId)}
                />
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .issue-plot {
    border: 1px solid #4a4a6a;
    border-radius: 10px;
    background: rgba(45, 43, 85, 0.6);
    overflow: hidden;
    transition: border-color 0.3s ease, box-shadow 0.3s ease;
    min-width: 200px;
    max-width: 320px;
    flex: 1 1 260px;
  }

  .issue-plot:hover {
    border-color: rgba(180, 190, 254, 0.4);
  }

  /* Status visual styles */
  .pending {
    border-style: dashed;
    border-color: #6c7086;
    opacity: 0.7;
  }

  .planned {
    border-color: #74c7ec;
    background:
      linear-gradient(rgba(116, 199, 236, 0.05) 1px, transparent 1px),
      linear-gradient(90deg, rgba(116, 199, 236, 0.05) 1px, transparent 1px),
      rgba(45, 43, 85, 0.6);
    background-size: 12px 12px, 12px 12px, 100% 100%;
  }

  .in_progress {
    border-color: #74c7ec;
    box-shadow: 0 0 12px rgba(116, 199, 236, 0.15);
  }

  .ci_fail {
    border-color: #fab387;
    background:
      repeating-linear-gradient(
        45deg,
        transparent,
        transparent 8px,
        rgba(250, 179, 135, 0.06) 8px,
        rgba(250, 179, 135, 0.06) 16px
      ),
      rgba(45, 43, 85, 0.6);
  }

  .completed {
    border-color: #a6e3a1;
    box-shadow: 0 0 16px rgba(166, 227, 161, 0.2);
  }

  .failed {
    border-color: #f38ba8;
    background: rgba(243, 139, 168, 0.08);
  }

  .plot-header {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 10px 12px;
    background: none;
    border: none;
    border-bottom: 1px solid rgba(74, 74, 106, 0.5);
    cursor: pointer;
    text-align: left;
    color: inherit;
  }

  .plot-header :global(.plot-building) {
    flex-shrink: 0;
  }

  .plot-header:hover {
    background: rgba(180, 190, 254, 0.05);
  }

  .plot-header:focus-visible {
    outline: 2px solid #b4befe;
    outline-offset: -2px;
  }

  .plot-title {
    font-size: var(--ui-font-sm, 11px);
    color: #cdd6f4;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }

  .issue-num {
    color: rgba(180, 190, 254, 0.6);
    font-weight: 400;
    margin-right: 4px;
  }

  .plot-status {
    font-size: var(--ui-font-xs, 10px);
    color: #6c7086;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    flex-shrink: 0;
  }

  .plot-body {
    padding: 8px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .progress-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .progress-track {
    flex: 1;
    height: 4px;
    background: rgba(108, 112, 134, 0.3);
    border-radius: 2px;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background: #74c7ec;
    border-radius: 2px;
    transition: width 0.5s ease;
  }

  .completed .progress-fill {
    background: #a6e3a1;
  }

  .ci_fail .progress-fill {
    background: #fab387;
  }

  .failed .progress-fill {
    background: #f38ba8;
  }

  .progress-text {
    font-size: 10px;
    color: rgba(205, 214, 244, 0.6);
    font-family: var(--font-mono, monospace);
    flex-shrink: 0;
  }

  .agents-row {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }

  .plot-details {
    border-top: 1px solid rgba(74, 74, 106, 0.5);
    padding: 8px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .task-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .task-row :global(.task-bar) {
    flex: 1;
    min-width: 0;
  }

  .task-agents {
    display: flex;
    gap: 2px;
    flex-shrink: 0;
  }
</style>
