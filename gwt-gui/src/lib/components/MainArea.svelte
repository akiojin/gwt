<script lang="ts">
  import { onMount } from "svelte";
  import type {
    Tab,
    BranchInfo,
    LaunchAgentRequest,
    ToolSessionEntry,
    SessionSummaryResult,
  } from "../types";
  import TerminalView from "../terminal/TerminalView.svelte";
  import AgentModePanel from "./AgentModePanel.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";
  import GitSection from "./GitSection.svelte";
  import VersionHistoryPanel from "./VersionHistoryPanel.svelte";

  function isAgentTabWithPaneId(tab: Tab): tab is Tab & { paneId: string } {
    return tab.type === "agent" && typeof tab.paneId === "string" && tab.paneId.length > 0;
  }

  let {
    tabs,
    activeTabId,
    selectedBranch,
    projectPath,
    onLaunchAgent,
    onQuickLaunch,
    onTabSelect,
    onTabClose,
  }: {
    tabs: Tab[];
    activeTabId: string;
    selectedBranch: BranchInfo | null;
    projectPath: string;
    onLaunchAgent?: () => void;
    onQuickLaunch?: (request: LaunchAgentRequest) => Promise<void>;
    onTabSelect: (tabId: string) => void;
    onTabClose: (tabId: string) => void;
  } = $props();

  let activeTab = $derived(tabs.find((t) => t.id === activeTabId));
  let agentTabs = $derived(tabs.filter(isAgentTabWithPaneId));
  let showTerminalLayer = $derived(activeTab?.type === "agent");
  let isPinnedTab = (tabType?: Tab["type"]) =>
    tabType === "summary" || tabType === "agentMode";

  let quickStartEntries: ToolSessionEntry[] = $state([]);
  let quickStartLoading: boolean = $state(false);
  let quickStartError: string | null = $state(null);
  let quickLaunchError: string | null = $state(null);
  let quickLaunching: boolean = $state(false);

  let sessionSummaryLoading: boolean = $state(false);
  let sessionSummaryGenerating: boolean = $state(false);
  let sessionSummaryStatus: SessionSummaryResult["status"] | "" = $state("");
  let sessionSummaryMarkdown: string | null = $state(null);
  let sessionSummaryWarning: string | null = $state(null);
  let sessionSummaryError: string | null = $state(null);
  let sessionSummaryToolId: string | null = $state(null);
  let sessionSummarySessionId: string | null = $state(null);

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

  async function loadQuickStart() {
    quickLaunchError = null;
    quickStartError = null;

    const rawBranch = selectedBranch?.name?.trim() ?? "";
    const branch = normalizeBranchName(rawBranch);
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
      // Avoid clobbering if selection changed while loading.
      const currentBranch = normalizeBranchName(selectedBranch?.name?.trim() ?? "");
      const currentKey = `${projectPath}::${currentBranch}`;
      if (currentKey !== key) return;
      quickStartEntries = entries ?? [];
    } catch (err) {
      quickStartEntries = [];
      quickStartError = `Failed to load Quick Start: ${toErrorMessage(err)}`;
    } finally {
      const currentBranch = normalizeBranchName(selectedBranch?.name?.trim() ?? "");
      const currentKey = `${projectPath}::${currentBranch}`;
      if (currentKey === key) {
        quickStartLoading = false;
      }
    }
  }

  $effect(() => {
    void selectedBranch;
    void projectPath;
    loadQuickStart();
  });

  async function loadSessionSummary() {
    sessionSummaryError = null;
    sessionSummaryWarning = null;

    const rawBranch = selectedBranch?.name?.trim() ?? "";
    const branch = normalizeBranchName(rawBranch);
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
    sessionSummaryLoading = true;
    sessionSummaryGenerating = false;
    sessionSummaryStatus = "";
    sessionSummaryMarkdown = null;
    sessionSummaryToolId = null;
    sessionSummarySessionId = null;

    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const result = await invoke<SessionSummaryResult>("get_branch_session_summary", {
        projectPath,
        branch,
      });

      const currentBranch = normalizeBranchName(selectedBranch?.name?.trim() ?? "");
      const currentKey = `${projectPath}::${currentBranch}`;
      if (currentKey !== key) return;

      sessionSummaryStatus = result.status;
      sessionSummaryGenerating = !!result.generating;
      sessionSummaryMarkdown = result.markdown ?? null;
      sessionSummaryWarning = result.warning ?? null;
      sessionSummaryError = result.error ?? null;
      sessionSummaryToolId = result.toolId ?? null;
      sessionSummarySessionId = result.sessionId ?? null;
    } catch (err) {
      sessionSummaryStatus = "error";
      sessionSummaryGenerating = false;
      sessionSummaryMarkdown = null;
      sessionSummaryToolId = null;
      sessionSummarySessionId = null;
      sessionSummaryError = `Failed to generate session summary: ${toErrorMessage(err)}`;
    } finally {
      const currentBranch = normalizeBranchName(selectedBranch?.name?.trim() ?? "");
      const currentKey = `${projectPath}::${currentBranch}`;
      if (currentKey === key) {
        sessionSummaryLoading = false;
      }
    }
  }

  $effect(() => {
    void selectedBranch;
    void projectPath;
    void activeTabId;
    if (activeTab?.type !== "summary") return;
    loadSessionSummary();
  });

  onMount(() => {
    let unlisten: null | (() => void) = null;
    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        unlisten = await listen<SessionSummaryUpdatedPayload>(
          "session-summary-updated",
          (event) => {
            const payload = event.payload;
            if (!payload) return;
            if (payload.projectPath !== projectPath) return;

            const currentBranch = normalizeBranchName(
              selectedBranch?.name?.trim() ?? ""
            );
            if (!currentBranch || payload.branch !== currentBranch) return;

            const result = payload.result;
            const incomingSessionId = result.sessionId ?? null;
            if (!incomingSessionId) return;

            // Drop stale events when the branch's latest session has advanced while a job was running.
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
      } catch (err) {
        console.error("Failed to setup session summary event listener:", err);
      }
    })();

    return () => {
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
    }
  }
