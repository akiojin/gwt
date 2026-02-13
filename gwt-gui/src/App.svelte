<script lang="ts">
  import type {
    Tab,
    BranchInfo,
    ProjectInfo,
    LaunchAgentRequest,
    LaunchFinishedPayload,
    ProbePathResult,
    RollbackResult,
    TerminalInfo,
    TerminalAnsiProbe,
    CapturedEnvInfo,
    SettingsData,
    UpdateState,
    VoiceInputSettings,
  } from "./lib/types";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import MainArea from "./lib/components/MainArea.svelte";
  import StatusBar from "./lib/components/StatusBar.svelte";
  import OpenProject from "./lib/components/OpenProject.svelte";
  import AgentLaunchForm from "./lib/components/AgentLaunchForm.svelte";
  import LaunchProgressModal from "./lib/components/LaunchProgressModal.svelte";
  import MigrationModal from "./lib/components/MigrationModal.svelte";
  import CleanupModal from "./lib/components/CleanupModal.svelte";
  import {
    formatAboutVersion,
    formatWindowTitle,
    getAppVersionSafe,
  } from "./lib/windowTitle";
  import { inferAgentId } from "./lib/agentUtils";
  import {
    AGENT_TAB_RESTORE_MAX_RETRIES,
    loadStoredProjectAgentTabs,
    persistStoredProjectAgentTabs,
    buildRestoredAgentTabs,
    shouldRetryAgentTabRestore,
  } from "./lib/agentTabsPersistence";
  import { defaultAppTabs, shouldAllowRestoredActiveTab } from "./lib/appTabs";
  import {
    runStartupUpdateCheck,
    STARTUP_UPDATE_INITIAL_DELAY_MS,
    STARTUP_UPDATE_RETRY_DELAY_MS,
    STARTUP_UPDATE_MAX_RETRIES,
  } from "./lib/update/startupUpdate";
  import {
    VoiceInputController,
    type VoiceControllerState,
  } from "./lib/voice/voiceInputController";

  interface MenuActionPayload {
    action: string;
  }

  interface SettingsUpdatedPayload {
    uiFontSize?: number;
    terminalFontSize?: number;
    voiceInput?: VoiceInputSettings;
  }

  const SIDEBAR_WIDTH_STORAGE_KEY = "gwt.sidebar.width";
  const SIDEBAR_MODE_STORAGE_KEY = "gwt.sidebar.mode";
  const DEFAULT_SIDEBAR_WIDTH_PX = 260;
  const MIN_SIDEBAR_WIDTH_PX = 220;
  const MAX_SIDEBAR_WIDTH_PX = 520;
  type SidebarMode = "branch" | "agent";

  const DEFAULT_VOICE_INPUT_SETTINGS: VoiceInputSettings = {
    enabled: false,
    hotkey: "Mod+Shift+M",
    language: "auto",
    model: "base",
  };

  function clampSidebarWidth(widthPx: number): number {
    if (!Number.isFinite(widthPx)) return DEFAULT_SIDEBAR_WIDTH_PX;
    return Math.max(
      MIN_SIDEBAR_WIDTH_PX,
      Math.min(MAX_SIDEBAR_WIDTH_PX, Math.round(widthPx))
    );
  }

  function loadSidebarWidth(): number {
    if (typeof window === "undefined") return DEFAULT_SIDEBAR_WIDTH_PX;
    try {
      const raw = window.localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY);
      if (!raw) return DEFAULT_SIDEBAR_WIDTH_PX;
      return clampSidebarWidth(Number(raw));
    } catch {
      return DEFAULT_SIDEBAR_WIDTH_PX;
    }
  }

  function persistSidebarWidth(widthPx: number) {
    if (typeof window === "undefined") return;
    try {
      window.localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, String(widthPx));
    } catch {
      // Ignore localStorage failures (e.g., disabled in strict environments).
    }
  }

  function loadSidebarMode(): SidebarMode {
    if (typeof window === "undefined") return "branch";
    try {
      const raw = window.localStorage.getItem(SIDEBAR_MODE_STORAGE_KEY);
      return raw === "agent" || raw === "branch" ? raw : "branch";
    } catch {
      return "branch";
    }
  }

  function persistSidebarMode(mode: SidebarMode) {
    if (typeof window === "undefined") return;
    try {
      window.localStorage.setItem(SIDEBAR_MODE_STORAGE_KEY, mode);
    } catch {
      // Ignore localStorage failures (e.g., disabled in strict environments).
    }
  }

  let projectPath: string | null = $state(null);
  let appVersion: string | null = $state(null);
  let sidebarVisible: boolean = $state(true);
  let sidebarWidthPx: number = $state(loadSidebarWidth());
  let sidebarMode: SidebarMode = $state(loadSidebarMode());
  let showAgentLaunch: boolean = $state(false);
  let showCleanupModal: boolean = $state(false);
  let cleanupPreselectedBranch: string | null = $state(null);
  let showAbout: boolean = $state(false);
  let showTerminalDiagnostics: boolean = $state(false);
  let appError: string | null = $state(null);
  let sidebarRefreshKey: number = $state(0);
  let worktreesEventAvailable: boolean = $state(false);

  let selectedBranch: BranchInfo | null = $state(null);
  let currentBranch: string = $state("");

  let launchProgressOpen: boolean = $state(false);
  let launchJobId: string = $state("");
  let pendingLaunchRequest: LaunchAgentRequest | null = $state(null);
  type IssueLaunchFollowup = {
    projectPath: string;
    issueNumber: number;
    branchName: string;
  };
  let issueLaunchFollowups: Map<string, IssueLaunchFollowup> = $state(new Map());

  let migrationOpen: boolean = $state(false);
  let migrationSourceRoot: string = $state("");

  let tabs: Tab[] = $state(defaultAppTabs());
  let activeTabId: string = $state("agentMode");

  let agentTabsHydratedProjectPath: string | null = $state(null);
  let agentTabsRestoreToken = 0;
  const AGENT_TAB_RESTORE_RETRY_DELAY_MS = 150;
  const AGENT_TAB_RESTORE_RETRY_MAX_DELAY_MS = 1200;

  let terminalCount = $derived(tabs.filter((t) => t.type === "agent").length);

  let agentTabBranches = $derived(
    tabs
      .filter((t) => t.type === "agent")
      .map((t) => normalizeBranchName(t.label))
      .filter((b) => b && b !== "Worktree" && b !== "Agent")
  );

  let terminalDiagnosticsLoading: boolean = $state(false);
  let terminalDiagnostics: TerminalAnsiProbe | null = $state(null);
  let terminalDiagnosticsError: string | null = $state(null);

  let osEnvReady = $state(false);
  let voiceInputSettings: VoiceInputSettings = $state(DEFAULT_VOICE_INPUT_SETTINGS);
  let voiceInputListening = $state(false);
  let voiceInputSupported = $state(true);
  let voiceInputError: string | null = $state(null);
  let voiceController: VoiceInputController | null = null;

  let toastMessage = $state<string | null>(null);
  let toastTimeout: ReturnType<typeof setTimeout> | null = null;
  type ToastAction = { kind: "apply-update"; latest: string } | null;
  let toastAction = $state<ToastAction>(null);
  let lastUpdateToastVersion = $state<string | null>(null);

  let showOsEnvDebug = $state(false);
  let osEnvDebugData = $state<CapturedEnvInfo | null>(null);
  let osEnvDebugLoading = $state(false);
  let osEnvDebugError = $state<string | null>(null);
  type AvailableUpdateState = Extract<UpdateState, { state: "available" }>;

  function showToast(message: string, durationMs = 8000, action: ToastAction = null) {
    toastMessage = message;
    toastAction = action;
    if (toastTimeout) clearTimeout(toastTimeout);
    toastTimeout = null;
    if (durationMs > 0) {
      toastTimeout = setTimeout(() => {
        toastMessage = null;
        toastAction = null;
      }, durationMs);
    }
  }

  function showAvailableUpdateToast(s: AvailableUpdateState) {
    if (lastUpdateToastVersion === s.latest) return;
    lastUpdateToastVersion = s.latest;

    if (s.asset_url) {
      showToast(
        `Update available: v${s.latest} (click update)`,
        0,
        { kind: "apply-update", latest: s.latest },
      );
    } else {
      showToast(`Update available: v${s.latest}. Manual download required.`, 15000);
    }
  }
  async function confirmAndApplyUpdate(latest: string) {
    try {
      const { confirm } = await import("@tauri-apps/plugin-dialog");
      const ok = await confirm(
        `Update available: v${latest}\nRestart to update now?`,
        { title: "gwt", kind: "info" },
      );
      if (!ok) return;

      showToast(`Updating to v${latest}...`, 0);

      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("apply_app_update");
    } catch (err) {
      showToast(`Failed to apply update: ${toErrorMessage(err)}`);
    }
  }

  async function handleToastClick() {
    if (!toastAction) return;
    if (toastAction.kind === "apply-update") {
      await confirmAndApplyUpdate(toastAction.latest);
    }
  }

  function queueIssueLaunchFollowup(jobId: string, request: LaunchAgentRequest) {
    const issueNumber = request.issueNumber;
    const branchName = request.createBranch?.name?.trim();
    if (!projectPath || !issueNumber || !branchName) return;
    issueLaunchFollowups = new Map(issueLaunchFollowups).set(jobId, {
      projectPath,
      issueNumber,
      branchName,
    });
  }

  async function handleIssueLaunchFinished(payload: LaunchFinishedPayload) {
    const followup = issueLaunchFollowups.get(payload.jobId);
    if (!followup) return;

    const next = new Map(issueLaunchFollowups);
    next.delete(payload.jobId);
    issueLaunchFollowups = next;

    try {
      const { invoke } = await import("@tauri-apps/api/core");

      if (payload.status === "ok") {
        await invoke("link_branch_to_issue", {
          projectPath: followup.projectPath,
          issueNumber: followup.issueNumber,
          branchName: followup.branchName,
        });
        return;
      }

      const rollback = await invoke<RollbackResult>("rollback_issue_branch", {
        projectPath: followup.projectPath,
        branchName: followup.branchName,
        // Only rollback local artifacts on launch failure.
        // Remote deletion is unsafe here because the branch may have pre-existed remotely.
        deleteRemote: false,
      });

      sidebarRefreshKey++;

      const warnings: string[] = [];
      if (!rollback.localDeleted) {
        warnings.push("Local rollback was incomplete.");
      }
      if (rollback.error) {
        warnings.push(rollback.error.trim());
      }

      if (warnings.length > 0) {
        const verb = payload.status === "cancelled" ? "cancelled" : "failed";
        showToast(
          `Issue launch ${verb}. ${warnings.join(" ")}`,
          12000,
        );
      }
    } catch (err) {
      showToast(`Issue launch cleanup failed: ${toErrorMessage(err)}`, 12000);
    }
  }

  // Poll OS env readiness at startup; stop once ready.
  $effect(() => {
    if (osEnvReady) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        while (!cancelled && !osEnvReady) {
          const ready = await invoke<boolean>("is_os_env_ready");
          if (ready) {
            osEnvReady = true;
            return;
          }
          await new Promise(r => setTimeout(r, 200));
        }
      } catch { /* ignore */ }
    };
    poll();
    return () => { cancelled = true; };
  });

  // Listen for OS env fallback event and show toast.
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;
    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenFn = await listen<string>("os-env-fallback", (event) => {
          showToast(`Shell environment not loaded: ${event.payload}. Using process environment.`);
        });
        if (cancelled) { unlistenFn(); return; }
        unlisten = unlistenFn;
      } catch { /* ignore */ }
    })();
    return () => { cancelled = true; if (unlisten) unlisten(); };
  });

  // Best-effort: read app version from Tauri runtime (web preview will ignore).
  $effect(() => {
    let cancelled = false;
    (async () => {
      const v = await getAppVersionSafe();
      if (cancelled) return;
      appVersion = v;
    })();
    return () => { cancelled = true; };
  });

  // Best-effort: request update state once on startup.
  $effect(() => {
    if (lastUpdateToastVersion !== null) return;
    const controller = new AbortController();
    void runStartupUpdateCheck({
      signal: controller.signal,
      initialDelayMs: STARTUP_UPDATE_INITIAL_DELAY_MS,
      retryDelayMs: STARTUP_UPDATE_RETRY_DELAY_MS,
      maxRetries: STARTUP_UPDATE_MAX_RETRIES,
      checkUpdate: async () => {
        const { invoke } = await import("@tauri-apps/api/core");
        return invoke<UpdateState>("check_app_update", { force: false });
      },
      onAvailable: (s) => {
        showAvailableUpdateToast(s);
      },
    });
    return () => {
      controller.abort();
    };
  });

  // Listen for app update state notifications from backend startup checks.
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;
    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenFn = await listen<UpdateState>("app-update-state", (event) => {
          const s = event.payload;
          if (s.state !== "available") return;
          showAvailableUpdateToast(s);
        });
        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch {
        // Ignore when event API is unavailable.
      }
    })();
    return () => { cancelled = true; if (unlisten) unlisten(); };
  });

  $effect(() => {
    void projectPath;
    void setWindowTitle();
    void applyAppearanceSettings();
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

  // Handle issue-linking/rollback on actual launch completion.
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenFn = await listen<LaunchFinishedPayload>(
          "launch-finished",
          (event) => {
            void handleIssueLaunchFinished(event.payload);
          }
        );

        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch (err) {
        console.error("Failed to setup launch finished listener:", err);
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

  function clampFontSize(size: number): number {
    return Math.max(8, Math.min(24, Math.round(size)));
  }

  function normalizeVoiceInputSettings(
    value: Partial<VoiceInputSettings> | null | undefined
  ): VoiceInputSettings {
    const hotkey = (value?.hotkey ?? "").trim();
    const language = (value?.language ?? "").trim().toLowerCase();
    const model = (value?.model ?? "").trim();

    return {
      enabled: !!value?.enabled,
      hotkey: hotkey.length > 0 ? hotkey : DEFAULT_VOICE_INPUT_SETTINGS.hotkey,
      language:
        language === "ja" || language === "en" || language === "auto"
          ? (language as VoiceInputSettings["language"])
          : DEFAULT_VOICE_INPUT_SETTINGS.language,
      model: model.length > 0 ? model : DEFAULT_VOICE_INPUT_SETTINGS.model,
    };
  }

  function applyUiFontSize(size: number) {
    document.documentElement.style.setProperty("--ui-font-base", `${size}px`);
  }

  function applyTerminalFontSize(size: number) {
    (window as any).__gwtTerminalFontSize = size;
    window.dispatchEvent(new CustomEvent("gwt-terminal-font-size", { detail: size }));
  }

  function applyVoiceInputSettings(value: Partial<VoiceInputSettings> | null | undefined) {
    voiceInputSettings = normalizeVoiceInputSettings(value);
    voiceController?.updateSettings();
  }

  function activeAgentPaneId(): string | null {
    const active = tabs.find((t) => t.id === activeTabId);
    if (
      active?.type === "agent" &&
      typeof active.paneId === "string" &&
      active.paneId.length > 0
    ) {
      return active.paneId;
    }
    return null;
  }

  function readVoiceInputSettingsForController(): VoiceInputSettings {
    return voiceInputSettings;
  }

  function readVoiceFallbackTerminalPaneId(): string | null {
    return activeAgentPaneId();
  }

  async function applyAppearanceSettings() {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const settings = await invoke<
        Pick<SettingsData, "ui_font_size" | "terminal_font_size" | "voice_input">
      >(
        "get_settings"
      );
      applyUiFontSize(clampFontSize(settings.ui_font_size ?? 13));
      applyTerminalFontSize(clampFontSize(settings.terminal_font_size ?? 13));
      applyVoiceInputSettings(settings.voice_input);
    } catch {
      // Ignore: settings API not available outside Tauri runtime.
    }
  }

  async function setWindowTitle() {
    const title = formatWindowTitle({
      appName: "gwt",
      projectPath,
    });

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

  function handleBranchSelect(branch: BranchInfo) {
    selectedBranch = branch;
    if (branch.is_current) {
      currentBranch = branch.name;
    }
  }

  function requestAgentLaunch() {
    showAgentLaunch = true;
  }

  function handleSidebarResize(nextWidthPx: number) {
    const next = clampSidebarWidth(nextWidthPx);
    if (next === sidebarWidthPx) return;
    sidebarWidthPx = next;
    persistSidebarWidth(next);
  }

  function ensureAgentModeTab() {
    const existing = tabs.find((t) => t.type === "agentMode" || t.id === "agentMode");
    if (existing) return;

    const tab: Tab = { id: "agentMode", label: "Agent Mode", type: "agentMode" };
    tabs = [...tabs, tab];
  }

  function handleSidebarModeChange(next: SidebarMode) {
    if (sidebarMode === next) return;
    if (next === "agent") {
      ensureAgentModeTab();
    }
    sidebarMode = next;
    persistSidebarMode(next);
  }

  function handleBranchActivate(branch: BranchInfo) {
    handleBranchSelect(branch);
    requestAgentLaunch();
  }

  function handleCleanupRequest(preSelectedBranch?: string) {
    cleanupPreselectedBranch = preSelectedBranch ?? null;
    showCleanupModal = true;
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

  function mergeRestoredAgentTabs(existingTabs: Tab[], restoredTabs: Tab[]): Tab[] {
    const nonAgentTabs = existingTabs.filter((t) => t.type !== "agent");
    const restoredAgentPaneIds = new Set(restoredTabs.map((t) => t.paneId));
    const preservedAgentTabs = existingTabs.filter((t) => {
      return t.type === "agent" && typeof t.paneId === "string" && !restoredAgentPaneIds.has(t.paneId);
    });

    const dedupedPreserved: Tab[] = [];
    const seen = new Set<string>();
    for (const tab of preservedAgentTabs) {
      if (!tab.paneId || seen.has(tab.paneId)) continue;
      seen.add(tab.paneId);
      dedupedPreserved.push(tab);
    }

    return [...nonAgentTabs, ...restoredTabs, ...dedupedPreserved];
  }

  function getAgentTabRestoreDelayMs(attempt: number): number {
    return Math.min(
      AGENT_TAB_RESTORE_RETRY_MAX_DELAY_MS,
      AGENT_TAB_RESTORE_RETRY_DELAY_MS * 2 ** Math.min(attempt, 8),
    );
  }

  function triggerRestoreProjectAgentTabs(targetProjectPath: string) {
    const token = ++agentTabsRestoreToken;
    void restoreProjectAgentTabs(targetProjectPath, token);
  }

  async function handleAgentLaunch(request: LaunchAgentRequest) {
    const { invoke } = await import("@tauri-apps/api/core");
    const jobId = await invoke<string>("start_launch_job", { request });

    queueIssueLaunchFollowup(jobId, request);
    pendingLaunchRequest = request;
    launchJobId = jobId;
    launchProgressOpen = true;
  }

  function handleLaunchSuccess(paneId: string) {
    const req = pendingLaunchRequest;
    const label = req ? worktreeTabLabel(req.branch) : "Worktree";
    const requestedAgentId = inferAgentId(req?.agentId);

    const newTab: Tab = {
      id: `agent-${paneId}`,
      label,
      type: "agent",
      paneId,
    };

    if (requestedAgentId) {
      newTab.agentId = requestedAgentId;
    }

    tabs = [...tabs, newTab];
    activeTabId = newTab.id;
    if (projectPath) {
      agentTabsHydratedProjectPath = null;
      triggerRestoreProjectAgentTabs(projectPath);
    }

    if (!newTab.agentId) {
      void (async () => {
        try {
          const { invoke } = await import("@tauri-apps/api/core");
          const terminals = await invoke<TerminalInfo[]>("list_terminals");
          const terminal = terminals.find((t) => t.pane_id === paneId);
          const terminalAgentId = inferAgentId(terminal?.agent_name);
          if (!terminalAgentId) return;

          tabs = tabs.map((t) =>
            t.id === newTab.id ? { ...t, agentId: terminalAgentId } : t
          );
        } catch {
          // Ignore: fallback color is used when terminal metadata is unavailable.
        }
      })();
    }

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

  function openVersionHistoryTab() {
    const existing = tabs.find(
      (t) => t.type === "versionHistory" || t.id === "versionHistory",
    );
    if (existing) {
      activeTabId = existing.id;
      return;
    }

    const tab: Tab = {
      id: "versionHistory",
      label: "Version History",
      type: "versionHistory",
    };
    tabs = [...tabs, tab];
    activeTabId = tab.id;
  }

  function getActiveTerminalPaneId(): string | null {
    const active = tabs.find((t) => t.id === activeTabId);
    if (!active || active.type !== "agent") {
      return null;
    }
    return active.paneId && active.paneId.length > 0 ? active.paneId : null;
  }

  function getActiveEditableElement():
    | HTMLInputElement
    | HTMLTextAreaElement
    | HTMLElement
    | null {
    if (typeof document === "undefined") return null;
    const el = document.activeElement;
    if (!el) return null;

    if (el instanceof HTMLInputElement && !el.readOnly && !el.disabled) {
      return el;
    }
    if (el instanceof HTMLTextAreaElement && !el.readOnly && !el.disabled) {
      return el;
    }
    if (el instanceof HTMLElement && el.isContentEditable) {
      return el;
    }

    return null;
  }

  async function fallbackMenuEditAction(action: "copy" | "paste") {
    const target = getActiveEditableElement();
    if (!target) {
      // Let browser/editor defaults decide when there is no editable target.
      if (action === "copy") {
        document.execCommand("copy");
      }
      return;
    }

    if (action === "copy") {
      document.execCommand("copy");
      return;
    }

    if (!navigator.clipboard?.readText) return;
    let text: string;
    try {
      text = await navigator.clipboard.readText();
    } catch {
      return;
    }
    if (!text) return;

    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
      const start = target.selectionStart ?? target.value.length;
      const end = target.selectionEnd ?? target.value.length;
      target.setRangeText(text, start, end, "end");
      return;
    }

    if (target.isContentEditable) {
      target.focus();
      document.execCommand("insertText", false, text);
    }
  }

  function emitTerminalEditAction(action: "copy" | "paste") {
    const paneId = getActiveTerminalPaneId();
    if (!paneId) {
      void fallbackMenuEditAction(action);
      return;
    }

    if (typeof window === "undefined") return;

    window.dispatchEvent(
      new CustomEvent("gwt-terminal-edit-action", {
        detail: { action, paneId },
      })
    );
  }

  async function syncWindowAgentTabs() {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const agentTabs = tabs
        .filter((t) => t.type === "agent")
        .map((t) => ({ id: t.id, label: t.label }));
      const activeAgentTabId = agentTabs.some((t) => t.id === activeTabId)
        ? activeTabId
        : null;
      await invoke("sync_window_agent_tabs", {
        request: {
          tabs: agentTabs,
          activeTabId: activeAgentTabId,
        },
      });
    } catch {
      // Ignore: not available outside Tauri runtime.
    }
  }

  async function handleMenuAction(action: string) {
    if (action.startsWith("focus-agent-tab::")) {
      const tabId = action.slice("focus-agent-tab::".length).trim();
      if (tabId && tabs.some((t) => t.id === tabId && t.type === "agent")) {
        activeTabId = tabId;
      }
      return;
    }

    // Handle dynamic "open-recent-project::<path>" actions before the switch.
    if (action.startsWith("open-recent-project::")) {
      const recentPath = action.slice("open-recent-project::".length);
      if (recentPath) {
        try {
          const { invoke } = await import("@tauri-apps/api/core");
          const probe = await invoke<ProbePathResult>("probe_path", {
            path: recentPath,
          });

          if (probe.kind === "gwtProject" && probe.projectPath) {
            const info = await invoke<ProjectInfo>("open_project", {
              path: probe.projectPath,
            });
            projectPath = info.path;
            fetchCurrentBranch();
            return;
          }

          if (probe.kind === "migrationRequired" && probe.migrationSourceRoot) {
            migrationSourceRoot = probe.migrationSourceRoot;
            migrationOpen = true;
            return;
          }

          appError =
            probe.message || "Failed to open recent project.";
        } catch (err) {
          appError = `Failed to open project: ${toErrorMessage(err)}`;
        }
      }
      return;
    }

    switch (action) {
      case "open-project": {
        try {
          const { open } = await import("@tauri-apps/plugin-dialog");
          const selected = await open({ directory: true, multiple: false });
          if (selected) {
            const { invoke } = await import("@tauri-apps/api/core");
            const probe = await invoke<ProbePathResult>("probe_path", {
              path: selected as string,
            });

            if (probe.kind === "gwtProject" && probe.projectPath) {
              const info = await invoke<ProjectInfo>("open_project", {
                path: probe.projectPath,
              });
              projectPath = info.path;
              fetchCurrentBranch();
              break;
            }

            if (probe.kind === "migrationRequired" && probe.migrationSourceRoot) {
              migrationSourceRoot = probe.migrationSourceRoot;
              migrationOpen = true;
              break;
            }

            if (probe.kind === "emptyDir") {
              appError =
                "Selected folder is empty. Use New Project on the start screen.";
              break;
            }

            appError =
              probe.message ||
              (probe.kind === "notFound"
                ? "Path does not exist."
                : probe.kind === "invalid"
                  ? "Invalid path."
                  : "Not a gwt project.");
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
          tabs = defaultAppTabs();
          activeTabId = "agentMode";
          selectedBranch = null;
          currentBranch = "";
        }
        break;
      case "toggle-sidebar":
        sidebarVisible = !sidebarVisible;
        break;
      case "launch-agent":
        if (projectPath) {
          showAgentLaunch = true;
        }
        break;
      case "cleanup-worktrees":
        if (projectPath) {
          cleanupPreselectedBranch = null;
          showCleanupModal = true;
        }
        break;
      case "open-settings":
        openSettingsTab();
        break;
      case "version-history":
        openVersionHistoryTab();
        break;
      case "check-updates":
        {
          try {
            const { invoke } = await import("@tauri-apps/api/core");
            const s = await invoke<UpdateState>("check_app_update", { force: true });
            switch (s.state) {
              case "up_to_date":
                showToast("Up to date.");
                break;
              case "available":
                showAvailableUpdateToast(s);
                break;
              case "failed":
                showToast(`Update check failed: ${s.message}`);
                break;
            }
          } catch (err) {
            showToast(`Update check failed: ${toErrorMessage(err)}`);
          }
        }
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
      case "edit-copy":
        emitTerminalEditAction("copy");
        break;
      case "edit-paste":
        emitTerminalEditAction("paste");
        break;
      case "debug-os-env":
        showOsEnvDebug = true;
        osEnvDebugLoading = true;
        osEnvDebugError = null;
        (async () => {
          try {
            const { invoke } = await import("@tauri-apps/api/core");
            osEnvDebugData = await invoke<CapturedEnvInfo>("get_captured_environment");
          } catch (e) {
            osEnvDebugError = String(e);
          } finally {
            osEnvDebugLoading = false;
          }
        })();
        break;
      case "terminal-diagnostics": {
        const active = tabs.find((t) => t.id === activeTabId) ?? null;
        const paneId = active?.paneId ?? "";
        if (!paneId) {
          appError = "No active terminal tab.";
          break;
        }

        showTerminalDiagnostics = true;
        terminalDiagnosticsLoading = true;
        terminalDiagnosticsError = null;
        terminalDiagnostics = null;

        try {
          const { invoke } = await import("@tauri-apps/api/core");
          terminalDiagnostics = await invoke<TerminalAnsiProbe>("probe_terminal_ansi", {
            paneId,
          });
        } catch (err) {
          terminalDiagnosticsError = `Failed to probe terminal: ${toErrorMessage(err)}`;
        } finally {
          terminalDiagnosticsLoading = false;
        }
        break;
      }
    }
  }

  $effect(() => {
    void tabs;
    void activeTabId;
    void syncWindowAgentTabs();
  });

  async function restoreProjectAgentTabs(
    targetProjectPath: string,
    token: number,
    attempt = 0,
  ) {
    const stored = loadStoredProjectAgentTabs(targetProjectPath);

    // Even if no stored state exists, mark hydrated so persistence can proceed.
    if (!stored) {
      if (projectPath === targetProjectPath && agentTabsRestoreToken === token) {
        agentTabsHydratedProjectPath = targetProjectPath;
      }
      return;
    }

    let terminals: TerminalInfo[] = [];
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      terminals = await invoke<TerminalInfo[]>("list_terminals");
    } catch {
      // Ignore: not available outside Tauri runtime.
    }

    if (projectPath !== targetProjectPath || agentTabsRestoreToken !== token) {
      return;
    }

    const restored = buildRestoredAgentTabs(stored, terminals);
    const shouldRetry = shouldRetryAgentTabRestore(
      stored.tabs.length,
      restored.tabs.length,
      attempt,
      AGENT_TAB_RESTORE_MAX_RETRIES,
    );

    if (shouldRetry && projectPath === targetProjectPath && agentTabsRestoreToken === token) {
      setTimeout(() => {
        void restoreProjectAgentTabs(targetProjectPath, token, attempt + 1);
      }, getAgentTabRestoreDelayMs(attempt));
      return;
    }

    // Wait for terminal list to become available before persisting and wiping state.
    if (stored.tabs.length > 0 && restored.tabs.length === 0) {
      return;
    }

    const restoredTabs = restored.tabs;
    const mergedTabs = mergeRestoredAgentTabs(tabs, restoredTabs);
    tabs = mergedTabs;

    const allowOverrideActive = shouldAllowRestoredActiveTab(activeTabId);
    if (allowOverrideActive && restored.activeTabId) {
      activeTabId = restored.activeTabId;
    }

    agentTabsHydratedProjectPath = targetProjectPath;
  }

  // Restore persisted agent tabs when a project is opened.
  $effect(() => {
    void projectPath;

    if (!projectPath) {
      agentTabsHydratedProjectPath = null;
      return;
    }

    agentTabsHydratedProjectPath = null;
    const target = projectPath;
    triggerRestoreProjectAgentTabs(target);
  });

  // Persist agent tabs per project (best-effort).
  $effect(() => {
    void projectPath;
    void tabs;
    void activeTabId;
    void agentTabsHydratedProjectPath;

    if (!projectPath) return;
    if (agentTabsHydratedProjectPath !== projectPath) return;

    const agentTabs: Array<{ paneId: string; label: string }> = tabs
      .filter((t) => t.type === "agent" && typeof t.paneId === "string" && t.paneId.length > 0)
      .map((t) => ({ paneId: t.paneId as string, label: t.label }));

    const active = tabs.find((t) => t.id === activeTabId);
    const activePaneId =
      active?.type === "agent" && typeof active.paneId === "string" && active.paneId.length > 0
        ? active.paneId
        : null;

    persistStoredProjectAgentTabs(projectPath, {
      tabs: agentTabs,
      activePaneId,
    });
  });

  // Claude Code Hooks: check & register on startup
  $effect(() => {
    (async () => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const status = await invoke<{
          registered: boolean;
          updated: boolean;
          temporary_execution: boolean;
        }>("check_and_update_hooks");

        if (status.temporary_execution) {
          console.warn("gwt is running from a temporary execution environment; hooks may not persist.");
        }

        if (!status.registered) {
          const { confirm } = await import("@tauri-apps/plugin-dialog");
          const message = status.temporary_execution
            ? [
                "gwt is running from a temporary execution environment (e.g. bunx/npx cache).",
                "If you register hooks now, the stored executable path may not persist and hooks can break later.",
                "",
                "Register Claude Code hooks for gwt anyway? This allows gwt to track agent status.",
              ].join("\n")
            : "Register Claude Code hooks for gwt? This allows gwt to track agent status.";
          const ok = await confirm(
            message,
            { title: "gwt", kind: status.temporary_execution ? "warning" : "info" },
          );
          if (ok) {
            await invoke("register_hooks");
          }
        }
      } catch (err) {
        console.error("Failed to check/register Claude Code hooks:", err);
      }
    })();
  });

  // Native menubar integration (Tauri emits "menu-action" to the focused window).
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenFn = await listen<MenuActionPayload>("menu-action", (event) => {
          void handleMenuAction(event.payload.action);
        });
        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch {
        // Ignore: not available outside Tauri runtime.
      }
    })();

    return () => {
      cancelled = true;
      if (unlisten) {
        unlisten();
      }
    };
  });

  $effect(() => {
    const controller = new VoiceInputController({
      getSettings: readVoiceInputSettingsForController,
      getFallbackTerminalPaneId: readVoiceFallbackTerminalPaneId,
      onStateChange: (state: VoiceControllerState) => {
        voiceInputListening = state.listening;
        voiceInputSupported = state.supported;
        voiceInputError = state.error;
      },
    });
    voiceController = controller;
    controller.updateSettings();

    return () => {
      controller.dispose();
      if (voiceController === controller) {
        voiceController = null;
      }
      voiceInputListening = false;
      voiceInputError = null;
      voiceInputSupported = true;
    };
  });

  $effect(() => {
    function onSettingsUpdated(event: Event) {
      const detail = (event as CustomEvent<SettingsUpdatedPayload>).detail;
      if (!detail) return;
      if (typeof detail.uiFontSize === "number") {
        applyUiFontSize(clampFontSize(detail.uiFontSize));
      }
      if (typeof detail.terminalFontSize === "number") {
        applyTerminalFontSize(clampFontSize(detail.terminalFontSize));
      }
      if (detail.voiceInput) {
        applyVoiceInputSettings(detail.voiceInput);
      }
    }

    window.addEventListener("gwt-settings-updated", onSettingsUpdated);
    return () => window.removeEventListener("gwt-settings-updated", onSettingsUpdated);
  });

  // Global keyboard shortcut: Cmd+Shift+K / Ctrl+Shift+K to open Cleanup modal.
  // The native menu accelerator handles this on macOS, but this provides a
  // fallback for web-preview and non-Tauri contexts.
  $effect(() => {
    function onKeydown(e: KeyboardEvent) {
      if (
        e.key === "K" &&
        e.shiftKey &&
        (e.metaKey || e.ctrlKey) &&
        !e.altKey
      ) {
        e.preventDefault();
        void handleMenuAction("cleanup-worktrees");
      }
    }
    document.addEventListener("keydown", onKeydown);
    return () => document.removeEventListener("keydown", onKeydown);
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
          widthPx={sidebarWidthPx}
          minWidthPx={MIN_SIDEBAR_WIDTH_PX}
          maxWidthPx={MAX_SIDEBAR_WIDTH_PX}
          mode={sidebarMode}
          onModeChange={handleSidebarModeChange}
          {selectedBranch}
          {currentBranch}
          {agentTabBranches}
          onResize={handleSidebarResize}
          onBranchSelect={handleBranchSelect}
          onBranchActivate={handleBranchActivate}
          onCleanupRequest={handleCleanupRequest}
          onLaunchAgent={requestAgentLaunch}
          onQuickLaunch={handleAgentLaunch}
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
    <StatusBar
      {projectPath}
      {currentBranch}
      {terminalCount}
      {osEnvReady}
      voiceInputEnabled={voiceInputSettings.enabled}
      voiceInputListening={voiceInputListening}
      voiceInputSupported={voiceInputSupported}
      voiceInputError={voiceInputError}
    />
  </div>
{/if}

{#if showAgentLaunch}
  <AgentLaunchForm
    projectPath={projectPath as string}
    selectedBranch={selectedBranch?.name ?? currentBranch}
    osEnvReady={osEnvReady}
    onLaunch={handleAgentLaunch}
    onClose={() => (showAgentLaunch = false)}
  />
{/if}

<CleanupModal
  open={showCleanupModal}
  preselectedBranch={cleanupPreselectedBranch}
  refreshKey={sidebarRefreshKey}
  projectPath={projectPath ?? ""}
  {agentTabBranches}
  onClose={() => (showCleanupModal = false)}
/>

{#if showAbout}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay" onclick={() => (showAbout = false)}>
    <div class="about-dialog" onclick={(e) => e.stopPropagation()}>
      <h2>gwt</h2>
      <p>Git Worktree Manager</p>
      <p class="about-version">GUI Edition</p>
      <p class="about-version">{formatAboutVersion(appVersion)}</p>
      <button class="about-close" onclick={() => (showAbout = false)}>
        Close
      </button>
    </div>
  </div>
{/if}

{#if showTerminalDiagnostics}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay" onclick={() => (showTerminalDiagnostics = false)}>
    <div class="diag-dialog" onclick={(e) => e.stopPropagation()}>
      <h2>Terminal Diagnostics</h2>

      {#if terminalDiagnosticsLoading}
        <p class="diag-muted">Probing output...</p>
      {:else if terminalDiagnosticsError}
        <p class="diag-error">{terminalDiagnosticsError}</p>
      {:else if terminalDiagnostics}
        <div class="diag-grid">
          <div class="diag-item">
            <span class="diag-label">Pane</span>
            <span class="diag-value mono">{terminalDiagnostics.pane_id}</span>
          </div>
          <div class="diag-item">
            <span class="diag-label">Bytes</span>
            <span class="diag-value mono">{terminalDiagnostics.bytes_scanned}</span>
          </div>
          <div class="diag-item">
            <span class="diag-label">ESC</span>
            <span class="diag-value mono">{terminalDiagnostics.esc_count}</span>
          </div>
          <div class="diag-item">
            <span class="diag-label">SGR</span>
            <span class="diag-value mono">{terminalDiagnostics.sgr_count}</span>
          </div>
          <div class="diag-item">
            <span class="diag-label">Color SGR</span>
            <span class="diag-value mono">{terminalDiagnostics.color_sgr_count}</span>
          </div>
          <div class="diag-item">
            <span class="diag-label">256-color</span>
            <span class="diag-value mono">
              {terminalDiagnostics.has_256_color ? "yes" : "no"}
            </span>
          </div>
          <div class="diag-item">
            <span class="diag-label">TrueColor</span>
            <span class="diag-value mono">
              {terminalDiagnostics.has_true_color ? "yes" : "no"}
            </span>
          </div>
        </div>

        {#if terminalDiagnostics.color_sgr_count === 0}
          <div class="diag-hint">
            <p>
              No color SGR codes were detected in the tail of the scrollback. This
              usually means the program did not emit ANSI colors (for example, output
              was captured or treated as non-interactive).
            </p>
            <p class="diag-muted">Try forcing color output:</p>
            <pre class="diag-code mono">git -c color.ui=always diff</pre>
            <pre class="diag-code mono">rg --color=always PATTERN</pre>
          </div>
        {:else}
          <div class="diag-hint">
            <p>
              Color SGR codes were detected. If you still do not see colors, the issue
              is likely in the terminal rendering path.
            </p>
          </div>
        {/if}
      {:else}
        <p class="diag-muted">No data.</p>
      {/if}

      <button
        class="about-close"
        onclick={() => (showTerminalDiagnostics = false)}
      >
        Close
      </button>
    </div>
  </div>
{/if}

<MigrationModal
  open={migrationOpen}
  sourceRoot={migrationSourceRoot}
  onCompleted={async (p) => {
    migrationOpen = false;
    migrationSourceRoot = "";

    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const info = await invoke<ProjectInfo>("open_project", { path: p });
      projectPath = info.path;
      fetchCurrentBranch();
    } catch (err) {
      appError = `Failed to open migrated project: ${toErrorMessage(err)}`;
    }
  }}
  onDismiss={() => {
    migrationOpen = false;
    migrationSourceRoot = "";
  }}
/>

<LaunchProgressModal
  open={launchProgressOpen}
  jobId={launchJobId}
  onSuccess={handleLaunchSuccess}
  onClose={() => {
    launchProgressOpen = false;
    launchJobId = "";
    pendingLaunchRequest = null;
  }}
/>
{#if showOsEnvDebug}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay" onclick={() => (showOsEnvDebug = false)}>
    <div class="env-debug-dialog" onclick={(e) => e.stopPropagation()}>
      <h3>Captured Environment</h3>
      {#if osEnvDebugLoading}
        <p class="env-debug-loading">Loading...</p>
      {:else if osEnvDebugError}
        <p class="env-debug-error">{osEnvDebugError}</p>
      {:else if osEnvDebugData}
        <div class="env-debug-meta">
          <span>Source: <strong>{osEnvDebugData.source === 'login_shell' ? 'Login Shell' : osEnvDebugData.source === 'std_env_fallback' ? 'Process Env (fallback)' : osEnvDebugData.source}</strong></span>
          {#if osEnvDebugData.reason}
            <span class="env-debug-reason">Reason: {osEnvDebugData.reason}</span>
          {/if}
          <span>Variables: {osEnvDebugData.entries.length}</span>
        </div>
        <div class="env-debug-list">
          {#each osEnvDebugData.entries as entry}
            <div class="env-debug-row">
              <span class="env-debug-key">{entry.key}</span>
              <span class="env-debug-val">{entry.value}</span>
            </div>
          {/each}
        </div>
      {/if}
      <button class="about-close" onclick={() => (showOsEnvDebug = false)}>Close</button>
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

{#if toastMessage}
  <div class="toast-container">
    <div class="toast-message">
      <span>{toastMessage}</span>
      {#if toastAction?.kind === "apply-update"}
        <button class="toast-action" onclick={handleToastClick}>Update</button>
      {/if}
      <button
        class="toast-close"
        onclick={() => { toastMessage = null; toastAction = null; }}
      >
        [x]
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

  .mono {
    font-family: monospace;
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
    font-size: var(--ui-font-base);
  }

  .about-version {
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
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
    font-size: var(--ui-font-md);
  }

  .about-close:hover {
    background: var(--bg-hover);
  }

  .diag-dialog {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 12px;
    padding: 24px 28px;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4);
    max-width: 720px;
    width: min(720px, 92vw);
  }

  .diag-dialog h2 {
    font-size: var(--ui-font-xl);
    font-weight: 800;
    color: var(--text-primary);
    margin-bottom: 12px;
  }

  .diag-muted {
    color: var(--text-muted);
    font-size: var(--ui-font-md);
  }

  .diag-error {
    color: rgb(255, 160, 160);
    font-size: var(--ui-font-md);
    white-space: pre-wrap;
  }

  .diag-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px 14px;
    margin: 14px 0 18px;
  }

  .diag-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 10px;
    border: 1px solid var(--border-color);
    border-radius: 8px;
    background: var(--bg-primary);
  }

  .diag-label {
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
  }

  .diag-value {
    color: var(--text-primary);
    font-size: var(--ui-font-md);
    text-align: right;
  }

  .diag-hint {
    border: 1px solid var(--border-color);
    border-radius: 10px;
    background: var(--bg-surface);
    padding: 12px 14px;
    color: var(--text-secondary);
    font-size: var(--ui-font-md);
    line-height: 1.55;
    margin-bottom: 16px;
  }

  .diag-hint p {
    margin: 0 0 8px;
  }

  .diag-code {
    margin: 8px 0;
    padding: 10px 12px;
    border: 1px solid var(--border-color);
    border-radius: 8px;
    background: var(--bg-primary);
    overflow-x: auto;
    white-space: pre;
    font-size: var(--ui-font-md);
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
    font-size: var(--ui-font-2xl);
    font-weight: 800;
    color: rgb(255, 160, 160);
    margin-bottom: 10px;
  }

  .error-text {
    color: var(--text-secondary);
    font-size: var(--ui-font-md);
    line-height: 1.5;
    margin-bottom: 18px;
    white-space: pre-wrap;
  }

  .toast-container {
    position: fixed;
    bottom: 40px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 2000;
    pointer-events: none;
  }

  .toast-message {
    pointer-events: auto;
    background: var(--bg-tertiary, #45475a);
    color: var(--text-warning, #f9e2af);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    padding: 10px 16px;
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: 12px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  }

  .toast-action {
    border: 1px solid var(--border-color);
    background: var(--bg-secondary);
    color: var(--text-primary);
    border-radius: 6px;
    padding: 4px 10px;
    font-size: 12px;
    cursor: pointer;
  }

  .toast-action:hover {
    background: var(--bg-hover, rgba(255, 255, 255, 0.08));
  }

  .toast-close {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 13px;
    padding: 0;
  }

  .env-debug-dialog {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 12px;
    padding: 24px 28px;
    min-width: 600px;
    max-width: 800px;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4);
  }

  .env-debug-dialog h3 {
    margin: 0 0 16px;
    font-size: 16px;
    color: var(--text-primary);
  }

  .env-debug-meta {
    display: flex;
    gap: 16px;
    font-size: 13px;
    color: var(--text-secondary);
    margin-bottom: 12px;
    flex-wrap: wrap;
  }

  .env-debug-reason {
    color: var(--text-warning, #f9e2af);
  }

  .env-debug-list {
    overflow-y: auto;
    flex: 1;
    border: 1px solid var(--border-color);
    border-radius: 6px;
    margin-bottom: 16px;
  }

  .env-debug-row {
    display: flex;
    border-bottom: 1px solid var(--border-color);
    font-size: 12px;
    font-family: var(--font-mono, monospace);
  }

  .env-debug-row:last-child {
    border-bottom: none;
  }

  .env-debug-key {
    min-width: 200px;
    max-width: 200px;
    padding: 4px 8px;
    color: var(--text-accent, #89b4fa);
    word-break: break-all;
    border-right: 1px solid var(--border-color);
  }

  .env-debug-val {
    flex: 1;
    padding: 4px 8px;
    color: var(--text-primary);
    word-break: break-all;
    overflow-wrap: anywhere;
  }

  .env-debug-loading, .env-debug-error {
    font-size: 13px;
    padding: 12px 0;
  }

  .env-debug-error {
    color: var(--text-error, #f38ba8);
  }
</style>
