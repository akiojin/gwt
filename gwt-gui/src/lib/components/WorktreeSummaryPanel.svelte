<script lang="ts">
  import { onMount } from "svelte";
  import type {
    BranchInfo,
    LaunchAgentRequest,
    ToolSessionEntry,
    SessionSummaryResult,
  } from "../types";
  import GitSection from "./GitSection.svelte";
  import MarkdownRenderer from "./MarkdownRenderer.svelte";

  let {
    projectPath,
    selectedBranch = null,
    onLaunchAgent,
    onQuickLaunch,
  }: {
    projectPath: string;
    selectedBranch?: BranchInfo | null;
    onLaunchAgent?: () => void;
    onQuickLaunch?: (request: LaunchAgentRequest) => Promise<void>;
  } = $props();

  let quickStartEntries: ToolSessionEntry[] = $state([]);
  let quickStartLoading: boolean = $state(false);
  let quickStartError: string | null = $state(null);
  let quickLaunchError: string | null = $state(null);
  let quickLaunching: boolean = $state(false);
  let quickLaunchingKey: string | null = $state(null);

  let sessionSummaryLoading: boolean = $state(false);
  let sessionSummaryGenerating: boolean = $state(false);
  let sessionSummaryStatus: SessionSummaryResult["status"] | "" = $state("");
  let sessionSummaryMarkdown: string | null = $state(null);
  let sessionSummaryWarning: string | null = $state(null);
  let sessionSummaryError: string | null = $state(null);
  let sessionSummaryToolId: string | null = $state(null);
  let sessionSummarySessionId: string | null = $state(null);
  const SESSION_SUMMARY_POLL_INTERVAL_MS = 5000;

  type SessionSummaryUpdatedPayload = {
    projectPath: string;
    branch: string;
    result: SessionSummaryResult;
  };

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  function normalizeBranchName(name: string): string {
    return name.startsWith("origin/") ? name.slice("origin/".length) : name;
  }

  function currentBranchName(): string {
    const rawBranch = selectedBranch?.name?.trim() ?? "";
    return normalizeBranchName(rawBranch);
  }

  function agentIdForToolId(toolId: string): LaunchAgentRequest["agentId"] {
    const key = (toolId ?? "").toLowerCase();
    if (key.includes("claude")) return "claude";
    if (key.includes("codex")) return "codex";
    if (key.includes("gemini")) return "gemini";
    if (key.includes("opencode") || key.includes("open-code")) return "opencode";
    return toolId as LaunchAgentRequest["agentId"];
  }

  function toolClass(entry: ToolSessionEntry): string {
    const id = entry.tool_id?.toLowerCase() ?? "";
    if (id.includes("claude")) return "claude";
    if (id.includes("codex")) return "codex";
    if (id.includes("gemini")) return "gemini";
    if (id.includes("opencode") || id.includes("open-code")) return "opencode";
    return "";
  }

  function displayToolName(entry: ToolSessionEntry): string {
    const id = entry.tool_id?.toLowerCase() ?? "";
    if (id.includes("claude")) return "Claude";
    if (id.includes("codex")) return "Codex";
    if (id.includes("gemini")) return "Gemini";
    if (id.includes("opencode") || id.includes("open-code")) return "OpenCode";
    return entry.tool_label || entry.tool_id;
  }

  function displayToolVersion(entry: ToolSessionEntry): string {
    const v = entry.tool_version?.trim();
    return v && v.length > 0 ? v : "latest";
  }

  function displayModelLabel(entry: ToolSessionEntry): string | null {
    const model = entry.model?.trim();
    if (model) return model;
    const tool = entry.tool_id?.toLowerCase() ?? "";
    if (
      tool.includes("codex") ||
      tool.includes("claude") ||
      tool.includes("gemini") ||
      tool.includes("opencode") ||
      tool.includes("open-code")
    ) {
      return "default";
    }
    return null;
  }

  function quickStartEntryKey(entry: ToolSessionEntry): string {
    const session = entry.session_id?.trim();
    if (session) return session;
    return `${entry.tool_id}-${entry.timestamp}`;
  }

  async function loadQuickStart() {
    quickLaunchError = null;
    quickStartError = null;

    const branch = currentBranchName();
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
      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey !== key) return;
      quickStartEntries = entries ?? [];
    } catch (err) {
      quickStartEntries = [];
      quickStartError = `Failed to load Quick Start: ${toErrorMessage(err)}`;
    } finally {
      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey === key) {
        quickStartLoading = false;
      }
    }
  }

  async function loadSessionSummary(options: { silent?: boolean } = {}) {
    const silent = options.silent === true;
    sessionSummaryError = null;
    sessionSummaryWarning = null;

    const branch = currentBranchName();
    if (!branch) {
      sessionSummaryLoading = false;
      sessionSummaryGenerating = false;
      sessionSummaryStatus = "";
      sessionSummaryMarkdown = null;
      sessionSummaryToolId = null;
      sessionSummarySessionId = null;
      return;
    }

    const key = `${projectPath}::${branch}`;
    if (!silent) {
      sessionSummaryLoading = true;
      sessionSummaryGenerating = false;
      sessionSummaryStatus = "";
      sessionSummaryMarkdown = null;
      sessionSummaryToolId = null;
      sessionSummarySessionId = null;
    }

    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const result = await invoke<SessionSummaryResult>("get_branch_session_summary", {
        projectPath,
        branch,
      });

      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey !== key) return;

      sessionSummaryStatus = result.status;
      sessionSummaryGenerating = !!result.generating;
      const nextMarkdown = result.markdown ?? null;
      if (nextMarkdown !== null) {
        sessionSummaryMarkdown = nextMarkdown;
      } else if (!silent || result.status !== "ok") {
        sessionSummaryMarkdown = null;
      }
      sessionSummaryWarning = result.warning ?? null;
      sessionSummaryError = result.error ?? null;
      sessionSummaryToolId = result.toolId ?? null;
      sessionSummarySessionId = result.sessionId ?? null;
    } catch (err) {
      sessionSummaryStatus = "error";
      sessionSummaryGenerating = false;
      if (!silent) {
        sessionSummaryMarkdown = null;
      }
      sessionSummaryToolId = null;
      sessionSummarySessionId = null;
      sessionSummaryError = `Failed to generate session summary: ${toErrorMessage(err)}`;
    } finally {
      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey === key && !silent) {
        sessionSummaryLoading = false;
      }
    }
  }

  $effect(() => {
    void selectedBranch;
    void projectPath;
    loadQuickStart();
  });

  $effect(() => {
    void selectedBranch;
    void projectPath;

    const branch = currentBranchName();
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

  onMount(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;
    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenFn = await listen<SessionSummaryUpdatedPayload>(
          "session-summary-updated",
          (event) => {
            const payload = event.payload;
            if (!payload) return;
            if (payload.projectPath !== projectPath) return;

            const currentBranch = currentBranchName();
            if (!currentBranch || payload.branch !== currentBranch) return;

            const result = payload.result;
            const incomingSessionId = result.sessionId ?? null;
            if (!incomingSessionId) return;

            const currentSessionId = sessionSummarySessionId ?? null;
            if (currentSessionId && incomingSessionId !== currentSessionId) return;

            sessionSummaryStatus = result.status;
            sessionSummaryGenerating = !!result.generating;
            sessionSummaryMarkdown = result.markdown ?? null;
            sessionSummaryWarning = result.warning ?? null;
            sessionSummaryError = result.error ?? null;
            sessionSummaryToolId = result.toolId ?? null;
            sessionSummarySessionId = result.sessionId ?? null;
          }
        );
        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch (err) {
        // Ignore when Tauri event bridge is unavailable (e.g., tests/web preview).
      }
    })();

    return () => {
      cancelled = true;
      if (unlisten) {
        unlisten();
      }
    };
  });

  async function quickLaunch(entry: ToolSessionEntry, action: "continue" | "new") {
    if (!selectedBranch) return;
    if (!onQuickLaunch) return;
    if (quickLaunching) return;

    quickLaunchError = null;
    quickLaunching = true;
    quickLaunchingKey = quickStartEntryKey(entry);
    try {
      const agentId = agentIdForToolId(entry.tool_id);
      const mode = action === "continue" ? "continue" : "normal";
      const resumeSessionId =
        action === "continue"
          ? entry.session_id?.trim() || undefined
          : undefined;

      const request: LaunchAgentRequest = {
        agentId,
        branch: selectedBranch.name,
        mode,
        resumeSessionId,
        model: entry.model?.trim() || undefined,
        agentVersion: displayToolVersion(entry),
        skipPermissions: entry.skip_permissions ?? undefined,
        reasoningLevel: entry.reasoning_level?.trim() || undefined,
        dockerService: entry.docker_service?.trim() || undefined,
        dockerForceHost: entry.docker_force_host ?? undefined,
        dockerRecreate: entry.docker_recreate ?? undefined,
        dockerBuild: entry.docker_build ?? undefined,
        dockerKeep: entry.docker_keep ?? undefined,
      };

      await onQuickLaunch(request);
    } catch (err) {
      quickLaunchError = `Failed to launch: ${toErrorMessage(err)}`;
    } finally {
      quickLaunching = false;
      quickLaunchingKey = null;
    }
  }