</script>

<main class="main-area">
  <div class="tab-bar">
    {#each tabs as tab}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="tab"
        class:active={activeTabId === tab.id}
        onclick={() => onTabSelect(tab.id)}
      >
        {#if tab.type === "agent"}
          <span class="tab-dot"></span>
        {/if}
        <span class="tab-label">{tab.label}</span>
        {#if !isPinnedTab(tab.type)}
          <button
            class="tab-close"
            onclick={(e) => {
              e.stopPropagation();
              onTabClose(tab.id);
            }}
          >
            x
          </button>
        {/if}
      </div>
    {/each}
  </div>
  <div class="tab-content">
      <div class="panel-layer" class:hidden={showTerminalLayer}>
      {#if activeTab?.type === "settings"}
        <SettingsPanel onClose={() => onTabClose(activeTabId)} />
      {:else if activeTab?.type === "summary"}
        <div class="summary-content">
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
                  <span class="detail-value">
                    {selectedBranch.is_current ? "Yes" : "No"}
                  </span>
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
	                    {#each quickStartEntries as entry (entry.tool_id)}
	                      <div class="quick-row">
	                        <div class="quick-info">
                          <div class="quick-tool {toolClass(entry)}">
                            <span class="quick-tool-name">{displayToolName(entry)}</span>
                            <span class="quick-tool-version">
                              @{displayToolVersion(entry)}
                            </span>
                          </div>
                          <div class="quick-meta">
                            {#if entry.model}
                              <span class="quick-pill">model: {entry.model}</span>
                            {/if}
                            {#if toolClass(entry) === "codex" && entry.reasoning_level}
                              <span class="quick-pill">
                                reasoning: {entry.reasoning_level}
                              </span>
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
                            {quickLaunching ? "Launching..." : "Continue"}
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
                  {:else if sessionSummaryStatus === "ok" && sessionSummaryToolId && sessionSummarySessionId}
                    <span class="quick-subtitle">
                      {sessionSummaryToolId} #{sessionSummarySessionId}
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
                  <div class="session-summary-placeholder">
                    Session summary disabled.
                  </div>
                {:else if sessionSummaryStatus === "no-session"}
                  <div class="session-summary-placeholder">No session.</div>
                {:else if sessionSummaryStatus === "error"}
                  <div class="quick-error">
                    {sessionSummaryError ?? "Failed to generate session summary."}
                  </div>
                {:else if sessionSummaryStatus === "ok" && sessionSummaryMarkdown}
                  <pre class="session-summary-markdown">{sessionSummaryMarkdown}</pre>
                {:else}
                  <div class="session-summary-placeholder">No summary.</div>
                {/if}
              </div>

              <GitSection projectPath={projectPath} branch={selectedBranch.name} />
            </div>
          {:else}
            <div class="placeholder">
              <h2>Session Summary</h2>
              <p>Select a branch to view details.</p>
            </div>
          {/if}
        </div>
      {:else if activeTab?.type === "versionHistory"}
        <VersionHistoryPanel {projectPath} />
      {:else if activeTab?.type === "agentMode"}
        <AgentModePanel />
      {/if}
    </div>

    <div class="terminal-layer" class:hidden={!showTerminalLayer}>
      {#each agentTabs as tab (tab.id)}
        <div class="terminal-wrapper" class:active={activeTabId === tab.id}>
          <TerminalView paneId={tab.paneId} active={activeTabId === tab.id} />
        </div>
      {/each}
    </div>
  </div>
</main>

<style>
  .main-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background-color: var(--bg-primary);
  }

  .tab-bar {
    display: flex;
    height: var(--tab-height);
    background-color: var(--bg-secondary);
    border-bottom: 1px solid var(--border-color);
    overflow-x: auto;
    scrollbar-width: none;
  }

  .tab-bar::-webkit-scrollbar {
    display: none;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 16px;
    background: none;
    border: none;
    border-right: 1px solid var(--border-color);
    color: var(--text-secondary);
    font-size: var(--ui-font-md);
    cursor: pointer;
    white-space: nowrap;
    font-family: inherit;
  }

  .tab:hover {
    color: var(--text-primary);
    background-color: var(--bg-hover);
  }

  .tab.active {
    color: var(--text-primary);
    background-color: var(--bg-primary);
    border-bottom: 2px solid var(--accent);
  }

  .tab-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background-color: var(--green);
    flex-shrink: 0;
  }

  .tab-label {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tab-close {
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
    font-family: monospace;
    cursor: pointer;
    padding: 0 2px;
    line-height: 1;
    flex-shrink: 0;
  }

  .tab-close:hover {
    color: var(--red);
  }

  .tab-content {
    flex: 1;
    position: relative;
    overflow: hidden;
  }

  .panel-layer {
    position: absolute;
    inset: 0;
    overflow: auto;
    padding: 24px;
    z-index: 2;
  }

  .terminal-layer {
    position: absolute;
    inset: 0;
    overflow: hidden;
    z-index: 1;
  }

  .panel-layer.hidden,
  .terminal-layer.hidden {
    visibility: hidden;
    pointer-events: none;
  }

  .terminal-wrapper {
    position: absolute;
    inset: 0;
    visibility: hidden;
    pointer-events: none;
  }

  .terminal-wrapper.active {
    visibility: visible;
    pointer-events: auto;
  }

  .summary-content {
    height: 100%;
  }

  .placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-muted);
  }

  .placeholder h2 {
    font-size: var(--ui-font-2xl);
    font-weight: 500;
    margin-bottom: 8px;
    color: var(--text-secondary);
  }

  .branch-detail {
    max-width: 600px;
  }

  .branch-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 20px;
  }

  .branch-detail h2 {
    font-size: var(--ui-font-3xl);
    font-weight: 600;
    color: var(--text-primary);
    font-family: monospace;
  }

  .launch-btn {
    background: var(--accent);
    color: var(--bg-primary);
    border: none;
    border-radius: 8px;
    padding: 8px 12px;
    font-size: var(--ui-font-md);
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    white-space: nowrap;
  }

  .launch-btn:hover {
    background: var(--accent-hover);
  }

  .detail-grid {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .detail-item {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }

  .detail-label {
    font-size: var(--ui-font-sm);
    font-weight: 500;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    min-width: 80px;
  }

  .detail-value {
    font-size: var(--ui-font-base);
    color: var(--text-primary);
  }

  .detail-value.mono {
    font-family: monospace;
  }

  .quick-start {
    margin-top: 16px;
    border: 1px solid var(--border-color);
    border-radius: 12px;
    background: var(--bg-secondary);
    padding: 14px;
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
    font-size: var(--ui-font-md);
    font-weight: 700;
    letter-spacing: 0.5px;
    text-transform: uppercase;
    color: var(--text-secondary);
  }

  .quick-subtitle {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    font-family: monospace;
  }

  .quick-error {
    padding: 10px 12px;
    border: 1px solid rgba(255, 0, 0, 0.35);
    background: rgba(255, 0, 0, 0.08);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: var(--ui-font-md);
    line-height: 1.4;
  }

  .quick-empty {
    font-size: var(--ui-font-md);
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
    align-items: center;
    justify-content: space-between;
    gap: 12px;
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
    font-size: var(--ui-font-base);
    font-weight: 700;
  }

  .quick-tool-version {
    font-size: var(--ui-font-sm);
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
    font-size: var(--ui-font-sm);
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
    flex-shrink: 0;
  }

  .quick-btn {
    padding: 8px 10px;
    border-radius: 8px;
    border: 1px solid var(--border-color);
    background: var(--bg-surface);
    color: var(--text-primary);
    font-size: var(--ui-font-md);
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
</style>
