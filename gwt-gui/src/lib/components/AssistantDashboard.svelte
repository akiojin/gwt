<script lang="ts">
  import type { AssistantState, DashboardData } from "../types";

  interface Props {
    dashboard: DashboardData | null;
    assistantState: AssistantState | null;
  }

  let { dashboard, assistantState }: Props = $props();

  function statusLabel(status?: string | null): string {
    switch (status) {
      case "analyzing":
        return "Analyzing";
      case "awaiting_goal_confirmation":
        return "Needs Goal";
      case "blocked":
        return "Blocked";
      case "monitoring":
        return "Monitoring";
      default:
        return "Idle";
    }
  }

  function confidenceLabel(confidence?: string | null): string {
    if (!confidence) return "unknown";
    return confidence;
  }

  function paneStatusTone(status: string): "running" | "stopped" | "error" {
    if (status.startsWith("error")) return "error";
    if (status.startsWith("completed")) return "stopped";
    return "running";
  }
</script>

{#if !dashboard}
  <div class="dashboard-loading">Loading...</div>
{:else}
  <div class="dashboard">
    {#if assistantState}
      <div class="goal-strip" data-testid="assistant-goal-strip">
        <div class="goal-main">
          <span class="goal-label">Current goal</span>
          <strong class="goal-text">
            {assistantState.workingGoal ?? "Goal not confirmed yet"}
          </strong>
        </div>
        <div class="goal-meta">
          <span class="meta-pill">{statusLabel(assistantState.currentStatus)}</span>
          <span class="meta-pill confidence">
            confidence: {confidenceLabel(assistantState.goalConfidence)}
          </span>
        </div>

        {#if assistantState.blockers.length > 0}
          <div class="goal-section blockers">
            <span class="section-label">Blockers</span>
            <div class="insight-cards blockers">
              {#each assistantState.blockers as blocker}
                <div class="insight-card blocker-card">{blocker}</div>
              {/each}
            </div>
          </div>
        {/if}

        {#if assistantState.recommendedNextActions.length > 0}
          <div class="goal-section actions">
            <span class="section-label">Next actions</span>
            <div class="insight-cards actions">
              {#each assistantState.recommendedNextActions as action}
                <div class="insight-card action-card">{action}</div>
              {/each}
            </div>
          </div>
        {/if}
      </div>
    {/if}

    {#if dashboard.specProgress}
      <div class="spec-progress">
        <h3>SPEC Progress</h3>
        <div class="spec-card">
          <div class="spec-header">
            <span class="spec-title">#{dashboard.specProgress.issueNumber} {dashboard.specProgress.title}</span>
            <span class="phase-badge" data-phase={dashboard.specProgress.phase}>
              {dashboard.specProgress.phase}
            </span>
          </div>
          {#if dashboard.specProgress.tasksTotal > 0}
            <div class="progress-bar-container">
              <div
                class="progress-bar-fill"
                style="width: {(dashboard.specProgress.tasksCompleted / dashboard.specProgress.tasksTotal) * 100}%"
              ></div>
            </div>
            <span class="progress-text">
              {dashboard.specProgress.tasksCompleted}/{dashboard.specProgress.tasksTotal} tasks
            </span>
          {/if}
        </div>
      </div>
    {/if}

    {#if dashboard.ciStatus}
      <div class="ci-status">
        <h3>CI / Review</h3>
        <div class="ci-card">
          <span
            class="ci-indicator"
            class:passing={dashboard.ciStatus.checkStatus === "passing"}
            class:failing={dashboard.ciStatus.checkStatus === "failing"}
            class:pending={dashboard.ciStatus.checkStatus === "pending"}
          ></span>
          <div class="ci-info">
            <span class="ci-pr">PR #{dashboard.ciStatus.prNumber} {dashboard.ciStatus.prTitle}</span>
            <span class="ci-detail">
              Checks: {dashboard.ciStatus.checkStatus}
              {#if dashboard.ciStatus.failingChecks.length > 0}
                ({dashboard.ciStatus.failingChecks.join(", ")})
              {/if}
              · Review: {dashboard.ciStatus.reviewStatus}
            </span>
          </div>
        </div>
      </div>
    {/if}

    {#if (dashboard.consultationCount ?? 0) > 0}
      <div class="consultation-badge">
        <span class="badge-text">{dashboard.consultationCount} pending consultation{dashboard.consultationCount === 1 ? '' : 's'}</span>
      </div>
    {/if}

    <div class="git-summary">
      <h3>Git Summary</h3>
      <div class="git-info">
        <span class="git-branch">{dashboard.git.branch || "No branch"}</span>
        <span class="git-stat">{dashboard.git.uncommittedCount} uncommitted</span>
        <span class="git-stat">{dashboard.git.unpushedCount} unpushed</span>
      </div>
    </div>

    <div class="panes-section">
      <h3>Agents</h3>
      {#if dashboard.panes.length === 0}
        <p class="no-panes">No active agent for this project</p>
      {:else}
        <div class="pane-cards">
          {#each dashboard.panes as pane}
            <div class="pane-card">
              <span
                class="status-indicator"
                class:running={paneStatusTone(pane.status) === "running"}
                class:stopped={paneStatusTone(pane.status) === "stopped"}
                class:error={paneStatusTone(pane.status) === "error"}
              ></span>
              <div class="pane-info">
                <span class="pane-agent">{pane.agentName}</span>
                <span class="pane-id">{pane.paneId}</span>
              </div>
              <span class="pane-status">{pane.status}</span>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .dashboard-loading {
    padding: 12px;
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
  }

  .dashboard {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px;
    border-bottom: 1px solid var(--border-color);
    background:
      linear-gradient(180deg, color-mix(in srgb, var(--bg-secondary) 82%, transparent), transparent);
  }

  .goal-strip {
    display: grid;
    gap: 10px;
    padding: 12px;
    border-radius: 10px;
    background:
      linear-gradient(135deg, color-mix(in srgb, var(--accent) 14%, var(--bg-secondary)), var(--bg-secondary));
    border: 1px solid color-mix(in srgb, var(--accent) 24%, var(--border-color));
  }

  .goal-main {
    display: grid;
    gap: 4px;
  }

  .goal-label,
  .section-label {
    font-size: var(--ui-font-xs);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-secondary);
  }

  .goal-text {
    font-size: var(--ui-font-md);
    color: var(--text-primary);
    line-height: 1.4;
  }

  .goal-meta {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }

  .meta-pill {
    padding: 3px 8px;
    border-radius: 999px;
    font-size: var(--ui-font-xs);
    background: color-mix(in srgb, var(--bg-primary) 70%, transparent);
    color: var(--text-primary);
  }

  .meta-pill.confidence {
    color: var(--text-secondary);
  }

  .goal-section {
    display: grid;
    gap: 4px;
  }

  .insight-cards {
    display: grid;
    gap: 6px;
  }

  .insight-card {
    padding: 8px 10px;
    border-radius: 8px;
    font-size: var(--ui-font-sm);
    line-height: 1.45;
    color: var(--text-primary);
    background: color-mix(in srgb, var(--bg-primary) 66%, transparent);
    border: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
  }

  .blocker-card {
    border-color: color-mix(in srgb, var(--red) 35%, var(--border-color));
    background: color-mix(in srgb, var(--red) 8%, var(--bg-primary));
  }

  .action-card {
    border-color: color-mix(in srgb, var(--accent) 30%, var(--border-color));
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-primary));
  }

  h3 {
    margin: 0 0 6px 0;
    font-size: var(--ui-font-sm);
    font-weight: 600;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .spec-progress,
  .ci-status {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .spec-card,
  .ci-card {
    padding: 8px 10px;
    border-radius: 8px;
    background: var(--bg-secondary);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .spec-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
  }

  .spec-title {
    font-size: var(--ui-font-sm);
    color: var(--text-primary);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .phase-badge {
    padding: 2px 8px;
    border-radius: 999px;
    font-size: var(--ui-font-xs);
    background: color-mix(in srgb, var(--accent) 20%, var(--bg-primary));
    color: var(--accent);
    flex-shrink: 0;
  }

  .progress-bar-container {
    height: 4px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--border-color) 50%, transparent);
    overflow: hidden;
  }

  .progress-bar-fill {
    height: 100%;
    border-radius: 2px;
    background: var(--accent);
    transition: width 0.3s ease;
  }

  .progress-text {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .ci-card {
    flex-direction: row;
    align-items: center;
  }

  .ci-indicator {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    background-color: var(--text-muted);
  }

  .ci-indicator.passing {
    background-color: var(--green);
  }

  .ci-indicator.failing {
    background-color: var(--red);
  }

  .ci-indicator.pending {
    background-color: var(--yellow);
  }

  .ci-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .ci-pr {
    font-size: var(--ui-font-sm);
    color: var(--text-primary);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ci-detail {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .consultation-badge {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--accent) 12%, var(--bg-secondary));
    border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border-color));
  }

  .badge-text {
    font-size: var(--ui-font-sm);
    color: var(--text-primary);
  }

  .git-info {
    display: flex;
    gap: 12px;
    align-items: center;
    flex-wrap: wrap;
  }

  .git-branch {
    font-weight: 600;
    color: var(--accent);
    font-size: var(--ui-font-sm);
  }

  .git-stat {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .no-panes {
    margin: 0;
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
  }

  .pane-cards {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .pane-card {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-radius: 4px;
    background-color: var(--bg-secondary);
  }

  .status-indicator {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    background-color: var(--text-muted);
  }

  .status-indicator.running {
    background-color: var(--green);
  }

  .status-indicator.stopped {
    background-color: var(--yellow);
  }

  .status-indicator.error {
    background-color: var(--red);
  }

  .pane-info {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }

  .pane-agent {
    font-size: var(--ui-font-sm);
    font-weight: 500;
    color: var(--text-primary);
  }

  .pane-id {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pane-status {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    flex-shrink: 0;
  }
</style>
