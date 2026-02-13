<script lang="ts">
  import type {
    BranchInfo,
    SessionSummaryResult,
    AgentSidebarView,
    AgentSidebarTask,
    AgentSidebarSubAgent,
  } from "../types";

  let {
    projectPath,
    selectedBranch = null,
    currentBranch = "",
  }: {
    projectPath: string;
    selectedBranch?: BranchInfo | null;
    currentBranch?: string;
  } = $props();

  let sidebarView: AgentSidebarView = $state({ spec_id: null, tasks: [] });
  let sidebarLoading = $state(false);
  let sidebarError: string | null = $state(null);
  let selectedTaskId: string | null = $state(null);

  let sessionSummaryLoading: boolean = $state(false);
  let sessionSummaryGenerating: boolean = $state(false);
  let sessionSummaryStatus: SessionSummaryResult["status"] | "" = $state("");
  let sessionSummaryWarning: string | null = $state(null);
  let sessionSummaryError: string | null = $state(null);
  let sessionSummaryToolId: string | null = $state(null);
  const SIDEBAR_POLL_INTERVAL_MS = 5000;
  const SESSION_SUMMARY_POLL_INTERVAL_MS = 5000;

  function normalizeBranchName(name: string): string {
    const trimmed = name.trim();
    return trimmed.startsWith("origin/") ? trimmed.slice("origin/".length) : trimmed;
  }

  function activeBranchName(): string {
    const raw = selectedBranch?.name?.trim() ?? currentBranch?.trim() ?? "";
    return normalizeBranchName(raw);
  }

  function taskStatusRank(status: AgentSidebarTask["status"]): number {
    if (status === "running") return 0;
    if (status === "pending") return 1;
    if (status === "failed") return 2;
    if (status === "completed") return 3;
    return 4;
  }

  let activeBranch = $derived(activeBranchName());
  let tasks = $derived(
    [...(sidebarView.tasks ?? [])].sort((a, b) => {
      const byStatus = taskStatusRank(a.status) - taskStatusRank(b.status);
      if (byStatus !== 0) return byStatus;
      return a.id.localeCompare(b.id);
    }),
  );
  let selectedTask = $derived(
    tasks.find((task) => task.id === selectedTaskId) ?? tasks[0] ?? null,
  );

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  function summaryStatusLabel(
    status: SessionSummaryResult["status"] | "",
    generating: boolean,
  ): string {
    if (generating) return "Generating";
    switch (status) {
      case "ok":
        return "Ready";
      case "no-session":
        return "No session";
      case "ai-not-configured":
        return "AI not configured";
      case "disabled":
        return "Disabled";
      case "error":
        return "Error";
      default:
        return "Idle";
    }
  }

  function summaryStatusClass(
    status: SessionSummaryResult["status"] | "",
    generating: boolean,
  ): string {
    if (generating) return "warning";
    switch (status) {
      case "ok":
        return "ok";
      case "error":
        return "error";
      case "ai-not-configured":
      case "disabled":
      case "no-session":
        return "muted";
      default:
        return "muted";
    }
  }

  function taskStatusClass(status: AgentSidebarTask["status"]): string {
    if (status === "running") return "running";
    if (status === "pending") return "pending";
    if (status === "failed") return "failed";
    if (status === "completed") return "completed";
    return "pending";
  }

  function subAgentStatusClass(status: AgentSidebarSubAgent["status"]): string {
    if (status === "running") return "running";
    if (status === "failed") return "failed";
    return "completed";
  }

  async function loadSidebarView() {
    sidebarLoading = true;
    sidebarError = null;

    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const view = await invoke<AgentSidebarView>("get_agent_sidebar_view", {
        projectPath,
      });
      sidebarView = view ?? { spec_id: null, tasks: [] };
      const hasSelected =
        !!selectedTaskId && sidebarView.tasks.some((t) => t.id === selectedTaskId);
      if (!hasSelected) {
        selectedTaskId = sidebarView.tasks[0]?.id ?? null;
      }
    } catch (err) {
      sidebarView = { spec_id: null, tasks: [] };
      sidebarError = `Failed to load tasks: ${toErrorMessage(err)}`;
      selectedTaskId = null;
    } finally {
      sidebarLoading = false;
    }
  }

  async function loadSessionSummary(options: { silent?: boolean } = {}) {
    const silent = options.silent === true;
    sessionSummaryError = null;
    sessionSummaryWarning = null;

    const branch = activeBranchName();
    if (!branch) {
      sessionSummaryLoading = false;
      sessionSummaryGenerating = false;
      sessionSummaryStatus = "";
      sessionSummaryToolId = null;
      return;
    }

    const key = `${projectPath}::${branch}`;
    if (!silent) {
      sessionSummaryLoading = true;
      sessionSummaryGenerating = false;
      sessionSummaryStatus = "";
      sessionSummaryToolId = null;
    }

    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const result = await invoke<SessionSummaryResult>("get_branch_session_summary", {
        projectPath,
        branch,
      });
      if (`${projectPath}::${activeBranchName()}` !== key) return;
      sessionSummaryStatus = result.status;
      sessionSummaryGenerating = !!result.generating;
      sessionSummaryWarning = result.warning ?? null;
      sessionSummaryError = result.error ?? null;
      sessionSummaryToolId = result.toolId ?? null;
    } catch (err) {
      sessionSummaryStatus = "error";
      sessionSummaryGenerating = false;
      sessionSummaryToolId = null;
      sessionSummaryError = `Failed to load summary: ${toErrorMessage(err)}`;
    } finally {
      if (`${projectPath}::${activeBranchName()}` === key && !silent) {
        sessionSummaryLoading = false;
      }
    }
  }

  $effect(() => {
    void projectPath;
    void selectedBranch;
    void currentBranch;
    loadSidebarView();

    const timer = window.setInterval(() => {
      loadSidebarView();
    }, SIDEBAR_POLL_INTERVAL_MS);

    return () => {
      window.clearInterval(timer);
    };
  });

  $effect(() => {
    void projectPath;
    void selectedBranch;
    void currentBranch;
    const branch = activeBranchName();
    if (!branch) {
      loadSessionSummary();
      return;
    }

    loadSessionSummary();

    const timer = window.setInterval(() => {
      if (
        sessionSummaryStatus === "disabled" ||
        sessionSummaryStatus === "ai-not-configured"
      ) {
        return;
      }
      loadSessionSummary({ silent: true });
    }, SESSION_SUMMARY_POLL_INTERVAL_MS);

    return () => {
      window.clearInterval(timer);
    };
  });
