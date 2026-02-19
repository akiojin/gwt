<script lang="ts">
  import type { ProjectTeamState } from "../types";

  let {
    session = null,
    aiReady = true,
    onOpenSettings = () => {},
    onOpenIssues = () => {},
    agentBranches = [],
    viewTerminalPaneId,
    onBackToDashboard,
  }: {
    session: ProjectTeamState | null;
    aiReady?: boolean;
    onOpenSettings?: () => void;
    onOpenIssues?: () => void;
    agentBranches?: string[];
    viewTerminalPaneId?: string | null;
    onBackToDashboard?: () => void;
  } = $props();

  let isTerminalView = $derived(
    viewTerminalPaneId !== undefined &&
    viewTerminalPaneId !== null &&
    viewTerminalPaneId !== "",
  );

  function handleBackToDashboard() {
    onBackToDashboard?.();
  }

  let hasSession = $derived(session !== null);
  let issueCount = $derived(session?.issues.length ?? 0);
  let issueLabel = $derived(issueCount === 1 ? "1 issue" : `${issueCount} issues`);

  function formatTokens(tokens: number): string {
    if (tokens >= 1_000_000) {
      const m = tokens / 1_000_000;
      return `~${parseFloat(m.toFixed(1))}M`;
    }
    if (tokens >= 1_000) {
      const k = Math.round(tokens / 1_000);
      return `~${k}K`;
    }
    return `~${tokens}`;
  }

  let apiCalls = $derived(session?.lead.llmCallCount ?? 0);
  let tokensDisplay = $derived(formatTokens(session?.lead.estimatedTokens ?? 0));
  let hasAgentBranches = $derived(hasSession && agentBranches.length > 0);
</script>

<section class="project-team-panel">
  {#if !aiReady}
    <div class="ai-error">
      <span class="ai-error-message">AI provider is not configured</span>
      <button class="ai-error-settings-btn" onclick={onOpenSettings}>Settings</button>
    </div>
  {:else if hasSession && session}
    <header class="project-team-header">
      <div class="header-left">
        <span class="header-title">Project Team</span>
        <span class="status-badge">{session.status}</span>
      </div>
      <div class="header-right">
        <button
          type="button"
          class="header-btn"
          data-testid="github-issues-btn"
          onclick={onOpenIssues}
        >GitHub Issues</button>
        <span class="header-stat">API Calls: {apiCalls}</span>
        <span class="header-stat">Tokens: {tokensDisplay}</span>
        <span class="header-stat">{issueLabel}</span>
        <span class="header-stat agent-type">{session.developerAgentType}</span>
      </div>
    </header>

    {#if isTerminalView}
      <div class="terminal-view">
        <div class="terminal-view-toolbar">
          <button
            type="button"
            class="back-to-dashboard-btn"
            data-testid="back-to-dashboard"
            onclick={handleBackToDashboard}
          >Back to Dashboard</button>
          <span class="terminal-pane-label">Terminal: {viewTerminalPaneId}</span>
        </div>
        <div
          class="terminal-embed"
          data-testid="terminal-embed"
          data-pane-id={viewTerminalPaneId}
        >
          <!-- Terminal will be mounted here by xterm integration -->
        </div>
      </div>
    {:else}
      <div class="project-team-content">
        <div class="project-team-dashboard">
          {#if hasAgentBranches}
            <div class="agent-branches" data-testid="agent-branches">
              <span class="branches-heading">Agent Branches</span>
              <ul class="branch-list">
                {#each agentBranches as branch}
                  <li class="branch-item">
                    {#if branch.startsWith("agent/")}
                      <span class="agent-branch-badge">agent</span>
                    {/if}
                    <span class="branch-name">{branch}</span>
                  </li>
                {/each}
              </ul>
            </div>
          {/if}
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
    {/if}
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

  .ai-error {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    height: 100%;
  }

  .ai-error-message {
    color: var(--text-muted);
    font-size: 14px;
  }

  .ai-error-settings-btn {
    padding: 6px 16px;
    border: 1px solid var(--border-color);
    border-radius: 6px;
    background: var(--bg-surface);
    color: var(--text-primary);
    cursor: pointer;
    font-size: 13px;
  }

  .ai-error-settings-btn:hover {
    background: var(--bg-hover);
  }

  .header-btn {
    padding: 4px 12px;
    font-size: 12px;
    border: 1px solid var(--border-color);
    border-radius: 6px;
    background: var(--bg-surface);
    color: var(--text-secondary);
    cursor: pointer;
    white-space: nowrap;
  }

  .header-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .agent-branches {
    padding: 10px 12px;
    border-bottom: 1px solid var(--border-color);
  }

  .branches-heading {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .branch-list {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .branch-item {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .agent-branch-badge {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 8px;
    background: rgba(99, 102, 241, 0.15);
    color: rgb(99, 102, 241);
    font-weight: 500;
  }

  .branch-name {
    font-family: monospace;
    font-size: 12px;
  }

  .terminal-view {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
  }

  .terminal-view-toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 12px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 8px;
  }

  .back-to-dashboard-btn {
    padding: 4px 12px;
    font-size: var(--ui-font-xs);
    border: 1px solid var(--border-color);
    border-radius: 4px;
    background: var(--bg-surface);
    color: var(--text-secondary);
    cursor: pointer;
  }

  .back-to-dashboard-btn:hover {
    background: var(--bg-primary);
    color: var(--text-primary);
  }

  .terminal-pane-label {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    font-family: monospace;
  }

  .terminal-embed {
    flex: 1;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    min-height: 200px;
  }
</style>
