<script lang="ts">
  import type { Tab, BranchInfo, ProjectInfo, LaunchAgentRequest } from "./lib/types";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import MainArea from "./lib/components/MainArea.svelte";
  import StatusBar from "./lib/components/StatusBar.svelte";
  import OpenProject from "./lib/components/OpenProject.svelte";
  import AgentLaunchForm from "./lib/components/AgentLaunchForm.svelte";

  interface MenuActionPayload {
    action: string;
  }

  let projectPath: string | null = $state(null);
  let sidebarVisible: boolean = $state(true);
  let showAgentLaunch: boolean = $state(false);
  let showAbout: boolean = $state(false);
  let appError: string | null = $state(null);
  let sidebarRefreshKey: number = $state(0);
  let worktreesEventAvailable: boolean = $state(false);

  let selectedBranch: BranchInfo | null = $state(null);
  let currentBranch: string = $state("");

  let tabs: Tab[] = $state([
    { id: "summary", label: "Session Summary", type: "summary" },
  ]);
  let activeTabId: string = $state("summary");

  let terminalCount = $derived(tabs.filter((t) => t.type === "agent").length);

  $effect(() => {
    void projectPath;
    void setWindowTitle();
  });

  // Best-effort: subscribe once and refresh Sidebar when worktrees change.
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenFn = await listen<unknown>("worktrees-changed", (event) => {
          if (!projectPath) return;

          // If payload includes a project_path, only refresh the active project.
          const p = (event as { payload?: unknown }).payload;
          if (p && typeof p === "object" && "project_path" in p) {
            const raw = (p as { project_path?: unknown }).project_path;
            if (typeof raw === "string" && raw && raw !== projectPath) return;
          }

          sidebarRefreshKey++;
        });

        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
        worktreesEventAvailable = true;
      } catch {
        worktreesEventAvailable = false;
      }
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  });

  // Best-effort: close agent tabs when the backend closes the pane.
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenFn = await listen<{ pane_id: string }>(
          "terminal-closed",
          (event) => {
            removeTabLocal(`agent-${event.payload.pane_id}`);
          }
        );

        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch (err) {
        console.error("Failed to setup terminal closed listener:", err);
      }
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  });

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  async function setWindowTitle() {
    const title = projectPath ? `gwt - ${projectPath}` : "gwt";

    // Document title also covers non-tauri contexts (e.g. web preview).
    document.title = title;

    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().setTitle(title);
    } catch {
      // Ignore: title API not available outside Tauri runtime.
    }
  }

  function handleProjectOpen(path: string) {
    projectPath = path;
    fetchCurrentBranch();
  }

  function openSessionSummaryTab() {
    const existing = tabs.find((t) => t.type === "summary" || t.id === "summary");
    if (existing) {
      activeTabId = existing.id;
      return;
    }

    const tab: Tab = { id: "summary", label: "Session Summary", type: "summary" };
    tabs = [tab, ...tabs];
    activeTabId = tab.id;
  }

  function handleBranchSelect(branch: BranchInfo) {
    selectedBranch = branch;
    if (branch.is_current) {
      currentBranch = branch.name;
    }
    // Switch to session summary (re-open tab if it was closed).
    openSessionSummaryTab();
  }

  function requestAgentLaunch() {
    showAgentLaunch = true;
  }

  function handleBranchActivate(branch: BranchInfo) {
    handleBranchSelect(branch);
    requestAgentLaunch();
  }

  async function fetchCurrentBranch() {
    if (!projectPath) return;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const branch = await invoke<BranchInfo | null>("get_current_branch", {
        projectPath,
      });
      if (branch) {
        currentBranch = branch.name;
      }
    } catch (err) {
      console.error("Failed to fetch current branch:", err);
      currentBranch = "";
    }
  }

  function agentTabLabel(agentId: string): string {
    return agentId === "claude"
      ? "Claude Code"
      : agentId === "codex"
        ? "Codex"
        : agentId === "gemini"
          ? "Gemini"
          : agentId === "opencode"
            ? "OpenCode"
            : agentId;
  }

  function normalizeBranchName(name: string): string {
    const trimmed = name.trim();
    return trimmed.startsWith("origin/") ? trimmed.slice("origin/".length) : trimmed;
  }

  function worktreeTabLabel(branch: string): string {
    const b = branch.trim();
    return b ? normalizeBranchName(b) : "Worktree";
  }

  async function handleAgentLaunch(request: LaunchAgentRequest) {
    const { invoke } = await import("@tauri-apps/api/core");
    const paneId = await invoke<string>("launch_agent", { request });

    const newTab: Tab = {
      id: `agent-${paneId}`,
      label: worktreeTabLabel(request.branch),
      type: "agent",
      paneId,
    };

    tabs = [...tabs, newTab];
    activeTabId = newTab.id;

    // Fallback: if the event API is not available, trigger a best-effort refresh.
    if (!worktreesEventAvailable) {
      sidebarRefreshKey++;
    }
  }

  function removeTabLocal(tabId: string) {
    const idx = tabs.findIndex((t) => t.id === tabId);
    if (idx < 0) return;

    const nextTabs = tabs.filter((t) => t.id !== tabId);
    tabs = nextTabs;

    if (activeTabId !== tabId) return;
    const fallback =
      nextTabs[idx] ?? nextTabs[idx - 1] ?? nextTabs[nextTabs.length - 1] ?? null;
    activeTabId = fallback?.id ?? "";
  }

  async function handleTabClose(tabId: string) {
    const tab = tabs.find((t) => t.id === tabId);
    if (tab?.paneId) {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        await invoke("close_terminal", { paneId: tab.paneId });
      } catch {
        // Dev mode: ignore
      }
    }

    removeTabLocal(tabId);
  }

  function handleTabSelect(tabId: string) {
    activeTabId = tabId;
  }

  function openSettingsTab() {
    const existing = tabs.find((t) => t.type === "settings" || t.id === "settings");
    if (existing) {
      activeTabId = existing.id;
      return;
    }

    const tab: Tab = { id: "settings", label: "Settings", type: "settings" };
    tabs = [...tabs, tab];
    activeTabId = tab.id;
  }

  async function handleMenuAction(action: string) {
    switch (action) {
      case "open-project": {
        try {
          const { open } = await import("@tauri-apps/plugin-dialog");
          const selected = await open({ directory: true, multiple: false });
          if (selected) {
            const { invoke } = await import("@tauri-apps/api/core");
            const info = await invoke<ProjectInfo>("open_project", {
              path: selected as string,
            });
            projectPath = info.path;
            fetchCurrentBranch();
          }
        } catch (err) {
          appError = `Failed to open project: ${toErrorMessage(err)}`;
        }
        break;
      }
      case "close-project":
        {
          // Clear backend state (window-scoped) best-effort.
          try {
            const { invoke } = await import("@tauri-apps/api/core");
            await invoke("close_project");
          } catch {
            // Ignore: not available outside Tauri runtime.
          }

          projectPath = null;
          tabs = [{ id: "summary", label: "Session Summary", type: "summary" }];
          activeTabId = "summary";
          selectedBranch = null;
          currentBranch = "";
          sidebarRefreshKey = 0;
        }
        break;
      case "toggle-sidebar":
        sidebarVisible = !sidebarVisible;
        break;
      case "launch-agent":
        showAgentLaunch = true;
        break;
      case "open-settings":
        openSettingsTab();
        break;
      case "about":
        showAbout = true;
        break;
      case "list-terminals":
        // Just switch to first terminal tab if any
        {
          const firstAgent = tabs.find((t) => t.type === "agent");
          if (firstAgent) {
            activeTabId = firstAgent.id;
          }
        }
        break;
    }
  }

  // Native menubar integration (Tauri emits "menu-action" to the focused window).
  $effect(() => {
    let unlisten: null | (() => void) = null;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        unlisten = await listen<MenuActionPayload>("menu-action", (event) => {
          void handleMenuAction(event.payload.action);
        });
      } catch {
        // Ignore: not available outside Tauri runtime.
      }
    })();

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  });
</script>

