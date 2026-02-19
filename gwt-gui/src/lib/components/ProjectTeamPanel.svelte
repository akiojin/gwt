<script lang="ts">
  import type { ProjectTeamState } from "../types";

  let { session = null }: { session: ProjectTeamState | null } = $props();

  let hasSession = $derived(session !== null);
  let issueCount = $derived(session?.issues.length ?? 0);
  let issueLabel = $derived(issueCount === 1 ? "1 issue" : `${issueCount} issues`);
</script>

<section class="project-team-panel">
  {#if hasSession && session}
    <header class="project-team-header">
      <div class="header-left">
        <span class="header-title">Project Team</span>
        <span class="status-badge">{session.status}</span>
      </div>
      <div class="header-right">
        <span class="header-stat">{issueLabel}</span>
        <span class="header-stat agent-type">{session.developerAgentType}</span>
      </div>
    </header>

    <div class="project-team-content">
      <div class="project-team-dashboard">
        <div class="placeholder-content">
          <span class="placeholder-label">Dashboard</span>
        </div>
      </div>
      <div class="project-team-chat">
        <div class="placeholder-content">
          <span class="placeholder-label">Lead Chat</span>
        </div>
      </div>
    </div>
  {:else}
    <div class="empty-state">
      <span class="empty-message">No active session</span>
    </div>
  {/if}
</section>

<style>
  .project-team-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    gap: 12px;
  }

  .project-team-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 8px;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .header-title {
    font-weight: 600;
    color: var(--text-primary);
  }

  .status-badge {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 10px;
    background: rgba(46, 196, 182, 0.15);
    color: var(--text-secondary);
    text-transform: lowercase;
  }

  .header-right {
    display: flex;
    gap: 12px;
    font-size: 12px;
    color: var(--text-muted);
  }

  .header-stat {
    white-space: nowrap;
  }

  .agent-type {
    text-transform: capitalize;
  }

  .project-team-content {
    flex: 1;
    display: grid;
    grid-template-columns: 2fr 3fr;
    gap: 12px;
    min-height: 0;
  }

  .project-team-dashboard,
  .project-team-chat {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .placeholder-content {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .placeholder-label {
    color: var(--text-muted);
    font-size: 14px;
  }

  .empty-state {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
  }

  .empty-message {
    color: var(--text-muted);
    font-size: 14px;
  }
</style>