</script>

<div class="worktree-summary-panel">
  {#if selectedBranch}
    <div class="branch-detail">
      <div class="branch-header">
        <h2>{selectedBranch.name}</h2>
        <button class="launch-btn" onclick={() => onLaunchAgent?.()}>
          Launch Agent...
        </button>
      </div>

      <div class="detail-grid">
        <div class="detail-item">
          <span class="detail-label">Commit</span>
          <span class="detail-value mono">{selectedBranch.commit}</span>
        </div>
        <div class="detail-item">
          <span class="detail-label">Status</span>
          <span class="detail-value">
            {selectedBranch.divergence_status}
            {#if selectedBranch.ahead > 0}
              (+{selectedBranch.ahead})
            {/if}
            {#if selectedBranch.behind > 0}
              (-{selectedBranch.behind})
            {/if}
          </span>
        </div>
        <div class="detail-item">
          <span class="detail-label">Current</span>
          <span class="detail-value">{selectedBranch.is_current ? "Yes" : "No"}</span>
        </div>
      </div>

      <div class="quick-start">
        <div class="quick-header">
          <span class="quick-title">Quick Start</span>
          {#if quickStartLoading}
            <span class="quick-subtitle">Loading...</span>
          {:else if quickStartEntries.length > 0}
            <span class="quick-subtitle">
              {quickStartEntries.length} tool{quickStartEntries.length === 1 ? "" : "s"}
            </span>
          {:else}
            <span class="quick-subtitle">No history</span>
          {/if}
        </div>

        {#if quickStartError}
          <div class="quick-error">{quickStartError}</div>
        {/if}

        {#if quickLaunchError}
          <div class="quick-error">{quickLaunchError}</div>
        {/if}

        {#if !quickStartLoading && quickStartEntries.length === 0}
          <div class="quick-empty">
            Launch an agent once on this branch to enable Quick Start.
          </div>
        {:else if quickStartEntries.length > 0}
          <div class="quick-list">
            {#each quickStartEntries as entry (quickStartEntryKey(entry))}
              <div class="quick-row">
                <div class="quick-info">
                  <div class="quick-tool {toolClass(entry)}">
                    <span class="quick-tool-name">{displayToolName(entry)}</span>
                    <span class="quick-tool-version">
                      @{displayToolVersion(entry)}
                    </span>
                  </div>
                  <div class="quick-meta">
                    {#if displayModelLabel(entry) !== null}
                      <span class="quick-pill">model: {displayModelLabel(entry)}</span>
                    {/if}
                    {#if toolClass(entry) === "codex" && entry.reasoning_level}
                      <span class="quick-pill">reasoning: {entry.reasoning_level}</span>
                    {/if}
                    {#if entry.skip_permissions !== undefined && entry.skip_permissions !== null}
                      <span class="quick-pill">
                        skip: {entry.skip_permissions ? "on" : "off"}
                      </span>
                    {/if}
                  </div>
                </div>
                <div class="quick-actions">
                  <button
                    class="quick-btn"
                    disabled={quickLaunching}
                    onclick={() => quickLaunch(entry, "continue")}
                  >
                    {quickLaunching && quickLaunchingKey === quickStartEntryKey(entry)
                      ? "Launching..."
                      : "Continue"}
                  </button>
                  <button
                    class="quick-btn ghost"
                    disabled={quickLaunching}
                    onclick={() => quickLaunch(entry, "new")}
                  >
                    New
                  </button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>

      <div class="quick-start ai-summary">
        <div class="quick-header">
          <span class="quick-title">AI Summary</span>
          {#if sessionSummaryLoading}
            <span class="quick-subtitle">Loading...</span>
          {:else if sessionSummaryStatus === "ok" && sessionSummaryToolId}
            <span class="quick-subtitle">
              {#if sessionSummarySessionId?.startsWith("pane:")}
                {sessionSummaryToolId} - Live (pane summary)
              {:else if sessionSummarySessionId}
                {sessionSummaryToolId} #{sessionSummarySessionId}
              {:else}
                {sessionSummaryToolId}
              {/if}
              {#if sessionSummaryGenerating}
                {sessionSummaryMarkdown ? " - Updating..." : " - Generating..."}
              {/if}
            </span>
          {:else if sessionSummaryStatus === "ai-not-configured"}
            <span class="quick-subtitle">AI not configured</span>
          {:else if sessionSummaryStatus === "disabled"}
            <span class="quick-subtitle">Disabled</span>
          {:else if sessionSummaryStatus === "no-session"}
            <span class="quick-subtitle">No session</span>
          {:else if sessionSummaryStatus === "error"}
            <span class="quick-subtitle">Error</span>
          {/if}
        </div>

        {#if sessionSummaryWarning}
          <div class="session-summary-warning">
            {sessionSummaryWarning}
          </div>
        {/if}

        {#if sessionSummaryLoading}
          <div class="session-summary-placeholder">Loading...</div>
        {:else if sessionSummaryStatus === "ok" && sessionSummaryGenerating && !sessionSummaryMarkdown}
          <div class="session-summary-placeholder">Generating...</div>
        {:else if sessionSummaryStatus === "ai-not-configured"}
          <div class="session-summary-placeholder">
            Configure AI in Settings to enable session summary.
          </div>
        {:else if sessionSummaryStatus === "disabled"}
          <div class="session-summary-placeholder">Session summary disabled.</div>
        {:else if sessionSummaryStatus === "no-session"}
          <div class="session-summary-placeholder">No session.</div>
        {:else if sessionSummaryStatus === "error"}
          <div class="quick-error">
            {sessionSummaryError ?? "Failed to generate session summary."}
          </div>
        {:else if sessionSummaryStatus === "ok" && sessionSummaryMarkdown}
          <MarkdownRenderer
            className="session-summary-markdown"
            text={sessionSummaryMarkdown}
          />
        {:else}
          <div class="session-summary-placeholder">No summary.</div>
        {/if}
      </div>

      <GitSection projectPath={projectPath} branch={selectedBranch.name} />
    </div>
  {:else}
    <div class="placeholder">
      <h2>Worktree Summary</h2>
      <p>Select a branch to view details.</p>
    </div>
  {/if}
</div>

<style>
  .worktree-summary-panel {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    color: var(--text-muted);
    text-align: center;
    min-height: 120px;
  }

  .placeholder h2 {
    font-size: var(--ui-font-lg);
    font-weight: 600;
    color: var(--text-secondary);
  }

  .placeholder p {
    font-size: var(--ui-font-sm);
  }

  .branch-detail {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .branch-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  .branch-detail h2 {
    font-size: var(--ui-font-lg);
    font-weight: 700;
    color: var(--text-primary);
    font-family: monospace;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .launch-btn {
    background: var(--accent);
    color: var(--bg-primary);
    border: none;
    border-radius: 8px;
    padding: 6px 10px;
    font-size: var(--ui-font-sm);
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .launch-btn:hover {
    background: var(--accent-hover);
  }

  .detail-grid {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .detail-item {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }

  .detail-label {
    font-size: var(--ui-font-xs);
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    min-width: 56px;
    flex-shrink: 0;
  }

  .detail-value {
    font-size: var(--ui-font-sm);
    color: var(--text-primary);
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .detail-value.mono {
    font-family: monospace;
  }

  .quick-start {
    border: 1px solid var(--border-color);
    border-radius: 12px;
    background: var(--bg-secondary);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .quick-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  .quick-title {
    font-size: var(--ui-font-sm);
    font-weight: 700;
    letter-spacing: 0.5px;
    text-transform: uppercase;
    color: var(--text-secondary);
  }

  .quick-subtitle {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    font-family: monospace;
    text-align: right;
  }

  .quick-error {
    padding: 10px 12px;
    border: 1px solid rgba(255, 0, 0, 0.35);
    background: rgba(255, 0, 0, 0.08);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: var(--ui-font-sm);
    line-height: 1.4;
  }

  .session-summary-warning {
    padding: 10px 12px;
    border: 1px solid rgba(249, 226, 175, 0.35);
    background: rgba(249, 226, 175, 0.08);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: var(--ui-font-sm);
    line-height: 1.4;
  }

  .session-summary-placeholder {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    line-height: 1.4;
  }

  .quick-empty {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    line-height: 1.4;
  }

  .quick-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .quick-row {
    display: flex;
    flex-direction: column;
    gap: 10px;
    border: 1px solid var(--border-color);
    background: var(--bg-primary);
    border-radius: 10px;
    padding: 10px 12px;
  }

  .quick-info {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .quick-tool {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-family: monospace;
    min-width: 0;
  }

  .quick-tool-name {
    font-size: var(--ui-font-sm);
    font-weight: 700;
  }

  .quick-tool-version {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .quick-tool.claude .quick-tool-name {
    color: var(--yellow);
  }

  .quick-tool.codex .quick-tool-name {
    color: var(--cyan);
  }

  .quick-tool.gemini .quick-tool-name {
    color: var(--magenta);
  }

  .quick-tool.opencode .quick-tool-name {
    color: var(--green);
  }

  .quick-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    color: var(--text-muted);
    font-size: var(--ui-font-xs);
  }

  .quick-pill {
    border: 1px solid var(--border-color);
    background: var(--bg-secondary);
    border-radius: 999px;
    padding: 2px 8px;
    font-family: monospace;
  }

  .quick-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    justify-content: flex-end;
    flex-wrap: wrap;
  }

  .quick-btn {
    padding: 7px 10px;
    border-radius: 8px;
    border: 1px solid var(--border-color);
    background: var(--bg-surface);
    color: var(--text-primary);
    font-size: var(--ui-font-sm);
    font-weight: 700;
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s, background-color 0.15s;
  }

  .quick-btn:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .quick-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .quick-btn.ghost {
    background: transparent;
    color: var(--text-secondary);
  }

  .session-summary-markdown {
    border: 1px solid var(--border-color);
    border-radius: 10px;
    background: var(--bg-primary);
    padding: 10px 12px;
    overflow: hidden;
    margin: 0;
  }
</style>