{#if projectPath === null}
  <OpenProject onOpen={handleProjectOpen} />
{:else}
  <div class="app-layout">
    <div class="app-body">
      {#if sidebarVisible}
        <Sidebar
          {projectPath}
          refreshKey={sidebarRefreshKey}
          onBranchSelect={handleBranchSelect}
          onBranchActivate={handleBranchActivate}
        />
      {/if}
      <MainArea
        {tabs}
        {activeTabId}
        {selectedBranch}
        projectPath={projectPath as string}
        onLaunchAgent={requestAgentLaunch}
        onQuickLaunch={handleAgentLaunch}
        onTabSelect={handleTabSelect}
        onTabClose={handleTabClose}
      />
    </div>
    <StatusBar {projectPath} {currentBranch} {terminalCount} />
  </div>
{/if}

{#if showAgentLaunch}
  <AgentLaunchForm
    projectPath={projectPath as string}
    selectedBranch={selectedBranch?.name ?? currentBranch}
    onLaunch={handleAgentLaunch}
    onClose={() => (showAgentLaunch = false)}
  />
{/if}

{#if showAbout}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay" onclick={() => (showAbout = false)}>
    <div class="about-dialog" onclick={(e) => e.stopPropagation()}>
      <h2>gwt</h2>
      <p>Git Worktree Manager</p>
      <p class="about-version">GUI Edition</p>
      <button class="about-close" onclick={() => (showAbout = false)}>
        Close
      </button>
    </div>
  </div>
{/if}

{#if appError}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay" onclick={() => (appError = null)}>
    <div class="error-dialog" onclick={(e) => e.stopPropagation()}>
      <h2>Error</h2>
      <p class="error-text">{appError}</p>
      <button class="about-close" onclick={() => (appError = null)}>
        Close
      </button>
    </div>
  </div>
{/if}

<style>
  .app-layout {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
  }

  .app-body {
    display: flex;
    flex: 1;
    overflow: hidden;
  }

  .overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .about-dialog {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 12px;
    padding: 32px 40px;
    text-align: center;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4);
  }

  .about-dialog h2 {
    font-size: 24px;
    font-weight: 700;
    color: var(--accent);
    margin-bottom: 4px;
  }

  .about-dialog p {
    color: var(--text-secondary);
    font-size: 13px;
  }

  .about-version {
    color: var(--text-muted);
    font-size: 11px;
    margin-top: 4px;
    margin-bottom: 20px;
  }

  .about-close {
    padding: 6px 20px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    cursor: pointer;
    font-family: inherit;
    font-size: 12px;
  }

  .about-close:hover {
    background: var(--bg-hover);
  }

  .error-dialog {
    background: var(--bg-secondary);
    border: 1px solid rgba(255, 90, 90, 0.35);
    border-radius: 12px;
    padding: 28px 32px;
    text-align: center;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4);
    max-width: 560px;
  }

  .error-dialog h2 {
    font-size: 18px;
    font-weight: 800;
    color: rgb(255, 160, 160);
    margin-bottom: 10px;
  }

  .error-text {
    color: var(--text-secondary);
    font-size: 12px;
    line-height: 1.5;
    margin-bottom: 18px;
    white-space: pre-wrap;
  }
</style>
