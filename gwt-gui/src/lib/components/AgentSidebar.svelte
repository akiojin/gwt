<script lang="ts">
  import type { BranchInfo, ToolSessionEntry, SessionSummaryResult } from "../types";

  let {
    projectPath,
    selectedBranch = null,
    currentBranch = "",
  }: {
    projectPath: string;
    selectedBranch?: BranchInfo | null;
    currentBranch?: string;
  } = $props();

  let quickStartEntries: ToolSessionEntry[] = $state([]);
  let quickStartLoading: boolean = $state(false);
  let quickStartError: string | null = $state(null);

  let sessionSummaryLoading: boolean = $state(false);
  let sessionSummaryGenerating: boolean = $state(false);
  let sessionSummaryStatus: SessionSummaryResult["status"] | "" = $state("");
  let sessionSummaryWarning: string | null = $state(null);
  let sessionSummaryError: string | null = $state(null);
  let sessionSummaryToolId: string | null = $state(null);
  let sessionSummarySessionId: string | null = $state(null);
  const SESSION_SUMMARY_POLL_INTERVAL_MS = 5000;

  function normalizeBranchName(name: string): string {
    const trimmed = name.trim();
    return trimmed.startsWith("origin/") ? trimmed.slice("origin/".length) : trimmed;
  }

  function activeBranchName(): string {
    const raw = selectedBranch?.name?.trim() ?? currentBranch?.trim() ?? "";
    return normalizeBranchName(raw);
  }

  let activeBranch = $derived(activeBranchName());

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  function toolClass(entry: ToolSessionEntry): string {
    const id = (entry.tool_id ?? entry.tool_label ?? "").toLowerCase();
    if (id.includes("claude")) return "claude";
    if (id.includes("codex")) return "codex";
    if (id.includes("gemini")) return "gemini";
    if (id.includes("opencode") || id.includes("open-code")) return "opencode";
    return "";
  }

  function displayToolName(entry: ToolSessionEntry): string {
    const id = (entry.tool_id ?? entry.tool_label ?? "").toLowerCase();
    if (id.includes("claude")) return "Claude";
    if (id.includes("codex")) return "Codex";
    if (id.includes("gemini")) return "Gemini";
    if (id.includes("opencode") || id.includes("open-code")) return "OpenCode";
    return entry.tool_label || entry.tool_id || "Agent";
  }

  function displayToolVersion(entry: ToolSessionEntry): string {
    const v = entry.tool_version?.trim();
    return v && v.length > 0 ? v : "latest";
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

  async function loadQuickStart() {
    quickStartError = null;
    const branch = activeBranchName();
    if (!branch) {
      quickStartEntries = [];
      quickStartLoading = false;
      return;
    }

    const key = `${projectPath}::${branch}`;
    quickStartLoading = true;

    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const entries = await invoke<ToolSessionEntry[]>("get_branch_quick_start", {
        projectPath,
        branch,
      });
      if (`${projectPath}::${activeBranchName()}` !== key) return;
      quickStartEntries = entries ?? [];
    } catch (err) {
      quickStartEntries = [];
      quickStartError = `Failed to load tasks: ${toErrorMessage(err)}`;
    } finally {
      if (`${projectPath}::${activeBranchName()}` === key) {
        quickStartLoading = false;
      }
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
      sessionSummarySessionId = null;
      return;
    }

    const key = `${projectPath}::${branch}`;
    if (!silent) {
      sessionSummaryLoading = true;
      sessionSummaryGenerating = false;
      sessionSummaryStatus = "";
      sessionSummaryToolId = null;
      sessionSummarySessionId = null;
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
      sessionSummarySessionId = result.sessionId ?? null;
    } catch (err) {
      sessionSummaryStatus = "error";
      sessionSummaryGenerating = false;
      sessionSummaryToolId = null;
      sessionSummarySessionId = null;
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
    loadQuickStart();
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
  </div>

  {#if activeBranch}
    <div class="agent-section">
      <div class="section-header">
        <span>Summary</span>
        <span class="status-pill {summaryStatusClass(sessionSummaryStatus, sessionSummaryGenerating)}">
          {summaryStatusLabel(sessionSummaryStatus, sessionSummaryGenerating)}
        </span>
      </div>
      {#if sessionSummaryLoading}
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
        {#if quickStartLoading}
          <span class="section-meta">Loading...</span>
        {:else if quickStartEntries.length > 0}
          <span class="section-meta">{quickStartEntries.length}</span>
        {/if}
      </div>
      {#if quickStartError}
        <div class="agent-error">{quickStartError}</div>
      {:else if !quickStartLoading && quickStartEntries.length === 0}
        <div class="agent-muted">No tasks yet.</div>
      {:else if quickStartEntries.length > 0}
        <div class="task-list">
          {#each quickStartEntries as entry (entry.session_id ?? entry.tool_id ?? String(entry.timestamp))}
            <div class="task-item">
              <div class="task-title">
                <span class="task-tool {toolClass(entry)}">{displayToolName(entry)}</span>
                <span class="task-version">@{displayToolVersion(entry)}</span>
              </div>
              {#if entry.model}
                <div class="task-meta">model: {entry.model}</div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
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
    align-items: center;
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
    gap: 8px;
  }

  .task-item {
    padding: 8px;
    border-radius: 6px;
    border: 1px solid var(--border-color);
    background: var(--bg-secondary);
  }

  .task-title {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: var(--ui-font-sm);
    color: var(--text-primary);
  }

  .task-tool {
    font-weight: 600;
  }

  .task-tool.claude {
    color: var(--yellow);
  }

  .task-tool.codex {
    color: var(--cyan);
  }

  .task-tool.gemini {
    color: var(--magenta);
  }

  .task-tool.opencode {
    color: var(--green);
  }

  .task-version {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    font-family: monospace;
  }

  .task-meta {
    margin-top: 4px;
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    font-family: monospace;
  }
</style>
