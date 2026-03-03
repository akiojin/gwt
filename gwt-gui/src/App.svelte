<script lang="ts">
  import type {
    Tab,
    BranchInfo,
    GitHubIssueInfo,
    OpenProjectResult,
    LaunchAgentRequest,
    LaunchFinishedPayload,
    LaunchProgressPayload,
    ProbePathResult,
    TerminalInfo,
    WorktreeInfo,
    TerminalAnsiProbe,
    CapturedEnvInfo,
    SettingsData,
    SkillRegistrationScope,
    UpdateState,
    VoiceInputSettings,
  } from "./lib/types";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import MainArea from "./lib/components/MainArea.svelte";
  import StatusBar from "./lib/components/StatusBar.svelte";
  import AboutDialog from "./lib/components/AboutDialog.svelte";
  import OpenProject from "./lib/components/OpenProject.svelte";
  import AgentLaunchForm from "./lib/components/AgentLaunchForm.svelte";
  import LaunchProgressModal from "./lib/components/LaunchProgressModal.svelte";
  import MigrationModal from "./lib/components/MigrationModal.svelte";
  import CleanupModal from "./lib/components/CleanupModal.svelte";
  import QuitConfirmToast from "./lib/components/QuitConfirmToast.svelte";
  import ReportDialog from "./lib/components/ReportDialog.svelte";
  import {
    formatWindowTitle,
  } from "./lib/windowTitle";
  import { inferAgentId } from "./lib/agentUtils";
  import {
    AGENT_TAB_RESTORE_MAX_RETRIES,
    loadStoredProjectTabs,
    persistStoredProjectTabs,
    buildRestoredProjectTabs,
    shouldRetryAgentTabRestore,
    type StoredProjectTab,
    type StoredTerminalTab,
  } from "./lib/agentTabsPersistence";
  import {
    defaultAppTabs,
    reorderTabsByDrop,
    shouldAllowRestoredActiveTab,
    type TabDropPosition,
  } from "./lib/appTabs";
  import { getNextTabId, getPreviousTabId } from "./lib/tabNavigation";
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
  import { createSystemMonitor } from "./lib/systemMonitor.svelte";
  import {
    deduplicateByProjectPath,
    getWindowSession,
    loadWindowSessions,
    pruneWindowSessions,
    removeWindowSession,
    upsertWindowSession,
  } from "./lib/windowSessions";
  import {
    releaseWindowSessionRestoreLead,
    tryAcquireWindowSessionRestoreLead,
  } from "./lib/windowSessionRestoreLeader";
  import { collectScreenText } from "./lib/screenCapture";
  import {
    isAllowedExternalHttpUrl,
    openExternalUrl,
  } from "./lib/openExternalUrl";
  import { errorBus, type StructuredError } from "./lib/errorBus";
  import { toastBus } from "./lib/toastBus";
  import {
    AGENT_PASTE_HINT_DISMISSED_KEY,
    platformName,
    shouldShowAgentPasteHint,
  } from "./lib/terminal/pasteGuidance";
  import {
    buildDocsEditorCommand,
    isTerminalProcessEnded,
    isWindowsPlatform,
    shouldAutoCloseDocsEditorTab,
    type DocsEditorShellId,
  } from "./lib/docsEditor";

  interface SettingsUpdatedPayload {
    uiFontSize?: number;
    terminalFontSize?: number;
    uiFontFamily?: string;
    terminalFontFamily?: string;
    appLanguage?: SettingsData["app_language"];
    voiceInput?: VoiceInputSettings;
  }

  interface ProjectModeSpecIssuePayload {
    issueNumber: number;
    specId?: string | null;
    issueUrl?: string | null;
  }

  const SIDEBAR_WIDTH_STORAGE_KEY = "gwt.sidebar.width";
  const SIDEBAR_MODE_STORAGE_KEY = "gwt.sidebar.mode";
  const DEFAULT_SIDEBAR_WIDTH_PX = 260;
  const MIN_SIDEBAR_WIDTH_PX = 220;
  const MAX_SIDEBAR_WIDTH_PX = 520;
  type SidebarMode = "branch" | "projectMode";

  const DEFAULT_VOICE_INPUT_SETTINGS: VoiceInputSettings = {
    enabled: false,
    engine: "qwen3-asr",
    hotkey: "Mod+Shift+M",
    ptt_hotkey: "Mod+Shift+Space",
    language: "auto",
    quality: "balanced",
    model: "Qwen/Qwen3-ASR-1.7B",
  };
  const DEFAULT_UI_FONT_FAMILY =
    'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';
  const DEFAULT_TERMINAL_FONT_FAMILY =
    '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace';
  const DOCS_EDITOR_AUTO_CLOSE_POLL_MS = 1200;

  function clampSidebarWidth(widthPx: number): number {
    if (!Number.isFinite(widthPx)) return DEFAULT_SIDEBAR_WIDTH_PX;
    return Math.max(
      MIN_SIDEBAR_WIDTH_PX,
      Math.min(MAX_SIDEBAR_WIDTH_PX, Math.round(widthPx)),
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
      if (raw === "projectMode") {
        return "projectMode";
      }
      return raw === "branch" ? "branch" : "branch";
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

  function loadAgentPasteHintDismissed(): boolean {
    if (typeof window === "undefined") return false;
    try {
      return (
        window.localStorage.getItem(AGENT_PASTE_HINT_DISMISSED_KEY) === "1"
      );
    } catch {
      return false;
    }
  }

  function persistAgentPasteHintDismissed(): void {
    if (typeof window === "undefined") return;
    try {
      window.localStorage.setItem(AGENT_PASTE_HINT_DISMISSED_KEY, "1");
    } catch {
      // Ignore localStorage failures (e.g., disabled in strict environments).
    }
  }

  let projectPath: string | null = $state(null);
  let sidebarVisible: boolean = $state(true);
  let sidebarWidthPx: number = $state(loadSidebarWidth());
  let sidebarMode: SidebarMode = $state(loadSidebarMode());
  let showAgentLaunch: boolean = $state(false);
  let prefillIssue: GitHubIssueInfo | null = $state(null);
  let showCleanupModal: boolean = $state(false);
  let cleanupPreselectedBranch: string | null = $state(null);
  let showAbout: boolean = $state(false);
  let aboutInitialTab: "general" | "system" | "statistics" = $state("general");
  let showTerminalDiagnostics: boolean = $state(false);
  let appError: string | null = $state(null);
  let sidebarRefreshKey: number = $state(0);
  let worktreesEventAvailable: boolean = $state(false);
  let windowSessionRestoreStarted: boolean = false;
  let currentWindowLabel: string | null = $state(null);
  let selectedBranch: BranchInfo | null = $state(null);
  let currentBranch: string = $state("");

  let launchProgressOpen: boolean = $state(false);
  let launchJobId: string = $state("");
  let pendingLaunchRequest: LaunchAgentRequest | null = $state(null);
  let docsEditorAutoClosePaneIds: string[] = $state([]);

  type LaunchStepId = "fetch" | "validate" | "paths" | "conflicts" | "create" | "deps";
  let launchStep: LaunchStepId = $state("fetch");
  let launchDetail: string = $state("");
  let launchStatus: "running" | "ok" | "error" | "cancelled" = $state("running");
  let launchError: string | null = $state(null);
  const LAUNCH_STEP_IDS: LaunchStepId[] = [
    "fetch",
    "validate",
    "paths",
    "conflicts",
    "create",
    "deps",
  ];
  const LAUNCH_EVENT_BUFFER_LIMIT = 64;
  let launchJobStartPending = false;
  let bufferedLaunchProgressEvents: LaunchProgressPayload[] = [];
  let bufferedLaunchFinishedEvents: LaunchFinishedPayload[] = [];

  let migrationOpen: boolean = $state(false);
  let migrationSourceRoot: string = $state("");

  let tabs: Tab[] = $state(defaultAppTabs());
  let activeTabId: string = $state("projectMode");
  let agentPasteHintDismissed = loadAgentPasteHintDismissed();
  let agentPasteHintShownInSession = false;

  let agentTabsHydratedProjectPath: string | null = $state(null);
  let agentTabsRestoreToken = 0;
  const AGENT_TAB_RESTORE_RETRY_DELAY_MS = 150;
  const AGENT_TAB_RESTORE_RETRY_MAX_DELAY_MS = 1200;

  let terminalCount = $derived(
    tabs.filter((t) => t.type === "agent" || t.type === "terminal").length,
  );

  let agentTabBranches = $derived(
    tabs
      .filter((t) => t.type === "agent")
      .map((t) => normalizeBranchName(t.label))
      .filter((b) => b && b !== "Worktree" && b !== "Agent"),
  );

  let activeAgentTabBranch = $derived(
    (() => {
      const active = tabs.find((t) => t.id === activeTabId);
      if (!active || active.type !== "agent") return null;
      const branch = normalizeBranchName(active.label);
      if (!branch || branch === "Worktree" || branch === "Agent") return null;
      return branch;
    })()
  );

  let terminalDiagnosticsLoading: boolean = $state(false);
  let terminalDiagnostics: TerminalAnsiProbe | null = $state(null);
  let terminalDiagnosticsError: string | null = $state(null);

  let osEnvReady = $state(false);
  let startupOsEnvCaptureChecked = false;
  let startupOsEnvCaptureResolved = $state(false);
  let voiceInputSettings: VoiceInputSettings = $state(
    DEFAULT_VOICE_INPUT_SETTINGS,
  );
  let appLanguage: SettingsData["app_language"] = $state("auto");
  let voiceInputListening = $state(false);
  let voiceInputPreparing = $state(false);
  let voiceInputSupported = $state(true);
  let voiceInputAvailable = $state(false);
  let voiceInputAvailabilityReason: string | null = $state(null);
  let voiceInputError: string | null = $state(null);
  let voiceController: VoiceInputController | null = null;

  const systemMonitor = createSystemMonitor();

  let toastMessage = $state<string | null>(null);
  let toastTimeout: ReturnType<typeof setTimeout> | null = null;
  type ToastAction =
    | { kind: "apply-update"; latest: string }
    | { kind: "report-error"; error: StructuredError }
    | null;
  let toastAction = $state<ToastAction>(null);
  let lastUpdateToastVersion = $state<string | null>(null);

  let showOsEnvDebug = $state(false);
  let osEnvDebugData = $state<CapturedEnvInfo | null>(null);
  let osEnvDebugLoading = $state(false);
  let osEnvDebugError = $state<string | null>(null);
  let startupSkillScopeChecked = false;
  let skillScopePromptOpen = $state(false);
  let skillScopeSelection = $state<SkillRegistrationScope>("user");
  let skillScopePromptBusy = $state(false);
  let skillScopePromptError = $state<string | null>(null);
  type AvailableUpdateState = Extract<UpdateState, { state: "available" }>;

  function showToast(
    message: string,
    durationMs = 8000,
    action: ToastAction = null,
  ) {
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

  function showAvailableUpdateToast(s: AvailableUpdateState, force = false) {
    if (!force && lastUpdateToastVersion === s.latest) return;
    lastUpdateToastVersion = s.latest;

    if (s.asset_url) {
      showToast(`Update available: v${s.latest} (click update)`, 0, {
        kind: "apply-update",
        latest: s.latest,
      });
    } else {
      showToast(
        `Update available: v${s.latest}. Manual download required.`,
        15000,
      );
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

      const { invoke } = await import("$lib/tauriInvoke");
      await invoke("apply_app_update");
    } catch (err) {
      showToast(`Failed to apply update: ${toErrorMessage(err)}`);
    }
  }

  let reportDialogOpen = $state(false);
  let reportDialogMode = $state<"bug" | "feature">("bug");
  let reportDialogPrefillError = $state<StructuredError | undefined>(undefined);

  function showReportDialog(mode: "bug" | "feature", prefillError?: StructuredError) {
    reportDialogMode = mode;
    reportDialogPrefillError = prefillError;
    reportDialogOpen = true;
    // Close the toast
    toastMessage = null;
    toastAction = null;
    // Bring window to front so the report dialog is visible (#1256)
    import("@tauri-apps/api/window")
      .then(({ getCurrentWindow }) => getCurrentWindow().setFocus())
      .catch(() => {});
  }

  // Subscribe to toast bus for success/info notifications (SPEC-merge-pr FR-006)
  const unsubToastBus = toastBus.subscribe((event) => {
    showToast(event.message, event.durationMs ?? 5000);
  });

  // Subscribe to error bus for report-worthy errors
  const unsubErrorBus = errorBus.subscribe((error) => {
    if (error.severity === "error" || error.severity === "critical") {
      showToast(
        `Error: ${error.message}`,
        0,
        { kind: "report-error", error },
      );
    }
  });

  async function handleToastClick() {
    if (!toastAction) return;
    if (toastAction.kind === "apply-update") {
      await confirmAndApplyUpdate(toastAction.latest);
    }
  }

  function debugLaunchEvent(message: string, payload: unknown) {
    console.debug(`[launch] ${message}`, payload);
  }

  function clearBufferedLaunchEvents() {
    launchJobStartPending = false;
    bufferedLaunchProgressEvents = [];
    bufferedLaunchFinishedEvents = [];
  }

  function bufferLaunchProgressEvent(payload: LaunchProgressPayload) {
    if (bufferedLaunchProgressEvents.length >= LAUNCH_EVENT_BUFFER_LIMIT) {
      bufferedLaunchProgressEvents.shift();
    }
    bufferedLaunchProgressEvents.push(payload);
  }

  function bufferLaunchFinishedEvent(payload: LaunchFinishedPayload) {
    if (bufferedLaunchFinishedEvents.length >= LAUNCH_EVENT_BUFFER_LIMIT) {
      bufferedLaunchFinishedEvents.shift();
    }
    bufferedLaunchFinishedEvents.push(payload);
  }

  function applyLaunchProgressPayload(payload: LaunchProgressPayload) {
    if (launchStatus !== "running") return;
    const next = payload.step as LaunchStepId;
    if (LAUNCH_STEP_IDS.includes(next) && next !== launchStep) {
      launchStep = next;
    }
    launchDetail = (payload.detail ?? "").toString();
  }

  function applyLaunchFinishedPayload(payload: LaunchFinishedPayload) {
    if (payload.status === "cancelled") {
      handleLaunchModalClose();
      return;
    }
    if (payload.status === "ok" && payload.paneId) {
      launchStatus = "ok";
      handleLaunchSuccess(payload.paneId);
      handleLaunchModalClose();
      return;
    }
    const error = payload.error || "Launch failed.";
    const recoveredBranch = parseE1004BranchName(error);
    if (recoveredBranch && pendingLaunchRequest) {
      pendingLaunchRequest = {
        ...pendingLaunchRequest,
        branch: recoveredBranch,
      };
    }
    launchStatus = "error";
    launchError = error;
  }

  function flushBufferedLaunchEventsForActiveJob() {
    if (!launchJobId) {
      clearBufferedLaunchEvents();
      return;
    }
    const activeJobId = launchJobId;

    for (const payload of bufferedLaunchProgressEvents) {
      if (payload.jobId !== activeJobId) continue;
      applyLaunchProgressPayload(payload);
    }

    for (const payload of bufferedLaunchFinishedEvents) {
      if (payload.jobId !== activeJobId) continue;
      applyLaunchFinishedPayload(payload);
      if (!launchJobId || launchJobId !== activeJobId) break;
    }

    clearBufferedLaunchEvents();
  }

  // Poll OS env readiness at startup; stop once ready.
  $effect(() => {
    if (osEnvReady) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const { invoke } = await import("$lib/tauriInvoke");
        while (!cancelled && !osEnvReady) {
          const ready = await invoke<boolean>("is_os_env_ready");
          if (ready) {
            osEnvReady = true;
            return;
          }
          await new Promise((r) => setTimeout(r, 200));
        }
      } catch {
        /* ignore */
      }
    };
    poll();
    return () => {
      cancelled = true;
    };
  });

  // Listen for OS env fallback event and show toast.
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;
    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenFn = await listen<string>("os-env-fallback", (event) => {
          showToast(
            `Shell environment not loaded: ${event.payload}. Using process environment.`,
          );
        });
        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch {
        /* ignore */
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
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
        const { invoke } = await import("$lib/tauriInvoke");
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
        const unlistenFn = await listen<UpdateState>(
          "app-update-state",
          (event) => {
            const s = event.payload;
            if (s.state !== "available") return;
            showAvailableUpdateToast(s);
          },
        );
        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch {
        // Ignore when event API is unavailable.
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  });

  // Keep external URL behavior consistent across all rendered links.
  $effect(() => {
    if (typeof document === "undefined") return;

    const handleDocumentClick = (event: MouseEvent) => {
      const anchor = nearestAnchor(event.target);
      if (!anchor) return;
      if (!shouldHandleExternalLinkClick(event, anchor)) return;

      const rawHref = anchor.getAttribute("href");
      if (!rawHref) return;

      event.preventDefault();
      void openExternalUrl(rawHref);
    };

    document.addEventListener("click", handleDocumentClick, true);
    return () => {
      document.removeEventListener("click", handleDocumentClick, true);
    };
  });

  $effect(() => {
    void projectPath;
    void setWindowTitle();
    void applyAppearanceSettings();
  });

  $effect(() => {
    if (startupOsEnvCaptureChecked) return;
    startupOsEnvCaptureChecked = true;
    void checkOsEnvCaptureOnStartup();
  });

  $effect(() => {
    if (!startupOsEnvCaptureResolved) return;
    if (startupSkillScopeChecked) return;
    startupSkillScopeChecked = true;
    void checkSkillScopePromptOnStartup();
  });

  $effect(() => {
    if (windowSessionRestoreStarted) return;
    windowSessionRestoreStarted = true;
    const releaseDelayMs = 3000;

    (async () => {
      pruneWindowSessions();
      const label = await resolveCurrentWindowLabel();
      if (!label) return;
      const isRestoreLeader = await tryAcquireWindowSessionRestoreLead(label);

      const sessions = loadWindowSessions();
      const normalizedSessions = deduplicateByProjectPath(
        sessions.filter(
          (entry) => entry.label !== label && entry.projectPath,
        ),
      );

      if (isRestoreLeader) {
        try {
          for (const entry of normalizedSessions) {
            await openAndNormalizeWindowSession(entry.label, entry.projectPath);
          }
          await new Promise<void>((resolve) => setTimeout(resolve, releaseDelayMs));
          await restoreWindowSessionProject(label);
        } finally {
          await releaseWindowSessionRestoreLead(label);
        }
      } else {
        await restoreWindowSessionProject(label);
      }
    })();
  });

  // Remove session entry when the window is hidden via the close button.
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;
    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenFn = await listen("window-will-hide", async () => {
          const label = await resolveCurrentWindowLabel();
          if (label) removeWindowSession(label);
        });
        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch {
        /* not in Tauri runtime */
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  });

  // Best-effort: subscribe once and refresh Sidebar when worktrees change.
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenFn = await listen<unknown>(
          "worktrees-changed",
          (event) => {
            if (!projectPath) return;

            // If payload includes a project_path, only refresh the active project.
            const p = (event as { payload?: unknown }).payload;
            if (p && typeof p === "object" && "project_path" in p) {
              const raw = (p as { project_path?: unknown }).project_path;
              if (typeof raw === "string" && raw && raw !== projectPath) return;
            }

            sidebarRefreshKey++;
          },
        );

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
            removeTabLocal(`terminal-${event.payload.pane_id}`);
          },
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

  // Update terminal tab cwd and label when the shell's working directory changes.
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenFn = await listen<{ pane_id: string; cwd: string }>(
          "terminal-cwd-changed",
          (event) => {
            const { pane_id, cwd } = event.payload;
            tabs = tabs.map((tab) => {
              if (tab.type === "terminal" && tab.paneId === pane_id) {
                return { ...tab, cwd, label: terminalTabLabel(cwd) };
              }
              return tab;
            });
          },
        );

        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch (err) {
        console.error("Failed to setup terminal-cwd-changed listener:", err);
      }
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  });

  // Subscribe to launch-progress events at mount time to avoid race conditions.
  // LaunchProgressModal is a pure display component; all event handling lives here.
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;
    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenFn = await listen<LaunchProgressPayload>(
          "launch-progress",
          (event) => {
            const payload = event.payload;
            if (launchJobId) {
              if (payload.jobId !== launchJobId) {
                debugLaunchEvent("Ignored launch-progress for different job", payload);
                return;
              }
              applyLaunchProgressPayload(payload);
              return;
            }

            if (launchJobStartPending) {
              bufferLaunchProgressEvent(payload);
              debugLaunchEvent("Buffered launch-progress before jobId assignment", payload);
              return;
            }

            debugLaunchEvent("Ignored launch-progress without active job", payload);
          },
        );
        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch (err) {
        console.error("Failed to setup launch progress listener:", err);
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  });

  // Handle progress modal state on launch completion.
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenFn = await listen<LaunchFinishedPayload>(
          "launch-finished",
          (event) => {
            const payload = event.payload;

            // Progress modal state update (moved from LaunchProgressModal).
            if (launchJobId) {
              if (payload.jobId !== launchJobId) {
                debugLaunchEvent("Ignored launch-finished for different job", payload);
                return;
              }
              applyLaunchFinishedPayload(payload);
              return;
            }

            if (launchJobStartPending) {
              bufferLaunchFinishedEvent(payload);
              debugLaunchEvent("Buffered launch-finished before jobId assignment", payload);
              return;
            }

            debugLaunchEvent("Ignored launch-finished without active job", payload);
          },
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

  // Poll backend for launch job results.  Tauri events can be silently
  // lost, so we periodically ask the backend directly.  When the job has
  // finished, the stored result is applied exactly as a launch-finished
  // event would be.
  $effect(() => {
    if (!launchProgressOpen || !launchJobId || launchStatus !== "running") return;
    const jobId = launchJobId;
    const timer = window.setInterval(async () => {
      if (launchJobId !== jobId || launchStatus !== "running") return;
      try {
        const { invoke } = await import("$lib/tauriInvoke");
        const result = await invoke<{
          running: boolean;
          finished: LaunchFinishedPayload | null;
        }>("poll_launch_job", { jobId });

        if (result.running) return; // still going
        if (launchJobId !== jobId || launchStatus !== "running") return;

        if (result.finished) {
          // Apply the stored result as if it were a normal event.
          applyLaunchFinishedPayload(result.finished);
        } else {
          // No result stored – genuinely lost.
          launchStatus = "error";
          launchError =
            "Launch job ended unexpectedly. Please retry.";
        }
      } catch {
        /* ignore polling errors */
      }
    }, 1500);
    return () => window.clearInterval(timer);
  });

  // Close docs editor tabs automatically after vi exits.
  $effect(() => {
    if (docsEditorAutoClosePaneIds.length === 0) return;

    let polling = false;
    const timer = window.setInterval(() => {
      if (polling) return;
      polling = true;

      void (async () => {
        const paneIds = [...docsEditorAutoClosePaneIds];
        if (paneIds.length === 0) return;

        try {
          const { invoke } = await import("$lib/tauriInvoke");
          const terminals = await invoke<TerminalInfo[]>("list_terminals");
          const statusByPane = new Map(
            terminals.map((terminal) => [terminal.pane_id, terminal.status]),
          );

          for (const paneId of paneIds) {
            const status = statusByPane.get(paneId);
            if (!status) {
              removeDocsEditorAutoClosePane(paneId);
              continue;
            }

            if (!isTerminalProcessEnded(status)) continue;

            try {
              await invoke("close_terminal", { paneId });
            } catch {
              // Ignore if already closed.
            }
            removeDocsEditorAutoClosePane(paneId);
          }
        } catch {
          // Ignore polling errors.
        }
      })().finally(() => {
        polling = false;
      });
    }, DOCS_EDITOR_AUTO_CLOSE_POLL_MS);

    return () => window.clearInterval(timer);
  });

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  function isTauriRuntimeAvailable(): boolean {
    if (typeof window === "undefined") return false;
    return (
      typeof (window as Window & { __TAURI_INTERNALS__?: unknown })
        .__TAURI_INTERNALS__ !== "undefined"
    );
  }

  function nearestAnchor(target: EventTarget | null): HTMLAnchorElement | null {
    if (!(target instanceof Element)) return null;
    const anchor = target.closest("a[href]");
    return anchor instanceof HTMLAnchorElement ? anchor : null;
  }

  function shouldHandleExternalLinkClick(
    event: MouseEvent,
    anchor: HTMLAnchorElement,
  ): boolean {
    if (event.defaultPrevented) return false;
    if (event.button !== 0) return false;
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
      return false;
    }
    if (anchor.hasAttribute("download")) return false;

    const rawHref = anchor.getAttribute("href");
    if (!rawHref) return false;
    return isAllowedExternalHttpUrl(rawHref);
  }

  function clampFontSize(size: number): number {
    return Math.max(8, Math.min(24, Math.round(size)));
  }

  function normalizeVoiceInputSettings(
    value: Partial<VoiceInputSettings> | null | undefined,
  ): VoiceInputSettings {
    const engine = (value?.engine ?? "").trim().toLowerCase();
    const hotkey = (value?.hotkey ?? "").trim();
    const pttHotkey = (value?.ptt_hotkey ?? "").trim();
    const language = (value?.language ?? "").trim().toLowerCase();
    const quality = (value?.quality ?? "").trim().toLowerCase();
    const model = (value?.model ?? "").trim();
    const normalizedQuality =
      quality === "fast" || quality === "balanced" || quality === "accurate"
        ? (quality as VoiceInputSettings["quality"])
        : DEFAULT_VOICE_INPUT_SETTINGS.quality;
    const defaultModel =
      normalizedQuality === "fast" ? "Qwen/Qwen3-ASR-0.6B" : "Qwen/Qwen3-ASR-1.7B";

    return {
      enabled: !!value?.enabled,
      engine:
        engine === "qwen3-asr" || engine === "qwen" || engine === "whisper"
          ? "qwen3-asr"
          : DEFAULT_VOICE_INPUT_SETTINGS.engine,
      hotkey: hotkey.length > 0 ? hotkey : DEFAULT_VOICE_INPUT_SETTINGS.hotkey,
      ptt_hotkey:
        pttHotkey.length > 0 ? pttHotkey : DEFAULT_VOICE_INPUT_SETTINGS.ptt_hotkey,
      language:
        language === "ja" || language === "en" || language === "auto"
          ? (language as VoiceInputSettings["language"])
          : DEFAULT_VOICE_INPUT_SETTINGS.language,
      quality: normalizedQuality,
      model: model.length > 0 ? model : defaultModel,
    };
  }

  function normalizeAppLanguage(
    value: string | null | undefined,
  ): SettingsData["app_language"] {
    const language = (value ?? "").trim().toLowerCase();
    if (language === "ja" || language === "en" || language === "auto") {
      return language as SettingsData["app_language"];
    }
    return "auto";
  }

  function normalizeUiFontFamily(value: string | null | undefined): string {
    const family = (value ?? "").trim();
    return family.length > 0 ? family : DEFAULT_UI_FONT_FAMILY;
  }

  function normalizeTerminalFontFamily(value: string | null | undefined): string {
    const family = (value ?? "").trim();
    return family.length > 0 ? family : DEFAULT_TERMINAL_FONT_FAMILY;
  }

  function normalizeSkillScope(
    value: string | null | undefined,
  ): SkillRegistrationScope | null {
    const normalized = (value ?? "").trim().toLowerCase();
    if (
      normalized === "user" ||
      normalized === "project" ||
      normalized === "local"
    ) {
      return normalized as SkillRegistrationScope;
    }
    return null;
  }

  function hasSkillScopeConfigured(
    settings: Partial<SettingsData> | null | undefined,
  ): boolean {
    return normalizeSkillScope(
      settings?.agent_skill_registration_default_scope ?? null,
    ) !== null;
  }

  function applyUiFontSize(size: number) {
    document.documentElement.style.setProperty("--ui-font-base", `${size}px`);
  }

  function applyUiFontFamily(family: string | null | undefined) {
    document.documentElement.style.setProperty(
      "--ui-font-family",
      normalizeUiFontFamily(family),
    );
  }

  function applyTerminalFontSize(size: number) {
    (window as any).__gwtTerminalFontSize = size;
    window.dispatchEvent(
      new CustomEvent("gwt-terminal-font-size", { detail: size }),
    );
  }

  function applyTerminalFontFamily(family: string | null | undefined) {
    const normalized = normalizeTerminalFontFamily(family);
    document.documentElement.style.setProperty("--terminal-font-family", normalized);
    (window as any).__gwtTerminalFontFamily = normalized;
    window.dispatchEvent(
      new CustomEvent("gwt-terminal-font-family", { detail: normalized }),
    );
  }

  function applyVoiceInputSettings(
    value: Partial<VoiceInputSettings> | null | undefined,
  ) {
    voiceInputSettings = normalizeVoiceInputSettings(value);
    voiceController?.updateSettings();
  }

  function applyAppLanguage(value: string | null | undefined) {
    appLanguage = normalizeAppLanguage(value);
  }

  async function checkOsEnvCaptureOnStartup() {
    // OS env capture is now automatic (login_shell on Unix, process_env on Windows).
    // No prompt needed - just mark as resolved and check readiness.
    if (!isTauriRuntimeAvailable()) {
      startupOsEnvCaptureResolved = true;
      return;
    }

    try {
      const { invoke } = await import("$lib/tauriInvoke");
      osEnvReady = await invoke<boolean>("is_os_env_ready");
      startupOsEnvCaptureResolved = true;
    } catch (err) {
      startupOsEnvCaptureResolved = true;
      console.error("Failed to check os env capture status:", err);
    }
  }

  async function checkSkillScopePromptOnStartup() {
    if (!isTauriRuntimeAvailable()) return;
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const settings = await invoke<SettingsData>("get_settings");
      if (hasSkillScopeConfigured(settings)) {
        skillScopePromptOpen = false;
        return;
      }

      skillScopeSelection = "user";
      skillScopePromptError = null;
      skillScopePromptOpen = true;
    } catch (err) {
      skillScopePromptOpen = false;
      skillScopePromptError = null;
      console.error("Failed to check startup skill scope setting:", err);
    }
  }

  async function applyStartupSkillScopeSelection() {
    if (skillScopePromptBusy) return;
    skillScopePromptBusy = true;
    skillScopePromptError = null;
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const currentSettings = await invoke<SettingsData>("get_settings");
      const nextSettings: SettingsData = {
        ...currentSettings,
        agent_skill_registration_default_scope: skillScopeSelection,
        agent_skill_registration_codex_scope: null,
        agent_skill_registration_claude_scope: null,
        agent_skill_registration_gemini_scope: null,
      };

      await invoke("save_settings", { settings: nextSettings });
      await invoke("repair_skill_registration_cmd");
      skillScopePromptOpen = false;
    } catch (err) {
      skillScopePromptError = toErrorMessage(err);
    } finally {
      skillScopePromptBusy = false;
    }
  }

  async function rebuildAllBranchSessionSummaries(language: SettingsData["app_language"]) {
    if (!projectPath) return;
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      await invoke("rebuild_all_branch_session_summaries", {
        projectPath,
        preferredLanguage: language,
      });
    } catch (err) {
      showToast(`Failed to rebuild summaries: ${toErrorMessage(err)}`, 12000);
    }
  }

  function activeAgentPaneId(): string | null {
    const active = tabs.find((t) => t.id === activeTabId);
    if (
      (active?.type === "agent" || active?.type === "terminal") &&
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
      const { invoke } = await import("$lib/tauriInvoke");
      const settings =
        await invoke<
          Pick<
            SettingsData,
            | "ui_font_size"
            | "terminal_font_size"
            | "ui_font_family"
            | "terminal_font_family"
            | "app_language"
            | "voice_input"
          >
        >("get_settings");
      applyUiFontSize(clampFontSize(settings.ui_font_size ?? 13));
      applyTerminalFontSize(clampFontSize(settings.terminal_font_size ?? 13));
      applyUiFontFamily(settings.ui_font_family);
      applyTerminalFontFamily(settings.terminal_font_family);
      applyAppLanguage(settings.app_language);
      applyVoiceInputSettings(settings.voice_input);
    } catch {
      // Ignore: settings API not available outside Tauri runtime.
    }
  }

  async function resolveCurrentWindowLabel(): Promise<string | null> {
    if (currentWindowLabel) return currentWindowLabel;

    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const label = await invoke<string>("get_current_window_label");
      const next = label?.trim();
      if (!next) return null;
      currentWindowLabel = next;
      return next;
    } catch {
      return null;
    }
  }

  async function updateWindowSession(projectPathForWindow: string | null) {
    const label = await resolveCurrentWindowLabel();
    if (!label) return;

    if (projectPathForWindow) {
      upsertWindowSession(label, projectPathForWindow);
      return;
    }
    removeWindowSession(label);
  }

  async function openAndNormalizeWindowSession(label: string, projectPath: string) {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const openedLabelRaw = await invoke<unknown>("open_gwt_window", { label });
      if (typeof openedLabelRaw !== "string") return;

      const openedLabel = openedLabelRaw.trim();
      if (!openedLabel || openedLabel === label) {
        return;
      }

      removeWindowSession(label);
      upsertWindowSession(openedLabel, projectPath);
    } catch {
      // Ignore restore failures: startup session restore is best-effort.
    }
  }

  async function restoreWindowSessionProject(label: string) {
    const session = getWindowSession(label);
    if (!session?.projectPath) return false;

    try {
      await openProjectAndApplyCurrentWindow(session.projectPath);
      return true;
    } catch {
      removeWindowSession(label);
      return false;
    }
  }

  function handleOpenedProjectPath(path: string) {
    projectPath = path;
    fetchCurrentBranch();
    void updateWindowSession(path);
  }

  async function openProjectAndApplyCurrentWindow(path: string): Promise<OpenProjectResult> {
    const { invoke } = await import("$lib/tauriInvoke");
    const result = await invoke<OpenProjectResult>("open_project", { path });
    if (result.action === "opened") {
      handleOpenedProjectPath(result.info.path);
    }
    return result;
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
    handleOpenedProjectPath(path);
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

  function ensureProjectModeTab() {
    const existing = tabs.find((t) => t.type === "projectMode" || t.id === "projectMode");
    if (existing) return;

    const tab: Tab = {
      id: "projectMode",
      label: "Project Mode",
      type: "projectMode",
    };
    tabs = [...tabs, tab];
  }

  function handleSidebarModeChange(next: SidebarMode) {
    if (sidebarMode === next) return;
    if (next === "projectMode") {
      ensureProjectModeTab();
      activeTabId = "projectMode";
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

  async function handleOpenCiLog(runId: number) {
    if (!projectPath) return;
    try {
      const workingDir = projectPath;
      const { invoke } = await import("$lib/tauriInvoke");
      const paneId = await invoke<string>("spawn_shell", { workingDir });

      const label = `CI #${runId}`;
      const newTab: Tab = {
        id: `terminal-${paneId}`,
        label,
        type: "terminal",
        paneId,
        cwd: workingDir || undefined,
      };
      tabs = [...tabs, newTab];
      activeTabId = newTab.id;

      // Resolve and read logs via backend so bare-repo project roots still work.
      const logOutput = await invoke<string>("fetch_ci_log", {
        projectPath,
        runId,
      });
      const delimiterBase = "__GWT_CI_LOG__";
      let delimiter = delimiterBase;
      let delimiterSuffix = 0;
      while (logOutput.includes(delimiter)) {
        delimiterSuffix += 1;
        delimiter = `${delimiterBase}_${delimiterSuffix}`;
      }
      const normalized = logOutput.endsWith("\n") ? logOutput : `${logOutput}\n`;
      const cmd = `cat <<'${delimiter}'\n${normalized}${delimiter}\n`;
      const data = Array.from(new TextEncoder().encode(cmd));
      await invoke("write_terminal", { paneId, data });
    } catch (err) {
      console.error("Failed to open CI log:", err);
    }
  }

  async function fetchCurrentBranch() {
    if (!projectPath) return;
    try {
      const { invoke } = await import("$lib/tauriInvoke");
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
    return trimmed.startsWith("origin/")
      ? trimmed.slice("origin/".length)
      : trimmed;
  }

  function worktreeTabLabel(branch: string): string {
    const b = branch.trim();
    return b ? normalizeBranchName(b) : "Worktree";
  }

  function parseE1004BranchName(errorMessage: string): string | null {
    const match = errorMessage.match(/\[E1004\]\s+Branch already exists:\s*(.+)$/m);
    if (!match) return null;
    const raw = match[1]?.trim() ?? "";
    if (!raw) return null;
    if (raw.startsWith("'") && raw.endsWith("'") && raw.length > 1) {
      return raw.slice(1, -1);
    }
    return raw;
  }

  function terminalTabLabel(
    pathLike: string | null | undefined,
    fallback = "Terminal",
  ): string {
    const value = (pathLike ?? "").trim();
    if (!value) return fallback;
    const parts = value.split(/[\\/]+/).filter(Boolean);
    if (parts.length === 0) {
      return value.startsWith("/") || value.startsWith("\\") ? value : fallback;
    }
    return parts[parts.length - 1] || fallback;
  }

  async function resolveNewTerminalWorkingDir(): Promise<string | null> {
    if (!projectPath) return null;

    const branchName = selectedBranch?.name?.trim() || "";
    if (!branchName) return projectPath;

    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const worktrees = await invoke<WorktreeInfo[]>("list_worktrees", {
        projectPath,
      });
      const normalizedBranchName = normalizeBranchName(branchName);
      const selectedWorktree = worktrees.find((worktree) => {
        const worktreeBranch = (worktree.branch ?? "").trim();
        if (!worktreeBranch) return false;
        return normalizeBranchName(worktreeBranch) === normalizedBranchName;
      });
      const selectedPath = selectedWorktree?.path?.trim() || "";
      if (selectedPath) return selectedPath;
    } catch (err) {
      console.error("Failed to resolve selected worktree path:", err);
    }

    return projectPath;
  }

  async function handleNewTerminal() {
    try {
      const workingDir = await resolveNewTerminalWorkingDir();
      const { invoke } = await import("$lib/tauriInvoke");
      const paneId = await invoke<string>("spawn_shell", { workingDir });
      const cwd = workingDir || "~";
      const label = terminalTabLabel(cwd);
      const newTab: Tab = {
        id: `terminal-${paneId}`,
        label,
        type: "terminal",
        paneId,
        cwd: workingDir || undefined,
      };
      tabs = [...tabs, newTab];
      activeTabId = newTab.id;
    } catch (err) {
      console.error("Failed to spawn new terminal:", err);
    }
  }

  function detectPlatform(): string {
    return platformName(
      navigator as Navigator & {
        userAgentData?: { platform?: string | null } | null;
      },
    );
  }

  function addDocsEditorAutoClosePane(paneId: string) {
    const normalized = paneId.trim();
    if (!normalized) return;
    if (docsEditorAutoClosePaneIds.includes(normalized)) return;
    docsEditorAutoClosePaneIds = [...docsEditorAutoClosePaneIds, normalized];
  }

  function removeDocsEditorAutoClosePane(paneId: string) {
    const normalized = paneId.trim();
    if (!normalized) return;
    if (!docsEditorAutoClosePaneIds.includes(normalized)) return;
    docsEditorAutoClosePaneIds = docsEditorAutoClosePaneIds.filter(
      (id) => id !== normalized,
    );
  }

  async function resolveWindowsDocsShellId(): Promise<DocsEditorShellId> {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const settings = await invoke<Pick<SettingsData, "default_shell">>("get_settings");
      const shell = (settings.default_shell ?? "").trim().toLowerCase();
      if (shell === "wsl" || shell === "powershell" || shell === "cmd") {
        return shell;
      }
    } catch {
      // Fall back to cmd when settings are unavailable.
    }
    return "cmd";
  }

  async function handleOpenDocsEditor(worktreePath: string) {
    const workingDir = worktreePath.trim();
    if (!workingDir) return;

    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const platform = detectPlatform();
      const shellId = isWindowsPlatform(platform)
        ? await resolveWindowsDocsShellId()
        : undefined;
      const paneId = await invoke<string>(
        "spawn_shell",
        shellId ? { workingDir, shell: shellId } : { workingDir },
      );

      const tab: Tab = {
        id: `terminal-${paneId}`,
        label: "Docs Edit",
        type: "terminal",
        paneId,
        cwd: workingDir,
      };
      tabs = [...tabs, tab];
      activeTabId = tab.id;

      const command = `${buildDocsEditorCommand(platform, shellId)}\n`;
      const data = Array.from(new TextEncoder().encode(command));
      await invoke("write_terminal", { paneId, data });
      if (shouldAutoCloseDocsEditorTab(platform, shellId)) {
        addDocsEditorAutoClosePane(paneId);
      }
    } catch (err) {
      showToast(`Failed to open docs editor: ${toErrorMessage(err)}`, 8000);
      console.error("Failed to open docs editor:", err);
    }
  }

  async function respawnStoredTerminalTabs(
    storedTabs: StoredTerminalTab[],
    targetProjectPath: string,
    token: number,
  ): Promise<{ tabs: Tab[]; paneIdMap: Map<string, string> }> {
    if (storedTabs.length === 0) {
      return { tabs: [], paneIdMap: new Map() };
    }

    const restoredTabs: Tab[] = [];
    const paneIdMap = new Map<string, string>();

    try {
      const { invoke } = await import("$lib/tauriInvoke");
      for (const storedTab of storedTabs) {
        if (
          projectPath !== targetProjectPath ||
          agentTabsRestoreToken !== token
        ) {
          break;
        }

        const cwdCandidate = (storedTab.cwd ?? "").trim();
        const workingDir = cwdCandidate || targetProjectPath;
        try {
          const paneId = await invoke<string>("spawn_shell", { workingDir });
          paneIdMap.set(storedTab.paneId, paneId);
          restoredTabs.push({
            id: `terminal-${paneId}`,
            label: terminalTabLabel(workingDir, storedTab.label || "Terminal"),
            type: "terminal",
            paneId,
            cwd: workingDir,
          });
        } catch (err) {
          console.error("Failed to respawn stored terminal tab:", err);
        }
      }
    } catch (err) {
      console.error("Failed to load Tauri API for terminal restore:", err);
    }

    return { tabs: restoredTabs, paneIdMap };
  }

  function tabMergeKey(tab: Tab): string {
    if (
      (tab.type === "agent" || tab.type === "terminal") &&
      typeof tab.paneId === "string" &&
      tab.paneId.length > 0
    ) {
      return `pane:${tab.paneId}`;
    }
    return `id:${tab.id}`;
  }

  function normalizePrimaryTab(tab: Tab): Tab {
    if (tab.type === "projectMode" || tab.id === "projectMode") {
      return {
        ...tab,
        id: "projectMode",
        label: "Project Mode",
        type: "projectMode",
      };
    }
    return tab;
  }

  function mergeRestoredTabs(existingTabs: Tab[], restoredTabs: Tab[]): Tab[] {
    const merged = restoredTabs.map((tab) => normalizePrimaryTab(tab));
    const seen = new Set(merged.map(tabMergeKey));

    for (const tab of existingTabs) {
      const normalized = normalizePrimaryTab(tab);
      const key = tabMergeKey(normalized);
      if (seen.has(key)) continue;
      seen.add(key);
      merged.push(normalized);
    }

    if (!merged.some((tab) => tab.id === "projectMode")) {
      merged.unshift({
        id: "projectMode",
        label: "Project Mode",
        type: "projectMode",
      });
    }

    return merged;
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
    // Reset progress state before starting the job.
    launchStep = "fetch";
    launchDetail = "";
    launchStatus = "running";
    launchError = null;
    launchJobStartPending = true;
    bufferedLaunchProgressEvents = [];
    bufferedLaunchFinishedEvents = [];

    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const jobId = await invoke<string>("start_launch_job", { request });

      pendingLaunchRequest = request;
      launchJobId = jobId;
      launchJobStartPending = false;
      launchProgressOpen = true;
      flushBufferedLaunchEventsForActiveJob();
    } catch (err) {
      clearBufferedLaunchEvents();
      throw err;
    }
  }

  async function handleLaunchCancel() {
    if (!launchJobId) {
      handleLaunchModalClose();
      return;
    }
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      await invoke("cancel_launch_job", { jobId: launchJobId });
      handleLaunchModalClose();
    } catch (err) {
      console.error("Failed to cancel launch job:", err);
      launchStatus = "error";
      launchError = "Failed to send cancel request. Close this dialog and retry.";
    }
  }

  function handleLaunchModalClose() {
    launchProgressOpen = false;
    launchJobId = "";
    pendingLaunchRequest = null;
    clearBufferedLaunchEvents();
    launchStatus = "running";
    launchStep = "fetch";
    launchDetail = "";
    launchError = null;
  }

  function handleUseExistingBranch() {
    const req = pendingLaunchRequest;
    if (!req) return;
    const retryRequest: LaunchAgentRequest = { ...req };
    delete retryRequest.createBranch;
    handleLaunchModalClose();
    void handleAgentLaunch(retryRequest);
  }

  function handleLaunchSuccess(paneId: string) {
    const req = pendingLaunchRequest;
    const requestedBranch = req?.branch?.trim() ?? "";
    const label = req ? worktreeTabLabel(requestedBranch) : "Worktree";
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

    const needsBranchResolution = requestedBranch.length === 0;
    if (needsBranchResolution || !newTab.agentId) {
      void (async () => {
        try {
          const { invoke } = await import("$lib/tauriInvoke");
          const terminals = await invoke<TerminalInfo[]>("list_terminals");
          const terminal = terminals.find((t) => t.pane_id === paneId);
          if (!terminal) return;

          const updates: Partial<Tab> = {};
          if (needsBranchResolution) {
            const resolvedBranch = terminal.branch_name?.trim() ?? "";
            if (resolvedBranch) {
              updates.label = worktreeTabLabel(resolvedBranch);
            }
          }

          const terminalAgentId = inferAgentId(terminal?.agent_name);
          if (!newTab.agentId && terminalAgentId) {
            updates.agentId = terminalAgentId;
          }

          if (Object.keys(updates).length === 0) return;

          tabs = tabs.map((t) =>
            t.id === newTab.id ? { ...t, ...updates } : t,
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
    const removed = tabs[idx];
    if (removed?.paneId) {
      removeDocsEditorAutoClosePane(removed.paneId);
    }

    const nextTabs = tabs.filter((t) => t.id !== tabId);
    tabs = nextTabs;

    if (activeTabId !== tabId) return;
    const fallback =
      nextTabs[idx] ??
      nextTabs[idx - 1] ??
      nextTabs[nextTabs.length - 1] ??
      null;
    activeTabId = fallback?.id ?? "";
  }

  async function handleTabClose(tabId: string) {
    const tab = tabs.find((t) => t.id === tabId);
    if (tab?.paneId) {
      try {
        const { invoke } = await import("$lib/tauriInvoke");
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

  function handleTabReorder(
    dragTabId: string,
    overTabId: string,
    position: TabDropPosition,
  ) {
    const nextTabs = reorderTabsByDrop(tabs, dragTabId, overTabId, position);
    if (nextTabs === tabs) return;
    tabs = nextTabs;
  }

  function openSettingsTab() {
    const existing = tabs.find(
      (t) => t.type === "settings" || t.id === "settings",
    );
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

  function openIssuesTab() {
    const existing = tabs.find(
      (t) => t.type === "issues" || t.id === "issues",
    );
    if (existing) {
      activeTabId = existing.id;
      return;
    }

    const tab: Tab = {
      id: "issues",
      label: "Issues",
      type: "issues",
    };
    tabs = [...tabs, tab];
    activeTabId = tab.id;
  }

  function openPullRequestsTab() {
    const existing = tabs.find(
      (t) => t.type === "prs" || t.id === "prs",
    );
    if (existing) {
      activeTabId = existing.id;
      return;
    }
    const tab: Tab = {
      id: "prs",
      label: "Pull Requests",
      type: "prs",
    };
    tabs = [...tabs, tab];
    activeTabId = tab.id;
  }

  function openProjectIndexTab() {
    const existing = tabs.find(
      (t) => t.type === "projectIndex" || t.id === "projectIndex",
    );
    if (existing) {
      activeTabId = existing.id;
      return;
    }
    const tab: Tab = {
      id: "projectIndex",
      label: "Project Index",
      type: "projectIndex",
    };
    tabs = [...tabs, tab];
    activeTabId = tab.id;
  }

  function handleIssueCountChange(count: number) {
    tabs = tabs.map((t) =>
      t.id === "issues" ? { ...t, label: count > 0 ? `Issues (${count})` : "Issues" } : t,
    );
  }

  function handleWorkOnIssueFromTab(issue: GitHubIssueInfo) {
    prefillIssue = issue;
    showAgentLaunch = true;
  }

  function handleSwitchToWorktreeFromTab(branchName: string) {
    // Find the matching agent tab and switch to it
    const agentTab = tabs.find(
      (t) => t.type === "agent" && normalizeBranchName(t.label) === normalizeBranchName(branchName),
    );
    if (agentTab) {
      activeTabId = agentTab.id;
      return;
    }
    // If no tab exists, select the branch in the sidebar
    sidebarRefreshKey++;
  }

  function openIssueSpecTab(payload: ProjectModeSpecIssuePayload) {
    const issueNumber = Number(payload.issueNumber);
    if (!Number.isFinite(issueNumber) || issueNumber <= 0) return;
    const specId = payload.specId?.trim() || undefined;
    const label = specId ? `Spec ${specId}` : `Issue #${issueNumber}`;

    const existing = tabs.find((t) => t.type === "issueSpec" || t.id === "issueSpec");
    if (existing) {
      tabs = tabs.map((t) =>
        t.id === existing.id
          ? {
              ...t,
              label,
              issueNumber,
              specId,
            }
          : t
      );
      activeTabId = existing.id;
      return;
    }

    const tab: Tab = {
      id: "issueSpec",
      label,
      type: "issueSpec",
      issueNumber,
      ...(specId ? { specId } : {}),
    };
    tabs = [...tabs, tab];
    activeTabId = tab.id;
  }

  function getActiveTerminalPaneId(): string | null {
    const active = tabs.find((t) => t.id === activeTabId);
    if (!active || (active.type !== "agent" && active.type !== "terminal")) {
      return null;
    }
    return active.paneId && active.paneId.length > 0 ? active.paneId : null;
  }

  function getActiveEditableElement(
    mode: "copy" | "paste" = "paste",
  ):
    | HTMLInputElement
    | HTMLTextAreaElement
    | HTMLElement
    | null {
    if (typeof document === "undefined") return null;
    const el = document.activeElement;
    if (!el) return null;

    if (el instanceof HTMLInputElement && !el.disabled) {
      if (mode === "copy" || !el.readOnly) return el;
    }
    if (el instanceof HTMLTextAreaElement && !el.disabled) {
      if (mode === "copy" || !el.readOnly) return el;
    }
    if (el instanceof HTMLElement && el.isContentEditable) {
      return el;
    }

    return null;
  }

  function getEditableSelectionText(
    target: HTMLInputElement | HTMLTextAreaElement | HTMLElement,
  ): string {
    if (
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement
    ) {
      const start = target.selectionStart;
      const end = target.selectionEnd;
      if (start === null || end === null || start === end) return "";
      const from = Math.min(start, end);
      const to = Math.max(start, end);
      return target.value.slice(from, to);
    }

    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0) return "";
    const range = selection.getRangeAt(0);
    if (!target.contains(range.commonAncestorContainer)) return "";
    return selection.toString();
  }

  async function fallbackMenuEditAction(action: "copy" | "paste") {
    const target = getActiveEditableElement(action);
    if (!target) {
      if (action === "copy") {
        const sel = window.getSelection()?.toString();
        if (sel && navigator.clipboard?.writeText) {
          await navigator.clipboard.writeText(sel);
        }
      }
      return;
    }

    if (action === "copy") {
      const sel = getEditableSelectionText(target);
      if (sel && navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(sel);
      }
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

    if (
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement
    ) {
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

  let copyFlashActive = $state(false);

  async function handleScreenCopy() {
    const activeTab = tabs.find((t) => t.id === activeTabId);
    const text = collectScreenText({
      branch: currentBranch,
      activeTab: activeTab?.label ?? activeTabId,
      activeTabType: activeTab?.type,
      activePaneId:
        activeTab?.type === "agent" || activeTab?.type === "terminal"
          ? activeTab.paneId
          : undefined,
    });
    try {
      await navigator.clipboard.writeText(text);
      copyFlashActive = true;
      setTimeout(() => { copyFlashActive = false; }, 300);
      showToast("Copied to clipboard", 2000);
    } catch {
      showToast("Failed to copy screen text", 4000);
    }
  }

  function emitTerminalEditAction(action: "copy" | "paste") {
    const editableEl = getActiveEditableElement(action);
    if (editableEl && !editableEl.closest("[data-pane-id]")) {
      void fallbackMenuEditAction(action);
      return;
    }

    const paneId = getActiveTerminalPaneId();
    if (!paneId) {
      void fallbackMenuEditAction(action);
      return;
    }

    if (typeof window === "undefined") return;

    window.dispatchEvent(
      new CustomEvent("gwt-terminal-edit-action", {
        detail: { action, paneId },
      }),
    );
  }

  async function syncWindowAgentTabs() {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const visibleTabs = tabs
        .filter((t) => t.type === "agent" || t.type === "terminal")
        .map((t) => ({ id: t.id, label: t.label, tab_type: t.type }));
      const activeVisibleTabId = visibleTabs.some((t) => t.id === activeTabId)
        ? activeTabId
        : null;
      await invoke("sync_window_agent_tabs", {
        request: {
          tabs: visibleTabs,
          activeTabId: activeVisibleTabId,
        },
      });
    } catch {
      // Ignore: not available outside Tauri runtime.
    }
  }

  async function handleMenuAction(action: string) {
    if (action.startsWith("focus-agent-tab::")) {
      const tabId = action.slice("focus-agent-tab::".length).trim();
      if (
        tabId &&
        tabs.some(
          (t) =>
            t.id === tabId && (t.type === "agent" || t.type === "terminal"),
        )
      ) {
        activeTabId = tabId;
      }
      return;
    }

    // Handle dynamic "open-recent-project::<path>" actions before the switch.
    if (action.startsWith("open-recent-project::")) {
      const recentPath = action.slice("open-recent-project::".length);
      if (recentPath) {
        try {
          const { invoke } = await import("$lib/tauriInvoke");
          const probe = await invoke<ProbePathResult>("probe_path", {
            path: recentPath,
          });

          if (probe.kind === "gwtProject" && probe.projectPath) {
            await openProjectAndApplyCurrentWindow(probe.projectPath);
            return;
          }

          if (probe.kind === "migrationRequired" && probe.migrationSourceRoot) {
            migrationSourceRoot = probe.migrationSourceRoot;
            migrationOpen = true;
            return;
          }

          appError = probe.message || "Failed to open recent project.";
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
            const { invoke } = await import("$lib/tauriInvoke");
            const probe = await invoke<ProbePathResult>("probe_path", {
              path: selected as string,
            });

            if (probe.kind === "gwtProject" && probe.projectPath) {
              await openProjectAndApplyCurrentWindow(probe.projectPath);
              break;
            }

            if (
              probe.kind === "migrationRequired" &&
              probe.migrationSourceRoot
            ) {
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
          // Kill plain terminal tab PTYs before clearing state.
          const terminalPanes = tabs
            .filter((t) => t.type === "terminal" && t.paneId)
            .map((t) => t.paneId as string);
          if (terminalPanes.length > 0) {
            try {
              const { invoke } = await import("$lib/tauriInvoke");
              await Promise.all(
                terminalPanes.map((paneId) =>
                  invoke("close_terminal", { paneId }).catch(() => {}),
                ),
              );
            } catch {
              // Ignore: not available outside Tauri runtime.
            }
          }

          // Clear backend state (window-scoped) best-effort.
          try {
            const { invoke } = await import("$lib/tauriInvoke");
            await invoke("close_project");
          } catch {
            // Ignore: not available outside Tauri runtime.
          }

          projectPath = null;
          void updateWindowSession(null);
          tabs = defaultAppTabs();
          activeTabId = "projectMode";
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
      case "new-terminal":
        if (projectPath) {
          await handleNewTerminal();
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
      case "git-issues":
        openIssuesTab();
        break;
      case "git-pull-requests":
        openPullRequestsTab();
        break;
      case "project-index":
        openProjectIndexTab();
        break;
      case "check-updates":
        {
          try {
            const { invoke } = await import("$lib/tauriInvoke");
            const s = await invoke<UpdateState>("check_app_update", {
              force: true,
            });
            switch (s.state) {
              case "up_to_date":
                showToast("Up to date.");
                break;
              case "available":
                // Manual check should surface availability even when startup already notified.
                showAvailableUpdateToast(s, true);
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
        aboutInitialTab = "general";
        showAbout = true;
        break;
      case "report-issue":
        showReportDialog("bug");
        break;
      case "suggest-feature":
        showReportDialog("feature");
        break;
      case "list-terminals":
        // Just switch to first terminal tab if any
        {
          const firstAgent = tabs.find(
            (t) => t.type === "agent" || t.type === "terminal",
          );
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
      case "screen-copy":
        void handleScreenCopy();
        break;
      case "debug-os-env":
        showOsEnvDebug = true;
        osEnvDebugLoading = true;
        osEnvDebugError = null;
        (async () => {
          try {
            const { invoke } = await import("$lib/tauriInvoke");
            osEnvDebugData = await invoke<CapturedEnvInfo>(
              "get_captured_environment",
            );
          } catch (e) {
            osEnvDebugError = String(e);
          } finally {
            osEnvDebugLoading = false;
          }
        })();
        break;
      case "new-terminal": {
        try {
          const workingDir = await resolveNewTerminalWorkingDir();
          const { invoke } = await import("$lib/tauriInvoke");
          const paneId = await invoke<string>("spawn_shell", { workingDir });
          const cwd = workingDir || "~";
          const label = terminalTabLabel(cwd);
          const newTab: Tab = {
            id: `terminal-${paneId}`,
            label,
            type: "terminal",
            paneId,
            cwd: workingDir || undefined,
          };
          tabs = [...tabs, newTab];
          activeTabId = newTab.id;
        } catch (err) {
          console.error("Failed to spawn shell:", err);
        }
        break;
      }
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
          const { invoke } = await import("$lib/tauriInvoke");
          terminalDiagnostics = await invoke<TerminalAnsiProbe>(
            "probe_terminal_ansi",
            {
              paneId,
            },
          );
        } catch (err) {
          terminalDiagnosticsError = `Failed to probe terminal: ${toErrorMessage(err)}`;
        } finally {
          terminalDiagnosticsLoading = false;
        }
        break;
      }
      case "previous-tab": {
        const prevId = getPreviousTabId(tabs, activeTabId);
        if (prevId) activeTabId = prevId;
        break;
      }
      case "next-tab": {
        const nextId = getNextTabId(tabs, activeTabId);
        if (nextId) activeTabId = nextId;
        break;
      }
    }
  }

  $effect(() => {
    void tabs;
    void activeTabId;
    void syncWindowAgentTabs();
  });

  $effect(() => {
    void tabs;
    void activeTabId;

    if (typeof navigator === "undefined") return;
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    const currentPlatform = platformName(
      navigator as Navigator & {
        userAgentData?: { platform?: string | null } | null;
      },
    );

    if (
      !shouldShowAgentPasteHint({
        activeTabType: activeTab?.type,
        platform: currentPlatform,
        dismissed: agentPasteHintDismissed,
        shownInSession: agentPasteHintShownInSession,
      })
    ) {
      return;
    }

    showToast(
      "Agent tab paste: Ctrl+Shift+V for text on Windows/Linux. Ctrl+V is passed through to the agent (for example, Codex image paste).",
      10000,
    );
    agentPasteHintShownInSession = true;
    if (!agentPasteHintDismissed) {
      agentPasteHintDismissed = true;
      persistAgentPasteHintDismissed();
    }
  });

  async function restoreProjectAgentTabs(
    targetProjectPath: string,
    token: number,
    attempt = 0,
  ) {
    const stored = loadStoredProjectTabs(targetProjectPath);

    // Even if no stored state exists, mark hydrated so persistence can proceed.
    if (!stored) {
      if (
        projectPath === targetProjectPath &&
        agentTabsRestoreToken === token
      ) {
        agentTabsHydratedProjectPath = targetProjectPath;
      }
      return;
    }

    let terminals: TerminalInfo[] = [];
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      terminals = await invoke<TerminalInfo[]>("list_terminals");
    } catch {
      // Ignore: not available outside Tauri runtime.
    }

    if (projectPath !== targetProjectPath || agentTabsRestoreToken !== token) {
      return;
    }

    const restored = buildRestoredProjectTabs(stored, terminals);
    const storedAgentTabsCount = stored.tabs.filter(
      (t) => t.type === "agent",
    ).length;
    const restoredAgentTabsCount = restored.tabs.filter(
      (t) => t.type === "agent",
    ).length;
    const shouldRetry = shouldRetryAgentTabRestore(
      storedAgentTabsCount,
      restoredAgentTabsCount,
      attempt,
      AGENT_TAB_RESTORE_MAX_RETRIES,
    );

    if (
      shouldRetry &&
      projectPath === targetProjectPath &&
      agentTabsRestoreToken === token
    ) {
      setTimeout(() => {
        void restoreProjectAgentTabs(targetProjectPath, token, attempt + 1);
      }, getAgentTabRestoreDelayMs(attempt));
      return;
    }

    const respawnedTerminalResult = await respawnStoredTerminalTabs(
      restored.terminalTabsToRespawn,
      targetProjectPath,
      token,
    );

    if (projectPath !== targetProjectPath || agentTabsRestoreToken !== token) {
      return;
    }

    const mergedTabs = mergeRestoredTabs(tabs, [
      ...restored.tabs,
      ...respawnedTerminalResult.tabs,
    ]);
    tabs = mergedTabs;

    const allowOverrideActive = shouldAllowRestoredActiveTab(activeTabId);
    if (allowOverrideActive) {
      if (
        restored.activeTabId &&
        mergedTabs.some((tab) => tab.id === restored.activeTabId)
      ) {
        activeTabId = restored.activeTabId;
      } else if (restored.activeTerminalPaneIdToRespawn) {
        const paneId = respawnedTerminalResult.paneIdMap.get(
          restored.activeTerminalPaneIdToRespawn,
        );
        if (paneId) {
          activeTabId = `terminal-${paneId}`;
        }
      }
    } else if (!mergedTabs.some((tab) => tab.id === activeTabId)) {
      activeTabId = mergedTabs[0]?.id ?? "projectMode";
    }

    agentTabsHydratedProjectPath = targetProjectPath;
  }

  // Restore persisted tabs when a project is opened.
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

  // Persist tabs per project (best-effort).
  $effect(() => {
    void projectPath;
    void tabs;
    void activeTabId;
    void agentTabsHydratedProjectPath;

    if (!projectPath) return;
    if (agentTabsHydratedProjectPath !== projectPath) return;

    const storedTabs: StoredProjectTab[] = [];
    for (const tab of tabs) {
      if (tab.type === "agent") {
        if (typeof tab.paneId !== "string" || tab.paneId.length === 0) continue;
        storedTabs.push({
          type: "agent",
          paneId: tab.paneId,
          label: tab.label,
          ...(tab.agentId ? { agentId: tab.agentId } : {}),
        });
        continue;
      }
      if (tab.type === "terminal") {
        if (typeof tab.paneId !== "string" || tab.paneId.length === 0) continue;
        storedTabs.push({
          type: "terminal",
          paneId: tab.paneId,
          label: tab.label,
          ...(tab.cwd ? { cwd: tab.cwd } : {}),
        });
        continue;
      }
      if (
        tab.type === "projectMode" ||
        tab.type === "settings" ||
        tab.type === "versionHistory" ||
        tab.type === "issues"
      ) {
        const staticType = tab.type === "projectMode" ? "projectMode" : tab.type;
        const staticId = tab.id === "projectMode" ? "projectMode" : tab.id;
        const staticLabel =
          tab.type === "projectMode" ? "Project Mode" : tab.label;
        storedTabs.push({
          type: staticType,
          id: staticId,
          label: staticLabel,
        });
      }
    }

    const storedActiveTabId = storedTabs.some((tab) => {
      if (tab.type === "agent") return `agent-${tab.paneId}` === activeTabId;
      if (tab.type === "terminal")
        return `terminal-${tab.paneId}` === activeTabId;
      return tab.id === activeTabId;
    })
      ? activeTabId
      : null;

    persistStoredProjectTabs(projectPath, {
      tabs: storedTabs,
      activeTabId: storedActiveTabId,
    });
  });

  // Native menubar integration (Tauri emits "menu-action" to the focused window).
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;

    (async () => {
      if (!isTauriRuntimeAvailable()) {
        return;
      }

      try {
        const { setupMenuActionListener } = await import(
          "./lib/menuAction"
        );
        const unlistenFn = await setupMenuActionListener((action) => {
          void handleMenuAction(action);
        });
        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
        console.info("menu listener ready");
      } catch (err) {
        if (cancelled) return;
        console.error("Failed to initialize menu action listener:", err);
        appError = `Menu integration failed: ${toErrorMessage(err)}`;
      }
    })();

    return () => {
      cancelled = true;
      if (unlisten) {
        unlisten();
      }
    };
  });

  // System monitor lifecycle: start polling, destroy on teardown.
  $effect(() => {
    systemMonitor.start();
    return () => {
      systemMonitor.destroy();
    };
  });

  $effect(() => {
    const controller = new VoiceInputController({
      getSettings: readVoiceInputSettingsForController,
      getFallbackTerminalPaneId: readVoiceFallbackTerminalPaneId,
      onStateChange: (state: VoiceControllerState) => {
        voiceInputListening = state.listening;
        voiceInputPreparing = state.preparing;
        voiceInputSupported = state.supported;
        voiceInputAvailable = state.available;
        voiceInputAvailabilityReason = state.availabilityReason;
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
      voiceInputPreparing = false;
      voiceInputError = null;
      voiceInputSupported = true;
      voiceInputAvailable = false;
      voiceInputAvailabilityReason = null;
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
      if (typeof detail.uiFontFamily === "string") {
        applyUiFontFamily(detail.uiFontFamily);
      }
      if (typeof detail.terminalFontFamily === "string") {
        applyTerminalFontFamily(detail.terminalFontFamily);
      }
      if (typeof detail.appLanguage === "string") {
        const nextLanguage = normalizeAppLanguage(detail.appLanguage);
        const changed = nextLanguage !== appLanguage;
        applyAppLanguage(nextLanguage);
        if (changed) {
          void rebuildAllBranchSessionSummaries(nextLanguage);
        }
      }
      if (detail.voiceInput) {
        applyVoiceInputSettings(detail.voiceInput);
      }
    }

    window.addEventListener("gwt-settings-updated", onSettingsUpdated);
    return () =>
      window.removeEventListener("gwt-settings-updated", onSettingsUpdated);
  });

  $effect(() => {
    function onOpenIssueSpec(event: Event) {
      const payload = (event as CustomEvent<ProjectModeSpecIssuePayload>).detail;
      if (!payload) return;
      openIssueSpecTab(payload);
    }
    window.addEventListener("gwt-project-mode-open-spec-issue", onOpenIssueSpec);
    return () =>
      window.removeEventListener(
        "gwt-project-mode-open-spec-issue",
        onOpenIssueSpec,
      );
  });

  // Keyboard shortcut fallbacks: these mirror native menu accelerators so that
  // shortcuts still work even when xterm or another element has swallowed the
  // key event before Tauri's accelerator layer can process it.
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
      if (
        e.ctrlKey &&
        e.code === "Backquote" &&
        !e.shiftKey &&
        !e.altKey &&
        !e.metaKey
      ) {
        e.preventDefault();
        void handleMenuAction("new-terminal");
      }
      // Cmd+O / Ctrl+O → Open Project
      if (
        e.key === "o" &&
        (e.metaKey || e.ctrlKey) &&
        !e.shiftKey &&
        !e.altKey
      ) {
        e.preventDefault();
        void handleMenuAction("open-project");
      }
      // Cmd+, / Ctrl+, → Settings
      if (
        e.key === "," &&
        (e.metaKey || e.ctrlKey) &&
        !e.shiftKey &&
        !e.altKey
      ) {
        e.preventDefault();
        void handleMenuAction("open-settings");
      }
      // Cmd+Shift+[ / Ctrl+Shift+[ → Previous Tab
      if (
        e.key === "{" &&
        e.shiftKey &&
        (e.metaKey || e.ctrlKey) &&
        !e.altKey
      ) {
        e.preventDefault();
        void handleMenuAction("previous-tab");
      }
      // Cmd+Shift+] / Ctrl+Shift+] → Next Tab
      if (
        e.key === "}" &&
        e.shiftKey &&
        (e.metaKey || e.ctrlKey) &&
        !e.altKey
      ) {
        e.preventDefault();
        void handleMenuAction("next-tab");
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
          {activeAgentTabBranch}
          {appLanguage}
          onResize={handleSidebarResize}
          onBranchSelect={handleBranchSelect}
          onBranchActivate={handleBranchActivate}
          onCleanupRequest={handleCleanupRequest}
          onLaunchAgent={requestAgentLaunch}
          onQuickLaunch={handleAgentLaunch}
          onNewTerminal={handleNewTerminal}
          onOpenDocsEditor={handleOpenDocsEditor}
          onOpenCiLog={handleOpenCiLog}
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
        onTabReorder={handleTabReorder}
        onWorkOnIssue={handleWorkOnIssueFromTab}
        onSwitchToWorktree={handleSwitchToWorktreeFromTab}
        onIssueCountChange={handleIssueCountChange}
      />
    </div>
    <StatusBar
      {projectPath}
      {currentBranch}
      {terminalCount}
      {osEnvReady}
      voiceInputEnabled={voiceInputSettings.enabled}
      voiceInputListening={voiceInputListening}
      voiceInputPreparing={voiceInputPreparing}
      voiceInputSupported={voiceInputSupported}
      voiceInputAvailable={voiceInputAvailable}
      voiceInputAvailabilityReason={voiceInputAvailabilityReason}
      voiceInputError={voiceInputError}
    />
  </div>
{/if}

{#if showAgentLaunch}
  <AgentLaunchForm
    projectPath={projectPath as string}
    selectedBranch={selectedBranch?.name ?? currentBranch}
    {osEnvReady}
    {prefillIssue}
    onLaunch={handleAgentLaunch}
    onClose={() => { showAgentLaunch = false; prefillIssue = null; }}
  />
{/if}

<QuitConfirmToast />

<CleanupModal
  open={showCleanupModal}
  preselectedBranch={cleanupPreselectedBranch}
  refreshKey={sidebarRefreshKey}
  projectPath={projectPath ?? ""}
  {agentTabBranches}
  onClose={() => (showCleanupModal = false)}
/>

<AboutDialog
  open={showAbout}
  initialTab={aboutInitialTab}
  cpuUsage={systemMonitor.cpuUsage}
  memUsed={systemMonitor.memUsed}
  memTotal={systemMonitor.memTotal}
  gpuInfos={systemMonitor.gpuInfos}
  onclose={() => (showAbout = false)}
/>

{#if showTerminalDiagnostics}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay modal-overlay" onclick={() => (showTerminalDiagnostics = false)}>
    <div class="diag-dialog modal-dialog-shell" onclick={(e) => e.stopPropagation()}>
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
            <span class="diag-value mono"
              >{terminalDiagnostics.bytes_scanned}</span
            >
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
            <span class="diag-value mono"
              >{terminalDiagnostics.color_sgr_count}</span
            >
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
              No color SGR codes were detected in the tail of the scrollback.
              This usually means the program did not emit ANSI colors (for
              example, output was captured or treated as non-interactive).
            </p>
            <p class="diag-muted">Try forcing color output:</p>
            <pre class="diag-code mono">git -c color.ui=always diff</pre>
            <pre class="diag-code mono">rg --color=always PATTERN</pre>
          </div>
        {:else}
          <div class="diag-hint">
            <p>
              Color SGR codes were detected. If you still do not see colors, the
              issue is likely in the terminal rendering path.
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
      await openProjectAndApplyCurrentWindow(p);
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
  step={launchStep}
  detail={launchDetail}
  status={launchStatus}
  error={launchError}
  onCancel={handleLaunchCancel}
  onClose={handleLaunchModalClose}
  onUseExisting={handleUseExistingBranch}
/>
{#if skillScopePromptOpen}
  <div class="overlay modal-overlay">
    <div class="scope-dialog modal-dialog-shell" role="dialog" aria-modal="true" aria-label="Skill registration scope">
      <h2>Skill Registration Scope</h2>
      <p class="scope-dialog-text">
        Select where managed skills and plugins are auto-registered. You can change this later in
        Settings.
      </p>
      <div class="scope-choice-grid">
        <button
          class={`scope-choice ${skillScopeSelection === "user" ? "active" : ""}`}
          onclick={() => (skillScopeSelection = "user")}
          disabled={skillScopePromptBusy}
        >
          <strong>User</strong>
          <span>~/.codex, ~/.gemini, ~/.claude</span>
        </button>
        <button
          class={`scope-choice ${skillScopeSelection === "project" ? "active" : ""}`}
          onclick={() => (skillScopeSelection = "project")}
          disabled={skillScopePromptBusy}
        >
          <strong>Project</strong>
          <span>&lt;repo&gt;/.codex, .gemini, .claude</span>
        </button>
        <button
          class={`scope-choice ${skillScopeSelection === "local" ? "active" : ""}`}
          onclick={() => (skillScopeSelection = "local")}
          disabled={skillScopePromptBusy}
        >
          <strong>Local</strong>
          <span>&lt;repo&gt;/.codex/skills.local etc.</span>
        </button>
      </div>
      {#if skillScopePromptError}
        <p class="scope-dialog-error">{skillScopePromptError}</p>
      {/if}
      <div class="scope-dialog-actions">
        <button
          class="about-close"
          onclick={() => {
            skillScopePromptOpen = false;
            skillScopePromptError = null;
          }}
          disabled={skillScopePromptBusy}
        >
          Skip for now
        </button>
        <button class="about-close" onclick={() => void applyStartupSkillScopeSelection()} disabled={skillScopePromptBusy}>
          {skillScopePromptBusy ? "Applying..." : "Apply and Continue"}
        </button>
      </div>
    </div>
  </div>
{/if}
{#if showOsEnvDebug}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay modal-overlay" onclick={() => (showOsEnvDebug = false)}>
    <div class="env-debug-dialog modal-dialog-shell" onclick={(e) => e.stopPropagation()}>
      <h3>Captured Environment</h3>
      {#if osEnvDebugLoading}
        <p class="env-debug-loading">Loading...</p>
      {:else if osEnvDebugError}
        <p class="env-debug-error">{osEnvDebugError}</p>
      {:else if osEnvDebugData}
        <div class="env-debug-meta">
          <span
            >Source: <strong
              >{osEnvDebugData.source === "login_shell"
                ? "Login Shell"
                : osEnvDebugData.source === "std_env_fallback"
                  ? "Process Env (fallback)"
                  : osEnvDebugData.source}</strong
            ></span
          >
          {#if osEnvDebugData.reason}
            <span class="env-debug-reason">Reason: {osEnvDebugData.reason}</span
            >
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
      <button class="about-close" onclick={() => (showOsEnvDebug = false)}
        >Close</button
      >
    </div>
  </div>
{/if}

{#if appError}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay modal-overlay" onclick={() => (appError = null)}>
    <div class="error-dialog modal-dialog-shell" onclick={(e) => e.stopPropagation()}>
      <h2>Error</h2>
      <p class="error-text">{appError}</p>
      <button class="about-close" onclick={() => (appError = null)}>
        Close
      </button>
    </div>
  </div>
{/if}

{#if copyFlashActive}
  <div class="copy-flash"></div>
{/if}

{#if toastMessage}
  <div class="toast-container">
    <div class="toast-message">
      <span>{toastMessage}</span>
      {#if toastAction?.kind === "apply-update"}
        <button class="toast-action" onclick={handleToastClick}>Update</button>
      {:else if toastAction?.kind === "report-error"}
        {@const reportError = toastAction.error}
        <button class="toast-action" onclick={() => showReportDialog("bug", reportError)}>Report</button>
      {/if}
      <button
        class="toast-close"
        aria-label="Close"
        onclick={() => {
          toastMessage = null;
          toastAction = null;
        }}
      >
        &times;
      </button>
    </div>
  </div>
{/if}

<ReportDialog
  open={reportDialogOpen}
  mode={reportDialogMode}
  prefillError={reportDialogPrefillError}
  projectPath={projectPath ?? ""}
  screenCaptureBranch={currentBranch}
  screenCaptureActiveTab={tabs.find((t) => t.id === activeTabId)?.label ?? activeTabId}
  onclose={() => { reportDialogOpen = false; }}
  onsuccess={(result) => {
    reportDialogOpen = false;
    showToast(`Issue #${result.number} created successfully.`, 8000);
  }}
/>

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
    z-index: var(--z-modal-base);
  }

  .mono {
    font-family: monospace;
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

  .scope-dialog {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 12px;
    padding: 24px 26px;
    max-width: 680px;
    width: min(680px, 92vw);
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4);
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .scope-dialog h2 {
    font-size: var(--ui-font-xl);
    font-weight: 800;
    color: var(--text-primary);
    margin: 0;
  }

  .scope-dialog-text {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--ui-font-md);
    line-height: 1.5;
  }

  .scope-choice-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 10px;
  }

  .scope-choice {
    text-align: left;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 10px;
    color: var(--text-secondary);
    padding: 12px;
    cursor: pointer;
    font-family: inherit;
    font-size: var(--ui-font-md);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .scope-choice strong {
    color: var(--text-primary);
    font-size: var(--ui-font-md);
  }

  .scope-choice span {
    font-family: monospace;
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    line-height: 1.3;
  }

  .scope-choice:hover:not(:disabled),
  .scope-choice.active {
    border-color: var(--accent);
    background: var(--bg-surface);
  }

  .scope-choice:disabled {
    opacity: 0.7;
    cursor: default;
  }

  .scope-dialog-actions {
    display: flex;
    justify-content: flex-end;
  }

  .scope-dialog-error {
    margin: 0;
    color: rgb(255, 160, 160);
    font-size: var(--ui-font-sm);
  }

  @media (max-width: 860px) {
    .scope-choice-grid {
      grid-template-columns: 1fr;
    }
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
    z-index: var(--z-toast);
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
    font-size: 20px;
    padding: 4px 8px;
    border-radius: 4px;
    line-height: 1;
  }

  .toast-close:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
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

  .env-debug-loading,
  .env-debug-error {
    font-size: 13px;
    padding: 12px 0;
  }

  .env-debug-error {
    color: var(--text-error, #f38ba8);
  }

  .copy-flash {
    position: fixed;
    inset: 0;
    background: var(--accent, #89b4fa);
    opacity: 0;
    pointer-events: none;
    z-index: var(--z-overlay-flash);
    animation: copy-flash-anim 0.25s ease-out forwards;
  }

  @keyframes copy-flash-anim {
    0% { opacity: 0; }
    30% { opacity: 0.12; }
    100% { opacity: 0; }
  }
</style>