</script>

<div class="agent-sidebar">
  <div class="agent-header">
    <div>
      <div class="agent-title">Agent Tasks</div>
      <div class="agent-branch">
        {#if activeBranch}
          {activeBranch}
        {:else}
          Select a branch to view tasks.
        {/if}
      </div>
    </div>
    {#if sidebarView.spec_id}
      <div class="spec-id">{sidebarView.spec_id}</div>
    {/if}
  </div>

  <div class="agent-section">
    <div class="section-header">
      <span>Summary</span>
      <span class="status-pill {summaryStatusClass(sessionSummaryStatus, sessionSummaryGenerating)}">
        {summaryStatusLabel(sessionSummaryStatus, sessionSummaryGenerating)}
      </span>
    </div>
    {#if !activeBranch}
      <div class="agent-muted">No branch context.</div>
    {:else if sessionSummaryLoading}
      <div class="agent-muted">Loading summary...</div>
    {:else if sessionSummaryError}
      <div class="agent-error">{sessionSummaryError}</div>
    {:else if sessionSummaryWarning}
      <div class="agent-warning">{sessionSummaryWarning}</div>
    {:else if sessionSummaryToolId}
      <div class="agent-muted">Latest tool: {sessionSummaryToolId}</div>
    {:else if sessionSummaryStatus === "ok"}
      <div class="agent-muted">Latest summary ready.</div>
    {:else if sessionSummaryStatus === "no-session"}
      <div class="agent-muted">No summary yet.</div>
    {/if}
  </div>

  <div class="agent-section">
    <div class="section-header">
      <span>Tasks</span>
      {#if sidebarLoading}
        <span class="section-meta">Loading...</span>
      {:else}
        <span class="section-meta">{tasks.length}</span>
      {/if}
    </div>
    {#if sidebarError}
      <div class="agent-error">{sidebarError}</div>
    {:else if !sidebarLoading && tasks.length === 0}
      <div class="agent-muted">No tasks yet.</div>
    {:else if tasks.length > 0}
      <div class="task-list">
        {#each tasks as task}
          <button
            type="button"
            class="task-row"
            class:selected={selectedTask?.id === task.id}
            onclick={() => (selectedTaskId = task.id)}
            data-testid={`agent-task-${task.id}`}
          >
            <div class="task-row-title">{task.title}</div>
            <div class="task-row-meta">
              <span class="task-status {taskStatusClass(task.status)}">{task.status}</span>
              <span class="task-count">{task.sub_agents.length}</span>
            </div>
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <div class="agent-section">
    <div class="section-header">
      <span>Assigned Agents</span>
      <span class="section-meta">
        {selectedTask ? selectedTask.sub_agents.length : 0}
      </span>
    </div>
    {#if !selectedTask}
      <div class="agent-muted">Select a task to view assigned agents.</div>
    {:else if selectedTask.sub_agents.length === 0}
      <div class="agent-muted">No assigned agents.</div>
    {:else}
      <div class="assigned-list">
        {#each selectedTask.sub_agents as assignee}
          <div class="assigned-item">
            <div class="assigned-title">
              <span class="assigned-name">{assignee.name}</span>
              <span class="assigned-status {subAgentStatusClass(assignee.status)}"
                >{assignee.status}</span
              >
            </div>
            <div class="assigned-meta">
              <span class="mono">{assignee.branch}</span>
              {#if assignee.model}
                <span class="mono">{assignee.model}</span>
              {/if}
            </div>
            <div
              class="assigned-path mono"
              title={assignee.worktree_abs_path ?? assignee.worktree_rel_path}
            >
              {assignee.worktree_rel_path}
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .agent-sidebar {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 10px 10px 12px;
    overflow-y: auto;
  }

  .agent-header {
    display: flex;
    justify-content: space-between;
    align-items: start;
    gap: 8px;
  }

  .agent-title {
    font-size: var(--ui-font-md);
    font-weight: 700;
    color: var(--text-primary);
  }

  .agent-branch {
    margin-top: 4px;
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    word-break: break-word;
  }

  .spec-id {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    font-family: monospace;
  }

  .agent-section {
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: var(--ui-font-sm);
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .section-meta {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .status-pill {
    font-size: var(--ui-font-xs);
    padding: 2px 6px;
    border-radius: 999px;
    border: 1px solid var(--border-color);
    text-transform: none;
    letter-spacing: 0;
  }

  .status-pill.ok {
    color: var(--green);
    border-color: rgba(166, 227, 161, 0.35);
  }

  .status-pill.warning {
    color: var(--yellow);
    border-color: rgba(249, 226, 175, 0.35);
  }

  .status-pill.error {
    color: var(--red);
    border-color: rgba(243, 139, 168, 0.35);
  }

  .status-pill.muted {
    color: var(--text-muted);
  }

  .agent-muted {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
  }

  .agent-warning {
    font-size: var(--ui-font-sm);
    color: var(--yellow);
  }

  .agent-error {
    font-size: var(--ui-font-sm);
    color: rgb(255, 160, 160);
  }

  .task-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .task-row {
    width: 100%;
    border: 1px solid var(--border-color);
    background: var(--bg-secondary);
    color: var(--text-primary);
    padding: 8px;
    border-radius: 6px;
    text-align: left;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .task-row.selected {
    border-color: rgba(137, 180, 250, 0.5);
    background: rgba(137, 180, 250, 0.12);
  }

  .task-row-title {
    font-size: var(--ui-font-sm);
    line-height: 1.3;
    word-break: break-word;
  }

  .task-row-meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .task-status {
    font-size: var(--ui-font-xs);
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .task-status.running {
    color: var(--cyan);
  }

  .task-status.pending {
    color: var(--yellow);
  }

  .task-status.failed {
    color: var(--red);
  }

  .task-status.completed {
    color: var(--green);
  }

  .task-count {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .assigned-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .assigned-item {
    padding: 8px;
    border-radius: 6px;
    border: 1px solid var(--border-color);
    background: var(--bg-secondary);
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .assigned-title {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }

  .assigned-name {
    font-size: var(--ui-font-sm);
    color: var(--text-primary);
    font-weight: 600;
  }

  .assigned-status {
    font-size: var(--ui-font-xs);
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .assigned-status.running {
    color: var(--cyan);
  }

  .assigned-status.completed {
    color: var(--green);
  }

  .assigned-status.failed {
    color: var(--red);
  }

  .assigned-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    color: var(--text-muted);
    font-size: var(--ui-font-xs);
  }

  .assigned-path {
    color: var(--text-secondary);
    font-size: var(--ui-font-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .mono {
    font-family: monospace;
  }
</style>
