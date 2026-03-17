<script lang="ts">
  import type { DashboardData } from "../types";

  interface Props {
    dashboard: DashboardData | null;
  }
  let { dashboard }: Props = $props();
</script>

{#if !dashboard}
  <div class="dashboard-loading">Loading...</div>
{:else}
  <div class="dashboard">
    <div class="git-summary">
      <h3>Git Summary</h3>
      <div class="git-info">
        <span class="git-branch">{dashboard.git.branch}</span>
        <span class="git-stat">
          {dashboard.git.uncommittedCount} uncommitted
        </span>
        <span class="git-stat">
          {dashboard.git.unpushedCount} unpushed
        </span>
      </div>
    </div>

    <div class="panes-section">
      <h3>Panes</h3>
      {#if dashboard.panes.length === 0}
        <p class="no-panes">No active panes</p>
      {:else}
        <div class="pane-cards">
          {#each dashboard.panes as pane}
            <div class="pane-card">
              <span
                class="status-indicator"
                class:running={pane.status === "running"}
                class:stopped={pane.status === "stopped"}
                class:error={pane.status === "error"}
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
  }

  h3 {
    margin: 0 0 6px 0;
    font-size: var(--ui-font-sm);
    font-weight: 600;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.05em;
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
    background-color: var(--text-muted);
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
