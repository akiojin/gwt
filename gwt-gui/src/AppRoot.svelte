<script lang="ts">
  import type {
    BranchBrowserPanelConfig,
    BranchBrowserPanelState,
    MaterializeWorktreeResult,
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
    UpdateState,
    VoiceInputSettings,
  } from "./lib/types";
  import {
    createToastState,
    showToast as showToastRuntime,
    showAvailableUpdateToast as showAvailableUpdateToastRuntime,
    dismissToast as dismissToastRuntime,
    setupToastSubscriptions,
    type ToastState,
    type ToastAction as ToastActionRT,
    type ShowToastCallbacks,
  } from "./lib/appToastRuntime";
  import {
    createVoiceInputState,
    applyVoiceInputSettings as applyVoiceInputSettingsRT,
    resetVoiceInputTransientState,
    syncControllerState,
    setupVoiceController,
    type VoiceInputState,
  } from "./lib/appVoiceInputRuntime";
  import {
    createModalState,
    showReportDialogRuntime,
    openAboutDialogRuntime,
    openCleanupModalRuntime,
    type ModalState,
  } from "./lib/appModalStateRuntime";
  import {
    createAppearanceState,
    applyAppearanceSettingsRuntime,
    checkOsEnvCaptureOnStartupRuntime,
    clampFontSizeRuntime as clampFontSizeAppearance,
    normalizeAppLanguageRuntime as normalizeAppLanguageAppearance,
    normalizeUiFontFamilyRuntime,
    normalizeTerminalFontFamilyRuntime,
    normalizeVoiceInputSettingsRuntime as normalizeVoiceInputSettingsAppearance,
    applyUiFontSizeRuntime,
    applyUiFontFamilyRuntime,
    applyTerminalFontSizeRuntime,
    applyTerminalFontFamilyRuntime,
    DEFAULT_UI_FONT_FAMILY as DEFAULT_UI_FONT_FAMILY_APPEARANCE,
    DEFAULT_TERMINAL_FONT_FAMILY as DEFAULT_TERMINAL_FONT_FAMILY_APPEARANCE,
    DEFAULT_VOICE_INPUT_SETTINGS as DEFAULT_VOICE_INPUT_SETTINGS_APPEARANCE,
    type AppearanceState,
  } from "./lib/appAppearanceRuntime";
  import {
    createLaunchState,
    bufferLaunchProgressEventRuntime,
    bufferLaunchFinishedEventRuntime,
    LAUNCH_STEP_IDS as LAUNCH_STEP_IDS_RT,
    LAUNCH_EVENT_BUFFER_LIMIT as LAUNCH_EVENT_BUFFER_LIMIT_RT,
    type LaunchState,
    type LaunchStepId as LaunchStepIdRT,
  } from "./lib/appLaunchStateRuntime";
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
  import TerminalDiagnosticsDialog from "./lib/components/TerminalDiagnosticsDialog.svelte";
  import CapturedEnvironmentDialog from "./lib/components/CapturedEnvironmentDialog.svelte";
  import AppErrorDialog from "./lib/components/AppErrorDialog.svelte";
  import ToastBanner from "./lib/components/ToastBanner.svelte";
  import { formatWindowTitle } from "./lib/windowTitle";
  import {
    buildWindowMenuTabsSignature,
    buildWindowMenuVisibleTabs,
    resolveActiveWindowMenuTabId,
    shouldKeepSnapshotActiveTabCache,
  } from "./lib/windowMenuSync";
  import { inferAgentId } from "./lib/agentUtils";
  import {
    AGENT_TAB_RESTORE_MAX_RETRIES,
    persistStoredProjectTabs,
  } from "./lib/agentTabsPersistence";
  import {
    createDefaultAgentCanvasViewport,
    type AgentCanvasTileLayout,
  } from "./lib/agentCanvas";
  import {
    defaultAppTabs,
    type TabDropPosition,
    reorderTabsByDrop,
    shouldAllowRestoredActiveTab,
  } from "./lib/appTabs";
  import { getNextTabId, getPreviousTabId } from "./lib/tabNavigation";
  import type { VoiceControllerState } from "./lib/voice/voiceInputController";
  import { VoiceInputController } from "./lib/voice/voiceInputController";
  import { createSystemMonitor } from "./lib/systemMonitor.svelte";
  import {
    deduplicateByProjectPath,
    loadWindowSessions,
    pruneWindowSessions,
    removeWindowSession,
    upsertWindowSession,
  } from "./lib/windowSessions";
  import {
    releaseWindowSessionRestoreLead,
    tryAcquireWindowSessionRestoreLead,
  } from "./lib/windowSessionRestoreLeader";
  import {
    openAndNormalizeRestoredWindowSession,
    restoreCurrentWindowSession,
  } from "./lib/windowSessionRestore";
  import { collectScreenText } from "./lib/screenCapture";
  import {
    startupProfilingTracker,
    type StartupFrontendMetric,
  } from "./lib/startupProfiling";
  import {
    isAllowedExternalHttpUrl,
    openExternalUrl,
  } from "./lib/openExternalUrl";
  import { errorBus, type StructuredError } from "./lib/errorBus";
  import { recordFrontendMetric } from "./lib/profiling.svelte";
  import { toastBus } from "./lib/toastBus";
  import {
    AGENT_PASTE_HINT_DISMISSED_KEY,
    platformName,
    shouldShowAgentPasteHint,
  } from "./lib/terminal/pasteGuidance";
  import {
    setupAppUpdateStateListenerEffect,
    setupDocsEditorAutoCloseEffect,
    setupExternalUrlHandlerEffect,
    setupLaunchFinishedListenerEffect,
    setupLaunchPollEffect,
    setupLaunchProgressListenerEffect,
    setupMenuActionListenerEffect,
    setupOsEnvFallbackListenerEffect,
    setupOsEnvReadyPollingEffect,
    setupProfilingEffect,
    setupStartupDiagnosticsEffect,
    setupStartupUpdateCheckEffect,
    setupTerminalClosedListenerEffect,
    setupTerminalCwdChangedListenerEffect,
    setupWindowSessionRestoreEffect,
    setupWindowWillHideListenerEffect,
    setupWorktreesChangedListenerEffect,
  } from "./lib/appEffects";
  import {
    buildDocsEditorCommand,
    isTerminalProcessEnded,
    isWindowsPlatform,
    shouldAutoCloseDocsEditorTab,
    type DocsEditorShellId,
  } from "./lib/docsEditor";
  import { runAppMenuAction } from "./lib/appMenuAction";
  import {
    checkForUpdates,
    closeProjectAndTerminals,
    focusAgentTabInCanvas,
    listTerminalSessions,
    openOsEnvDebug,
    openProjectViaDialog,
    openRecentProjectPath,
    openTerminalDiagnostics,
  } from "./lib/appMenuHandlers";
  import {
    applyLaunchFinishedRuntime,
    applyLaunchProgressRuntime,
    buildUseExistingBranchRetryRequest,
    closeLaunchModalRuntime,
    flushBufferedLaunchEventsRuntime,
  } from "./lib/appLaunchRuntime";
  import { createAppE2EHooks } from "./lib/appE2EHooks";
  import {
    applyRestoredWindowSessionRuntime,
    handleOpenedProjectPathRuntime,
    openProjectAndApplyCurrentWindowRuntime,
    resolveCurrentWindowLabelRuntime,
    updateWindowSessionRuntime,
  } from "./lib/appProjectRuntime";
  import {
    clampFontSizeRuntime,
    fallbackMenuEditActionRuntime,
    getActiveTerminalPaneIdRuntime,
    isTauriRuntimeAvailableRuntime,
    shouldHandleExternalLinkClickRuntime,
    toErrorMessageRuntime,
  } from "./lib/appUiRuntime";
  import {
    buildStoredProjectTabsSnapshot,
    getAgentTabRestoreDelayMs,
    restoreProjectTabsRuntime,
  } from "./lib/appProjectTabsRuntime";
  import {
    findAgentTabByBranchName,
    normalizeBranchName,
    resolveWorktreeTabLabel,
    syncAgentTabLabels,
  } from "./lib/worktreeTabLabels";

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
    issueUrl?: string | null;
  }

  interface StartupDiagnostics {
    startupTrace: boolean;
    disableTray: boolean;
    disableLoginShellCapture: boolean;
    disableHeartbeatWatchdog: boolean;
    disableSessionWatcher: boolean;
    disableStartupUpdateCheck: boolean;
    disableProfiling: boolean;
    disableTabRestore: boolean;
    disableWindowSessionRestore: boolean;
  }

  const DEFAULT_STARTUP_DIAGNOSTICS: StartupDiagnostics = {
    startupTrace: false,
    disableTray: false,
    disableLoginShellCapture: false,
    disableHeartbeatWatchdog: false,
    disableSessionWatcher: false,
    disableStartupUpdateCheck: false,
    disableProfiling: false,
    disableTabRestore: false,
    disableWindowSessionRestore: false,
  };

  const ISSUE_CACHE_WARMUP_DELAY_MS = 2_000;

  const DEFAULT_VOICE_INPUT_SETTINGS = DEFAULT_VOICE_INPUT_SETTINGS_APPEARANCE;
  const DEFAULT_UI_FONT_FAMILY = DEFAULT_UI_FONT_FAMILY_APPEARANCE;
  const DEFAULT_TERMINAL_FONT_FAMILY = DEFAULT_TERMINAL_FONT_FAMILY_APPEARANCE;
  const DOCS_EDITOR_AUTO_CLOSE_POLL_MS = 1200;
  const DEFAULT_BRANCH_BROWSER_STATE: BranchBrowserPanelState = {
    filter: "Local",
    query: "",
    selectedBranchName: null,
  };

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
  let showAgentLaunch: boolean = $state(false);
  let prefillIssue: GitHubIssueInfo | null = $state(null);
  const modal: ModalState = $state(createModalState());
  let worktreeInventoryRefreshKey: number = $state(0);
  let issueCacheWarmupTimer: ReturnType<typeof setTimeout> | null = null;
  let issueCacheWarmupProjectPath: string | null = null;
  let worktreesEventAvailable: boolean = $state(false);
  let windowSessionRestoreStarted: boolean = false;
  let currentWindowLabel: string | null = $state(null);
  let selectedBranch: BranchInfo | null = $state(null);
  let currentBranch: string = $state("");

  const launch: LaunchState = $state(createLaunchState());
  type LaunchStepId = LaunchStepIdRT;
  const LAUNCH_STEP_IDS = LAUNCH_STEP_IDS_RT;
  const LAUNCH_EVENT_BUFFER_LIMIT = LAUNCH_EVENT_BUFFER_LIMIT_RT;

  let migrationOpen: boolean = $state(false);
  let migrationSourceRoot: string = $state("");

  let tabs: Tab[] = $state(defaultAppTabs());
  let activeTabId: string = $state("agentCanvas");
  let lastWindowMenuTabsSignature: string | null = null;
  let lastWindowMenuActiveTabId: string | null = null;
  let agentPasteHintDismissed = loadAgentPasteHintDismissed();
  let agentPasteHintShownInSession = false;

  let agentTabsHydratedProjectPath: string | null = $state(null);
  let agentTabsRestoreToken = 0;
  let projectHydrationToken = 0;
  let activeStartupProfileToken: string | null = null;
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
    })(),
  );
  let selectedCanvasSessionTabId: string | null = $state(null);
  let selectedCanvasTileId: string | null = $state(null);
  let canvasViewport = $state(createDefaultAgentCanvasViewport());
  let canvasTileLayouts = $state<Record<string, AgentCanvasTileLayout>>({});
  let canvasWorktrees: WorktreeInfo[] = $state([]);
  let selectedCanvasWorktreeBranch: string | null = $state(null);
  let selectedCanvasWorktreePath: string | null = $state(null);
  let branchBrowserState = $state<BranchBrowserPanelState>(
    DEFAULT_BRANCH_BROWSER_STATE,
  );
  const appearance: AppearanceState = $state(createAppearanceState());
  let startupOsEnvCaptureChecked = false;
  let startupOsEnvCaptureResolved = $state(false);
  let startupDiagnostics: StartupDiagnostics | null = $state(null);
  const voice: VoiceInputState = $state(createVoiceInputState());
  let appLanguage: SettingsData["app_language"] = $state("auto");
  let voiceController: VoiceInputController | null = null;
  let branchBrowserConfig = $derived<BranchBrowserPanelConfig | undefined>(
    projectPath
      ? {
          projectPath,
          refreshKey: worktreeInventoryRefreshKey,
          initialFilter: branchBrowserState.filter,
          initialQuery: branchBrowserState.query,
          selectedBranchName: branchBrowserState.selectedBranchName,
          onStateChange: (state) => {
            const unchanged =
              branchBrowserState.filter === state.filter &&
              branchBrowserState.query === state.query &&
              branchBrowserState.selectedBranchName === state.selectedBranchName;
            if (unchanged) return;
            branchBrowserState = state;
          },
          selectedBranch,
          currentBranch,
          agentTabBranches,
          activeAgentTabBranch,
          appLanguage,
          onBranchSelect: handleBranchSelect,
          onBranchActivate: handleBranchActivate,
          onCleanupRequest: handleCleanupRequest,
          onLaunchAgent: requestAgentLaunch,
          onQuickLaunch: handleAgentLaunch,
          onNewTerminal: handleNewTerminal,
          onOpenDocsEditor: handleOpenDocsEditor,
          onOpenCiLog: handleOpenCiLog,
          onDisplayNameChanged: handleBranchDisplayNameChanged,
        }
      : undefined,
  );

  const systemMonitor = createSystemMonitor();

  const toast: ToastState = $state(createToastState());
  let toastMessage = $state<string | null>(null);
  let toastAction = $state<ToastActionRT>(null);
  type AvailableUpdateState = Extract<UpdateState, { state: "available" }>;

  const toastCallbacks: ShowToastCallbacks = {
    setState: (message, action) => {
      toastMessage = message;
      toastAction = action;
    },
  };

  function showToast(
    message: string,
    durationMs = 8000,
    action: ToastActionRT = null,
  ) {
    showToastRuntime(toast, message, durationMs, action, toastCallbacks);
  }

  function showAvailableUpdateToast(s: AvailableUpdateState, force = false) {
    showAvailableUpdateToastRuntime(toast, s, force, toastCallbacks);
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

  function showReportDialog(
    mode: "bug" | "feature",
    prefillError?: StructuredError,
  ) {
    showReportDialogRuntime(modal, mode, prefillError);
    // Close the toast
    dismissToastRuntime(toast, toastCallbacks);
    // Bring window to front so the report dialog is visible (#1256)
    import("@tauri-apps/api/window")
      .then(({ getCurrentWindow }) => getCurrentWindow().setFocus())
      .catch(() => {});
  }

  // Subscribe to toast bus and error bus for notifications
  const unsubToastSubscriptions = setupToastSubscriptions({
    toastBus,
    errorBus,
    state: toast,
    cb: toastCallbacks,
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
    launch.jobStartPending = false;
    launch.bufferedProgressEvents = [];
    launch.bufferedFinishedEvents = [];
  }

  function bufferLaunchProgressEvent(payload: LaunchProgressPayload) {
    bufferLaunchProgressEventRuntime(launch, payload);
  }

  function bufferLaunchFinishedEvent(payload: LaunchFinishedPayload) {
    bufferLaunchFinishedEventRuntime(launch, payload);
  }

  function applyLaunchProgressPayload(payload: LaunchProgressPayload) {
    applyLaunchProgressRuntime({
      payload,
      launchStatus: launch.status,
      launchStepIds: LAUNCH_STEP_IDS,
      currentLaunchStep: launch.step,
      setLaunchStep: (step) => {
        launch.step = step;
      },
      setLaunchDetail: (detail) => {
        launch.detail = detail;
      },
    });
  }

  function applyLaunchFinishedPayload(payload: LaunchFinishedPayload) {
    applyLaunchFinishedRuntime({
      payload,
      pendingLaunchRequest: launch.pendingRequest,
      parseE1004BranchName,
      setPendingLaunchRequest: (request) => {
        launch.pendingRequest = request;
      },
      setLaunchStatus: (status) => {
        launch.status = status;
      },
      setLaunchError: (error) => {
        launch.error = error;
      },
      onCancelled: () => {
        handleLaunchModalClose();
      },
      onSuccess: (paneId) => {
        handleLaunchSuccess(paneId);
      },
    });
  }

  function flushBufferedLaunchEventsForActiveJob() {
    flushBufferedLaunchEventsRuntime({
      launchJobId: launch.jobId,
      bufferedLaunchProgressEvents: launch.bufferedProgressEvents,
      bufferedLaunchFinishedEvents: launch.bufferedFinishedEvents,
      clearBufferedLaunchEvents,
      applyLaunchProgressPayload,
      applyLaunchFinishedPayload,
      getLaunchJobId: () => launch.jobId,
    });
  }

  $effect(() => {
    return setupStartupDiagnosticsEffect({
      isTauriRuntimeAvailable,
      defaultStartupDiagnostics: DEFAULT_STARTUP_DIAGNOSTICS,
      setStartupDiagnostics: (value) => {
        startupDiagnostics = value;
      },
    });
  });

  // Initialize profiling subsystem at startup.
  $effect(() => {
    return setupProfilingEffect({ startupDiagnostics });
  });

  // Poll OS env readiness at startup; stop once ready.
  $effect(() => {
    return setupOsEnvReadyPollingEffect({
      getOsEnvReady: () => appearance.osEnvReady,
      setOsEnvReady: (value) => {
        appearance.osEnvReady = value;
      },
    });
  });

  // Listen for OS env fallback event and show toast.
  $effect(() => {
    return setupOsEnvFallbackListenerEffect({ showToast });
  });

  // Best-effort: request update state once on startup.
  $effect(() => {
    return setupStartupUpdateCheckEffect({
      startupDiagnostics,
      lastUpdateToastVersion: toast.lastUpdateVersion,
      showAvailableUpdateToast,
    });
  });

  // Listen for app update state notifications from backend startup checks.
  $effect(() => {
    return setupAppUpdateStateListenerEffect({
      startupDiagnostics,
      showAvailableUpdateToast,
    });
  });

  // Keep external URL behavior consistent across all rendered links.
  $effect(() => {
    return setupExternalUrlHandlerEffect({
      nearestAnchor,
      shouldHandleExternalLinkClick,
      openExternalUrl,
    });
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
    return setupWindowSessionRestoreEffect({
      startupDiagnostics,
      windowSessionRestoreStarted,
      markWindowSessionRestoreStarted: () => {
        windowSessionRestoreStarted = true;
      },
      releaseDelayMs: 3000,
      pruneWindowSessions,
      resolveCurrentWindowLabel,
      tryAcquireWindowSessionRestoreLead,
      loadWindowSessions,
      deduplicateByProjectPath,
      openAndNormalizeRestoredWindowSession,
      applyRestoredWindowSession,
      releaseWindowSessionRestoreLead,
    });
  });

  // Remove session entry when the window is hidden via the close button.
  $effect(() => {
    return setupWindowWillHideListenerEffect({
      onHide: async () => {
        const label = await resolveCurrentWindowLabel();
        if (label) removeWindowSession(label);
      },
    });
  });

  // Best-effort: subscribe once and refresh Sidebar when worktrees change.
  $effect(() => {
    return setupWorktreesChangedListenerEffect({
      getProjectPath: () => projectPath,
      refreshCanvasWorktrees,
      incrementWorktreeInventoryRefreshKey: () => {
        worktreeInventoryRefreshKey++;
      },
      setWorktreesEventAvailable: (value) => {
        worktreesEventAvailable = value;
      },
    });
  });

  // Best-effort: close agent tabs when the backend closes the pane.
  $effect(() => {
    return setupTerminalClosedListenerEffect({
      removeTabLocal,
      onError: (message, err) => {
        console.error(message, err);
      },
    });
  });

  // Update terminal tab cwd and label when the shell's working directory changes.
  $effect(() => {
    return setupTerminalCwdChangedListenerEffect({
      updateTerminalCwd: (paneId, cwd) => {
        tabs = tabs.map((tab) => {
          if (tab.type === "terminal" && tab.paneId === paneId) {
            return { ...tab, cwd, label: terminalTabLabel(cwd) };
          }
          return tab;
        });
      },
      onError: (message, err) => {
        console.error(message, err);
      },
    });
  });

  // Subscribe to launch-progress events at mount time to avoid race conditions.
  // LaunchProgressModal is a pure display component; all event handling lives here.
  $effect(() => {
    return setupLaunchProgressListenerEffect({
      getLaunchJobId: () => launch.jobId,
      getLaunchJobStartPending: () => launch.jobStartPending,
      applyLaunchProgressPayload,
      bufferLaunchProgressEvent,
      debugLaunchEvent,
      onError: (message, err) => {
        console.error(message, err);
      },
    });
  });

  // Handle progress modal state on launch completion.
  $effect(() => {
    return setupLaunchFinishedListenerEffect({
      getLaunchJobId: () => launch.jobId,
      getLaunchJobStartPending: () => launch.jobStartPending,
      applyLaunchFinishedPayload,
      bufferLaunchFinishedEvent,
      debugLaunchEvent,
      onError: (message, err) => {
        console.error(message, err);
      },
    });
  });

  // Poll backend for launch job results.  Tauri events can be silently
  // lost, so we periodically ask the backend directly.  When the job has
  // finished, the stored result is applied exactly as a launch-finished
  // event would be.
  $effect(() => {
    return setupLaunchPollEffect({
      launchProgressOpen: launch.progressOpen,
      launchJobId: launch.jobId,
      launchStatus: launch.status,
      pollIntervalMs: 1500,
      applyLaunchFinishedPayload,
      setUnexpectedLaunchError: () => {
        launch.status = "error";
        launch.error = "Launch job ended unexpectedly. Please retry.";
      },
    });
  });

  // Close docs editor tabs automatically after vi exits.
  $effect(() => {
    return setupDocsEditorAutoCloseEffect({
      paneIds: launch.docsEditorAutoClosePaneIds,
      pollIntervalMs: DOCS_EDITOR_AUTO_CLOSE_POLL_MS,
      isTerminalProcessEnded,
      removeDocsEditorAutoClosePane,
    });
  });

  function toErrorMessage(err: unknown): string {
    return toErrorMessageRuntime(err);
  }

  function isTauriRuntimeAvailable(): boolean {
    return isTauriRuntimeAvailableRuntime();
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
    return shouldHandleExternalLinkClickRuntime(
      event,
      anchor,
      isAllowedExternalHttpUrl,
    );
  }

  function clampFontSize(size: number): number {
    return clampFontSizeRuntime(size);
  }

  function normalizeVoiceInputSettings(
    value: Partial<VoiceInputSettings> | null | undefined,
  ): VoiceInputSettings {
    return normalizeVoiceInputSettingsAppearance(value);
  }

  function normalizeAppLanguage(
    value: string | null | undefined,
  ): SettingsData["app_language"] {
    return normalizeAppLanguageAppearance(value);
  }

  function normalizeUiFontFamily(value: string | null | undefined): string {
    return normalizeUiFontFamilyRuntime(value);
  }

  function normalizeTerminalFontFamily(
    value: string | null | undefined,
  ): string {
    return normalizeTerminalFontFamilyRuntime(value);
  }

  function applyUiFontSize(size: number) {
    applyUiFontSizeRuntime(size);
  }

  function applyUiFontFamily(family: string | null | undefined) {
    applyUiFontFamilyRuntime(family);
  }

  function applyTerminalFontSize(size: number) {
    applyTerminalFontSizeRuntime(size);
  }

  function applyTerminalFontFamily(family: string | null | undefined) {
    applyTerminalFontFamilyRuntime(family);
  }

  function applyVoiceInputSettings(
    value: Partial<VoiceInputSettings> | null | undefined,
  ) {
    applyVoiceInputSettingsRT(voice, value, { controller: voiceController });
  }

  function applyAppLanguage(value: string | null | undefined) {
    appLanguage = normalizeAppLanguage(value);
  }

  async function checkOsEnvCaptureOnStartup() {
    await checkOsEnvCaptureOnStartupRuntime({
      state: appearance,
      isTauriAvailable: isTauriRuntimeAvailable,
      invokeIsOsEnvReady: async () => {
        const { invoke } = await import("$lib/tauriInvoke");
        return invoke<boolean>("is_os_env_ready");
      },
      onResolved: () => {
        startupOsEnvCaptureResolved = true;
      },
    });
  }

  async function rebuildAllBranchSessionSummaries(
    language: SettingsData["app_language"],
  ) {
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
    return voice.settings;
  }

  function readVoiceFallbackTerminalPaneId(): string | null {
    return activeAgentPaneId();
  }

  async function applyAppearanceSettings() {
    try {
      await applyAppearanceSettingsRuntime({
        state: appearance,
        onLanguageChange: (lang) => {
          appLanguage = lang;
        },
        onVoiceInputChange: (settings) => {
          applyVoiceInputSettings(settings);
        },
        invokeGetSettings: async () => {
          const { invoke } = await import("$lib/tauriInvoke");
          return invoke<SettingsData>("get_settings");
        },
      });
    } catch {
      // Ignore: settings API not available outside Tauri runtime.
    }
  }

  async function resolveCurrentWindowLabel(): Promise<string | null> {
    return resolveCurrentWindowLabelRuntime({
      cachedLabel: currentWindowLabel,
      setCachedLabel: (label) => {
        currentWindowLabel = label;
      },
    });
  }

  async function updateWindowSession(projectPathForWindow: string | null) {
    await updateWindowSessionRuntime({
      projectPathForWindow,
      resolveCurrentWindowLabel,
      upsertWindowSession,
      removeWindowSession,
    });
  }

  async function applyRestoredWindowSession(label: string) {
    const startupToken = startupProfilingTracker.start("restore_session");
    const result = await restoreCurrentWindowSession(label);
    applyRestoredWindowSessionRuntime({
      result,
      handleOpenedProjectPath,
      startupToken,
      discardStartupToken: (token) => startupProfilingTracker.discard(token),
      setMigrationSourceRoot: (value) => {
        migrationSourceRoot = value;
      },
      setMigrationOpen: (value) => {
        migrationOpen = value;
      },
      setAppError: (message) => {
        modal.appError = message;
      },
    });
  }

  function flushStartupMetrics(metrics: StartupFrontendMetric[]) {
    for (const metric of metrics) {
      recordFrontendMetric(metric);
    }
  }

  function handleOpenedProjectPath(
    path: string,
    startupToken: string | null = null,
  ) {
    handleOpenedProjectPathRuntime({
      path,
      startupToken,
      setActiveStartupProfileToken: (token) => {
        activeStartupProfileToken = token;
      },
      setProjectPath: (nextPath) => {
        projectPath = nextPath;
      },
      bumpProjectHydrationToken: () => ++projectHydrationToken,
      fetchCurrentBranch,
      refreshCanvasWorktrees,
      updateWindowSession,
      scheduleIssueCacheWarmup,
    });
  }

  function clearIssueCacheWarmupTimer() {
    if (issueCacheWarmupTimer === null) return;
    clearTimeout(issueCacheWarmupTimer);
    issueCacheWarmupTimer = null;
  }

  function scheduleIssueCacheWarmup(path: string) {
    clearIssueCacheWarmupTimer();
    issueCacheWarmupProjectPath = path;
    issueCacheWarmupTimer = setTimeout(() => {
      issueCacheWarmupTimer = null;
      const targetPath = issueCacheWarmupProjectPath;
      if (!targetPath || projectPath !== targetPath) return;

      void (async () => {
        try {
          const { invoke } = await import("$lib/tauriInvoke");
          await invoke("sync_issue_cache", {
            projectPath: targetPath,
            mode: "diff",
          });
        } catch (error) {
          console.error("Failed to warm issue cache in background:", error);
        }
      })();
    }, ISSUE_CACHE_WARMUP_DELAY_MS);
  }

  async function openProjectAndApplyCurrentWindow(
    path: string,
  ): Promise<OpenProjectResult> {
    return openProjectAndApplyCurrentWindowRuntime({
      path,
      startStartupProfile: () => startupProfilingTracker.start("open_project"),
      discardStartupProfile: (token) => startupProfilingTracker.discard(token),
      invokeOpenProject: async (nextPath) => {
        const { invoke } = await import("$lib/tauriInvoke");
        return invoke<OpenProjectResult>("open_project", { path: nextPath });
      },
      handleOpenedProjectPath,
    });
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

  function handleProjectOpen(path: string, startupToken?: string) {
    handleOpenedProjectPath(path, startupToken ?? null);
  }

  function handleBranchSelect(branch: BranchInfo) {
    selectedBranch = branch;
    if (branch.is_current) {
      currentBranch = branch.name;
    }
  }

  function resolveCanvasWorktreePath(branchName?: string | null): string | null {
    const normalizedBranch = (branchName ?? "").trim();
    if (!normalizedBranch) {
      return selectedCanvasWorktreePath;
    }
    return (
      canvasWorktrees.find((worktree) => worktree.branch === normalizedBranch)?.path ??
      selectedCanvasWorktreePath
    );
  }

  async function focusOrCreateWorktreeFromBranch(branch: BranchInfo) {
    if (!projectPath) return;
    handleBranchSelect(branch);

    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const result = await invoke<MaterializeWorktreeResult>(
        "materialize_worktree_ref",
        {
          projectPath,
          branchRef: branch.name,
        },
      );
      await refreshCanvasWorktrees(projectPath);
      selectedCanvasWorktreeBranch = result.worktree.branch;
      selectedCanvasWorktreePath = result.worktree.path;
      openAgentCanvasTab();
      showToast(
        result.created
          ? `Worktree created: ${result.worktree.branch}`
          : `Worktree focused: ${result.worktree.branch}`,
        5000,
      );
    } catch (err) {
      showToast(`Failed to open worktree: ${toErrorMessage(err)}`, 8000);
    }
  }

  function requestAgentLaunch() {
    showAgentLaunch = true;
  }

  function ensureWorkspaceTab(tab: Tab) {
    if (tabs.some((candidate) => candidate.id === tab.id || candidate.type === tab.type)) {
      return;
    }
    tabs = [...tabs, tab];
  }

  function ensurePrimaryShellTabs() {
    ensureWorkspaceTab({
      id: "agentCanvas",
      label: "Agent Canvas",
      type: "agentCanvas",
    });
    ensureWorkspaceTab({
      id: "branchBrowser",
      label: "Branch Browser",
      type: "branchBrowser",
    });
  }

  function openAgentCanvasTab() {
    ensurePrimaryShellTabs();
    activeTabId = "agentCanvas";
  }

  function openBranchBrowserTab() {
    ensurePrimaryShellTabs();
    activeTabId = "branchBrowser";
  }

  function getSelectedCanvasSessionTab(): Tab | null {
    if (!selectedCanvasSessionTabId) return null;
    return (
      tabs.find(
        (tab) =>
          tab.id === selectedCanvasSessionTabId &&
          (tab.type === "agent" || tab.type === "terminal"),
      ) ?? null
    );
  }

  function isShellTab(tab: Tab): boolean {
    return (
      tab.type === "agentCanvas" ||
      tab.type === "branchBrowser" ||
      tab.type === "settings" ||
      tab.type === "versionHistory" ||
      tab.type === "issues" ||
      tab.type === "prs" ||
      tab.type === "projectIndex" ||
      tab.type === "issueSpec"
    );
  }

  function getShellTabs(): Tab[] {
    return tabs.filter((tab) => isShellTab(tab));
  }

  function getEffectiveWindowMenuActiveTabId(): string | null {
    const active = tabs.find((t) => t.id === activeTabId) ?? null;
    if (active?.type === "agent" || active?.type === "terminal") {
      return active.id;
    }
    if (active?.type === "agentCanvas") {
      return getSelectedCanvasSessionTab()?.id ?? null;
    }
    return null;
  }

  function handleCanvasSessionSelect(tabId: string) {
    const sessionTab = tabs.find(
      (tab) => tab.id === tabId && (tab.type === "agent" || tab.type === "terminal"),
    );
    if (!sessionTab) return;
    selectedCanvasSessionTabId = tabId;
    selectedCanvasWorktreeBranch = sessionTab.branchName ?? selectedCanvasWorktreeBranch;
    selectedCanvasWorktreePath =
      sessionTab.worktreePath ?? resolveCanvasWorktreePath(sessionTab.branchName);
    openAgentCanvasTab();
  }

  function handleBranchActivate(branch: BranchInfo) {
    void focusOrCreateWorktreeFromBranch(branch);
  }

  function handleCleanupRequest(preSelectedBranch?: string) {
    openCleanupModalRuntime(modal, preSelectedBranch);
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
        ...(selectedCanvasWorktreeBranch
          ? { branchName: selectedCanvasWorktreeBranch }
          : {}),
        ...(workingDir ? { worktreePath: workingDir } : {}),
        cwd: workingDir || undefined,
      };
      tabs = [...tabs, newTab];
      selectedCanvasSessionTabId = newTab.id;
      openAgentCanvasTab();

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
      const normalized = logOutput.endsWith("\n")
        ? logOutput
        : `${logOutput}\n`;
      const cmd = `cat <<'${delimiter}'\n${normalized}${delimiter}\n`;
      const data = Array.from(new TextEncoder().encode(cmd));
      await invoke("write_terminal", { paneId, data });
    } catch (err) {
      console.error("Failed to open CI log:", err);
    }
  }

  async function fetchCurrentBranch(
    targetProjectPath = projectPath,
    hydrationToken = projectHydrationToken,
    startupToken: string | null = activeStartupProfileToken,
  ) {
    if (!targetProjectPath) return;
    startupProfilingTracker.beginPhase(startupToken, "fetch_current_branch");
    let success = false;
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const branch = await invoke<BranchInfo | null>("get_current_branch", {
        projectPath: targetProjectPath,
      });
      if (hydrationToken !== projectHydrationToken) return;
      if (branch) {
        currentBranch = branch.name;
        if (!selectedCanvasWorktreeBranch) {
          selectedCanvasWorktreeBranch = branch.name;
        }
        if (!selectedCanvasWorktreePath) {
          selectedCanvasWorktreePath = resolveCanvasWorktreePath(branch.name);
        }
      }
      success = true;
    } catch (err) {
      console.error("Failed to fetch current branch:", err);
      currentBranch = "";
    } finally {
      flushStartupMetrics(
        startupProfilingTracker.finishPhase(
          startupToken,
          "fetch_current_branch",
          success,
        ),
      );
    }
  }

  async function refreshCanvasWorktrees(
    targetProjectPath = projectPath,
    hydrationToken = projectHydrationToken,
    startupToken: string | null = activeStartupProfileToken,
  ) {
    if (!targetProjectPath) return;
    startupProfilingTracker.beginPhase(startupToken, "refresh_canvas_worktrees");
    let success = false;
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const worktrees = await invoke<WorktreeInfo[]>("list_worktrees", {
        projectPath: targetProjectPath,
      });
      if (hydrationToken !== projectHydrationToken) return;
      canvasWorktrees = worktrees;
      if (!selectedCanvasWorktreeBranch) {
        const selectedWorktree =
          worktrees.find((worktree) => worktree.is_current) ?? worktrees[0] ?? null;
        selectedCanvasWorktreeBranch = selectedWorktree?.branch ?? null;
        selectedCanvasWorktreePath = selectedWorktree?.path ?? null;
      } else if (
        !worktrees.some((worktree) => worktree.branch === selectedCanvasWorktreeBranch)
      ) {
        const selectedWorktree =
          worktrees.find((worktree) => worktree.is_current) ?? worktrees[0] ?? null;
        selectedCanvasWorktreeBranch = selectedWorktree?.branch ?? null;
        selectedCanvasWorktreePath = selectedWorktree?.path ?? null;
      } else {
        selectedCanvasWorktreePath = resolveCanvasWorktreePath(selectedCanvasWorktreeBranch);
      }
      success = true;
    } catch (err) {
      console.error("Failed to refresh canvas worktrees:", err);
    } finally {
      flushStartupMetrics(
        startupProfilingTracker.finishPhase(
          startupToken,
          "refresh_canvas_worktrees",
          success,
        ),
      );
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

  function worktreeTabLabel(branch: string): string {
    const branches =
      selectedBranch &&
      normalizeBranchName(selectedBranch.name) === normalizeBranchName(branch)
        ? [selectedBranch]
        : [];
    return resolveWorktreeTabLabel(branch, branches);
  }

  let refreshAgentTabLabelsInflight = false;
  async function refreshAgentTabLabelsForProject(targetProjectPath: string) {
    if (refreshAgentTabLabelsInflight) return;
    refreshAgentTabLabelsInflight = true;
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const branches = await invoke<BranchInfo[]>("list_worktree_branches", {
        projectPath: targetProjectPath,
      });
      if (projectPath !== targetProjectPath) return;
      tabs = syncAgentTabLabels(tabs, branches);
    } catch (err) {
      console.error("Failed to refresh agent tab labels:", err);
    } finally {
      refreshAgentTabLabelsInflight = false;
    }
  }

  function handleBranchDisplayNameChanged() {
    worktreeInventoryRefreshKey++;
    if (!projectPath) return;
    void refreshAgentTabLabelsForProject(projectPath);
  }

  function parseE1004BranchName(errorMessage: string): string | null {
    const match = errorMessage.match(
      /\[E1004\]\s+Branch already exists:\s*(.+)$/m,
    );
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
    return (
      selectedCanvasWorktreePath ||
      resolveCanvasWorktreePath(selectedCanvasWorktreeBranch) ||
      projectPath
    );
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
        ...(selectedCanvasWorktreeBranch
          ? { branchName: selectedCanvasWorktreeBranch }
          : {}),
        ...(workingDir ? { worktreePath: workingDir } : {}),
        cwd: workingDir || undefined,
      };
      tabs = [...tabs, newTab];
      selectedCanvasSessionTabId = newTab.id;
      openAgentCanvasTab();
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
    if (launch.docsEditorAutoClosePaneIds.includes(normalized)) return;
    launch.docsEditorAutoClosePaneIds = [...launch.docsEditorAutoClosePaneIds, normalized];
  }

  function removeDocsEditorAutoClosePane(paneId: string) {
    const normalized = paneId.trim();
    if (!normalized) return;
    if (!launch.docsEditorAutoClosePaneIds.includes(normalized)) return;
    launch.docsEditorAutoClosePaneIds = launch.docsEditorAutoClosePaneIds.filter(
      (id) => id !== normalized,
    );
  }

  async function resolveWindowsDocsShellId(): Promise<DocsEditorShellId> {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const settings =
        await invoke<Pick<SettingsData, "default_shell">>("get_settings");
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
        ...(selectedCanvasWorktreeBranch
          ? { branchName: selectedCanvasWorktreeBranch }
          : {}),
        ...(workingDir ? { worktreePath: workingDir } : {}),
        cwd: workingDir,
      };
      tabs = [...tabs, tab];
      selectedCanvasSessionTabId = tab.id;
      openAgentCanvasTab();

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

  function triggerRestoreProjectAgentTabs(
    targetProjectPath: string,
    startupToken: string | null = activeStartupProfileToken,
  ) {
    const token = ++agentTabsRestoreToken;
    void restoreProjectAgentTabs(targetProjectPath, token, startupToken);
  }

  async function handleAgentLaunch(request: LaunchAgentRequest) {
    // Reset progress state before starting the job.
    launch.step = "fetch";
    launch.detail = "";
    launch.status = "running";
    launch.error = null;
    launch.jobStartPending = true;
    launch.bufferedProgressEvents = [];
    launch.bufferedFinishedEvents = [];

    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const jobId = await invoke<string>("start_launch_job", { request });

      launch.pendingRequest = request;
      launch.jobId = jobId;
      launch.jobStartPending = false;
      launch.progressOpen = true;
      flushBufferedLaunchEventsForActiveJob();
    } catch (err) {
      clearBufferedLaunchEvents();
      throw err;
    }
  }

  async function handleLaunchCancel() {
    if (!launch.jobId) {
      handleLaunchModalClose();
      return;
    }
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      await invoke("cancel_launch_job", { jobId: launch.jobId });
      handleLaunchModalClose();
    } catch (err) {
      console.error("Failed to cancel launch job:", err);
      launch.status = "error";
      launch.error =
        "Failed to send cancel request. Close this dialog and retry.";
    }
  }

  function handleLaunchModalClose() {
    closeLaunchModalRuntime({
      clearBufferedLaunchEvents,
      setLaunchProgressOpen: (open) => {
        launch.progressOpen = open;
      },
      setLaunchJobId: (jobId) => {
        launch.jobId = jobId;
      },
      setPendingLaunchRequest: (request) => {
        launch.pendingRequest = request;
      },
      setLaunchStatus: (status) => {
        launch.status = status;
      },
      setLaunchStep: (step) => {
        launch.step = step;
      },
      setLaunchDetail: (detail) => {
        launch.detail = detail;
      },
      setLaunchError: (error) => {
        launch.error = error;
      },
    });
  }

  function handleUseExistingBranch() {
    const retryRequest = buildUseExistingBranchRetryRequest(launch.pendingRequest);
    if (!retryRequest) return;
    handleLaunchModalClose();
    void handleAgentLaunch(retryRequest);
  }

  function handleLaunchSuccess(paneId: string) {
    const req = launch.pendingRequest;
    const requestedBranch = req?.branch?.trim() ?? "";
    const label = req ? worktreeTabLabel(requestedBranch) : "Worktree";
    const requestedAgentId = inferAgentId(req?.agentId);

    const newTab: Tab = {
      id: `agent-${paneId}`,
      label,
      type: "agent",
      paneId,
      ...(requestedBranch ? { branchName: requestedBranch } : {}),
      ...(resolveCanvasWorktreePath(requestedBranch)
        ? { worktreePath: resolveCanvasWorktreePath(requestedBranch) ?? undefined }
        : {}),
    };

    if (requestedAgentId) {
      newTab.agentId = requestedAgentId;
    }

    tabs = [...tabs, newTab];
    selectedCanvasSessionTabId = newTab.id;
    openAgentCanvasTab();
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
          const resolvedBranch = terminal.branch_name?.trim() ?? "";
          if (resolvedBranch) {
            updates.branchName = resolvedBranch;
            const resolvedWorktreePath = resolveCanvasWorktreePath(resolvedBranch);
            if (resolvedWorktreePath) {
              updates.worktreePath = resolvedWorktreePath;
            }
            if (needsBranchResolution) {
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
          if (projectPath) {
            await refreshAgentTabLabelsForProject(projectPath);
          }
        } catch {
          // Ignore: fallback color is used when terminal metadata is unavailable.
        }
      })();
    }

    // Fallback: if the event API is not available, trigger a best-effort refresh.
    if (!worktreesEventAvailable) {
      worktreeInventoryRefreshKey++;
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
    if (selectedCanvasSessionTabId === tabId) {
      const fallbackSession =
        nextTabs.find((t) => t.type === "agent" || t.type === "terminal") ?? null;
      selectedCanvasSessionTabId = fallbackSession?.id ?? null;
      selectedCanvasWorktreeBranch = fallbackSession?.branchName ?? selectedCanvasWorktreeBranch;
      selectedCanvasWorktreePath =
        fallbackSession?.worktreePath ??
        resolveCanvasWorktreePath(fallbackSession?.branchName);
    }

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

  function handleTabSelect(groupId: string, tabId: string) {
    void groupId;
    activeTabId = tabId;
    if (tabId === "agentCanvas" || tabId === "branchBrowser") {
      return;
    }
    const selected = tabs.find((tab) => tab.id === tabId);
    if (selected?.type === "agent" || selected?.type === "terminal") {
      selectedCanvasSessionTabId = tabId;
      selectedCanvasWorktreeBranch = selected.branchName ?? selectedCanvasWorktreeBranch;
      selectedCanvasWorktreePath =
        selected.worktreePath ?? resolveCanvasWorktreePath(selected.branchName);
      activeTabId = "agentCanvas";
    }
  }

  function handleTabReorder(
    _groupId: string,
    dragTabId: string,
    overTabId: string,
    position: TabDropPosition,
  ) {
    const nextTabs = reorderTabsByDrop(tabs, dragTabId, overTabId, position);
    if (nextTabs !== tabs) {
      tabs = nextTabs;
    }
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
    const existing = tabs.find((t) => t.type === "issues" || t.id === "issues");
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
    const existing = tabs.find((t) => t.type === "prs" || t.id === "prs");
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
      t.id === "issues"
        ? { ...t, label: count > 0 ? `Issues (${count})` : "Issues" }
        : t,
    );
  }

  function handleWorkOnIssueFromTab(issue: GitHubIssueInfo) {
    prefillIssue = issue;
    showAgentLaunch = true;
  }

  function handleSwitchToWorktreeFromTab(branchName: string) {
    // Find the matching agent tab and switch to it
    const agentTab = findAgentTabByBranchName(tabs, branchName);
    if (agentTab) {
      selectedCanvasSessionTabId = agentTab.id;
      selectedCanvasWorktreeBranch = branchName;
      selectedCanvasWorktreePath = resolveCanvasWorktreePath(branchName);
      openAgentCanvasTab();
      return;
    }
    // If no session exists yet, move the user to Branch Browser and refresh its source view.
    openBranchBrowserTab();
    worktreeInventoryRefreshKey++;
  }

  function openIssueSpecTab(payload: ProjectModeSpecIssuePayload) {
    const issueNumber = Number(payload.issueNumber);
    if (!Number.isFinite(issueNumber) || issueNumber <= 0) return;
    const label = `Issue #${issueNumber}`;

    const existing = tabs.find(
      (t) => t.type === "issueSpec" || t.id === "issueSpec",
    );
    if (existing) {
      tabs = tabs.map((t) =>
        t.id === existing.id
          ? {
              ...t,
              label,
              issueNumber,
            }
          : t,
      );
      activeTabId = existing.id;
      return;
    }

    const tab: Tab = {
      id: "issueSpec",
      label,
      type: "issueSpec",
      issueNumber,
    };
    tabs = [...tabs, tab];
    activeTabId = tab.id;
  }

  function getActiveTerminalPaneId(): string | null {
    return getActiveTerminalPaneIdRuntime({
      tabs,
      activeTabId,
      selectedCanvasSessionTabId,
    });
  }

  let copyFlashActive = $state(false);

  async function handleScreenCopy() {
    const activeTab = tabs.find((t) => t.id === activeTabId);
    const selectedCanvasSession =
      activeTab?.type === "agentCanvas" ? getSelectedCanvasSessionTab() : null;
    const effectiveActiveTab = selectedCanvasSession ?? activeTab ?? null;
    const text = collectScreenText({
      branch: currentBranch,
      activeTab: effectiveActiveTab?.label ?? activeTabId,
      activeTabType: effectiveActiveTab?.type,
      activePaneId:
        effectiveActiveTab?.type === "agent" || effectiveActiveTab?.type === "terminal"
          ? effectiveActiveTab.paneId
          : undefined,
    });
    try {
      await navigator.clipboard.writeText(text);
      copyFlashActive = true;
      setTimeout(() => {
        copyFlashActive = false;
      }, 300);
      showToast("Copied to clipboard", 2000);
    } catch {
      showToast("Failed to copy screen text", 4000);
    }
  }

  function emitTerminalEditAction(action: "copy" | "paste") {
    const editableEl = document.activeElement;
    if (editableEl && !editableEl.closest("[data-pane-id]")) {
      void fallbackMenuEditActionRuntime(action);
      return;
    }

    const paneId = getActiveTerminalPaneId();
    if (!paneId) {
      void fallbackMenuEditActionRuntime(action);
      return;
    }

    if (typeof window === "undefined") return;

    window.dispatchEvent(
      new CustomEvent("gwt-terminal-edit-action", {
        detail: { action, paneId },
      }),
    );
  }

  async function syncWindowAgentTabsSnapshot() {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const visibleTabs = buildWindowMenuVisibleTabs(tabs);
      const tabsSignature = buildWindowMenuTabsSignature(visibleTabs);
      if (tabsSignature === lastWindowMenuTabsSignature) {
        return;
      }
      const requestedActiveTabId = getEffectiveWindowMenuActiveTabId();
      const activeVisibleTabId =
        requestedActiveTabId === null
          ? null
          : resolveActiveWindowMenuTabId(visibleTabs, requestedActiveTabId);
      await invoke("sync_window_agent_tabs", {
        request: {
          tabs: visibleTabs,
          activeTabId: activeVisibleTabId,
        },
      });
      lastWindowMenuTabsSignature = tabsSignature;
      if (
        shouldKeepSnapshotActiveTabCache(
          activeVisibleTabId,
          tabs,
          requestedActiveTabId ?? activeTabId,
        )
      ) {
        lastWindowMenuActiveTabId = activeVisibleTabId;
      }
    } catch {
      // Ignore: not available outside Tauri runtime.
    }
  }

  async function syncWindowActiveTabOnly() {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const visibleTabs = buildWindowMenuVisibleTabs(tabs);
      const requestedActiveTabId = getEffectiveWindowMenuActiveTabId();
      const activeVisibleTabId =
        requestedActiveTabId === null
          ? null
          : resolveActiveWindowMenuTabId(visibleTabs, requestedActiveTabId);
      if (activeVisibleTabId === lastWindowMenuActiveTabId) {
        return;
      }
      await invoke("sync_window_active_tab", {
        activeTabId: activeVisibleTabId,
      });
      lastWindowMenuActiveTabId = activeVisibleTabId;
    } catch {
      // Ignore: not available outside Tauri runtime.
    }
  }

  async function handleMenuAction(action: string) {
    await runAppMenuAction(action, {
      focusAgentTab: (tabId) => {
        void focusAgentTabInCanvas({
          tabId,
          tabs,
          setSelectedCanvasSessionTabId: (value) => {
            selectedCanvasSessionTabId = value;
          },
          setActiveTabId: (value) => {
            activeTabId = value;
          },
        });
      },
      openRecentProjectPath: async (recentPath) =>
        openRecentProjectPath({
          recentPath,
          openProjectAndApplyCurrentWindow,
          setMigrationSourceRoot: (value) => {
            migrationSourceRoot = value;
          },
          setMigrationOpen: (value) => {
            migrationOpen = value;
          },
          setAppError: (message) => {
            modal.appError = message;
          },
          toErrorMessage,
        }),
      openProjectViaDialog: async () =>
        openProjectViaDialog({
          openProjectAndApplyCurrentWindow,
          setMigrationSourceRoot: (value) => {
            migrationSourceRoot = value;
          },
          setMigrationOpen: (value) => {
            migrationOpen = value;
          },
          setAppError: (message) => {
            modal.appError = message;
          },
          toErrorMessage,
        }),
      closeProject: async () =>
        closeProjectAndTerminals({
          tabs,
          updateWindowSession,
          resetProjectState: () => {
            projectPath = null;
            tabs = defaultAppTabs();
            activeTabId = "agentCanvas";
            selectedCanvasSessionTabId = null;
            selectedCanvasWorktreeBranch = null;
            selectedCanvasWorktreePath = null;
            canvasWorktrees = [];
            selectedBranch = null;
            currentBranch = "";
          },
        }),
      toggleSidebar: () => {
        openBranchBrowserTab();
      },
      launchAgent: () => {
        if (projectPath) {
          showAgentLaunch = true;
        }
      },
      newTerminal: async () => {
        if (projectPath) {
          await handleNewTerminal();
        }
      },
      cleanupWorktrees: () => {
        if (projectPath) {
          openCleanupModalRuntime(modal);
        }
      },
      openSettings: () => {
        openSettingsTab();
      },
      openVersionHistory: () => {
        openVersionHistoryTab();
      },
      openIssues: () => {
        openIssuesTab();
      },
      openPullRequests: () => {
        openPullRequestsTab();
      },
      openProjectIndex: () => {
        openProjectIndexTab();
      },
      checkUpdates: async () =>
        checkForUpdates({
          showToast,
          showAvailableUpdateToast,
          toErrorMessage,
        }),
      openAbout: () => {
        openAboutDialogRuntime(modal);
      },
      reportIssue: () => {
        showReportDialog("bug");
      },
      suggestFeature: () => {
        showReportDialog("feature");
      },
      listTerminals: () =>
        listTerminalSessions({
          tabs,
          setSelectedCanvasSessionTabId: (value) => {
            selectedCanvasSessionTabId = value;
          },
          openAgentCanvasTab,
        }),
      editCopy: () => {
        emitTerminalEditAction("copy");
      },
      editPaste: () => {
        emitTerminalEditAction("paste");
      },
      screenCopy: () => {
        void handleScreenCopy();
      },
      debugOsEnv: () => {
        void openOsEnvDebug({
          setShowOsEnvDebug: (value) => {
            modal.osEnvDebug.open = value;
          },
          setOsEnvDebugLoading: (value) => {
            modal.osEnvDebug.loading = value;
          },
          setOsEnvDebugError: (value) => {
            modal.osEnvDebug.error = value;
          },
          setOsEnvDebugData: (value) => {
            modal.osEnvDebug.data = value;
          },
        });
      },
      terminalDiagnostics: async () =>
        openTerminalDiagnostics({
          activePaneId:
            getSelectedCanvasSessionTab()?.paneId ??
            (tabs.find((t) => t.id === activeTabId)?.paneId ?? null),
          setAppError: (message) => {
            modal.appError = message;
          },
          setShowTerminalDiagnostics: (value) => {
            modal.terminalDiagnostics.open = value;
          },
          setTerminalDiagnosticsLoading: (value) => {
            modal.terminalDiagnostics.loading = value;
          },
          setTerminalDiagnosticsError: (value) => {
            modal.terminalDiagnostics.error = value;
          },
          setTerminalDiagnostics: (value) => {
            modal.terminalDiagnostics.data = value;
          },
          toErrorMessage,
        }),
      previousTab: () => {
        const prevId = getPreviousTabId(getShellTabs(), activeTabId);
        if (prevId) activeTabId = prevId;
      },
      nextTab: () => {
        const nextId = getNextTabId(getShellTabs(), activeTabId);
        if (nextId) activeTabId = nextId;
      },
    });
  }

  $effect(() => {
    void tabs;
    void syncWindowAgentTabsSnapshot();
  });

  $effect(() => {
    void tabs;
    void activeTabId;
    void syncWindowActiveTabOnly();
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
    startupToken: string | null = activeStartupProfileToken,
    attempt = 0,
  ) {
    await restoreProjectTabsRuntime({
      targetProjectPath,
      token,
      attempt,
      startupToken,
      startupDiagnostics,
      currentProjectPath: projectPath,
      currentRestoreToken: agentTabsRestoreToken,
      activeTabId,
      existingTabs: tabs,
      defaultBranchBrowserState: DEFAULT_BRANCH_BROWSER_STATE,
      resolveCurrentWindowLabel,
      terminalTabLabel,
      resolveCanvasWorktreePath,
      refreshAgentTabLabelsForProject,
      beginPhase: (tokenValue, phase) => {
        startupProfilingTracker.beginPhase(tokenValue, phase);
      },
      finishPhase: (tokenValue, phase, success) =>
        startupProfilingTracker.finishPhase(tokenValue, phase, success),
      flushStartupMetrics,
      onRetry: (nextAttempt, delayMs) => {
        setTimeout(() => {
          void restoreProjectAgentTabs(
            targetProjectPath,
            token,
            startupToken,
            nextAttempt,
          );
        }, delayMs);
      },
      baseDelayMs: AGENT_TAB_RESTORE_RETRY_DELAY_MS,
      maxDelayMs: AGENT_TAB_RESTORE_RETRY_MAX_DELAY_MS,
      maxRetries: AGENT_TAB_RESTORE_MAX_RETRIES,
      onHydrated: (path) => {
        agentTabsHydratedProjectPath = path;
      },
      setTabs: (value) => {
        tabs = value;
      },
      setCanvasViewport: (value) => {
        canvasViewport = value;
      },
      setCanvasTileLayouts: (value) => {
        canvasTileLayouts = value as typeof canvasTileLayouts;
      },
      setSelectedCanvasTileId: (value) => {
        selectedCanvasTileId = value;
      },
      setBranchBrowserState: (value) => {
        branchBrowserState = value;
      },
      setSelectedCanvasSessionTabId: (value) => {
        selectedCanvasSessionTabId = value;
      },
      setSelectedCanvasWorktreeBranch: (value) => {
        selectedCanvasWorktreeBranch = value;
      },
      setSelectedCanvasWorktreePath: (value) => {
        selectedCanvasWorktreePath = value;
      },
      setActiveTabId: (value) => {
        activeTabId = value;
      },
      onError: (message, err) => {
        console.error(message, err);
      },
    });
  }

  // Restore persisted tabs when a project is opened.
  $effect(() => {
    void projectPath;

    if (!projectPath) {
      agentTabsHydratedProjectPath = null;
      return;
    }

    agentTabsHydratedProjectPath = null;
    canvasViewport = createDefaultAgentCanvasViewport();
    canvasTileLayouts = {};
    selectedCanvasTileId = null;
    branchBrowserState = DEFAULT_BRANCH_BROWSER_STATE;
    const target = projectPath;
    triggerRestoreProjectAgentTabs(target, activeStartupProfileToken);
  });

  $effect(() => {
    const target = projectPath;
    if (!target) return;
    const hydrationToken = projectHydrationToken;
    void fetchCurrentBranch(target, hydrationToken);
    void refreshCanvasWorktrees(target, hydrationToken);
  });

  // Persist tabs per project (best-effort).
  $effect(() => {
    void projectPath;
    void tabs;
    void activeTabId;
    void agentTabsHydratedProjectPath;

    if (!projectPath) return;
    if (agentTabsHydratedProjectPath !== projectPath) return;

    persistStoredProjectTabs(
      projectPath,
      buildStoredProjectTabsSnapshot({
        tabs,
        activeTabId,
        selectedCanvasSessionTabId,
        canvasViewport,
        canvasTileLayouts,
        selectedCanvasTileId,
        branchBrowserState,
      }),
      undefined,
      currentWindowLabel,
    );
  });

  // Native menubar integration (Tauri emits "menu-action" to the focused window).
  $effect(() => {
    return setupMenuActionListenerEffect({
      isTauriRuntimeAvailable,
      handleMenuAction,
      setAppError: (message) => {
        modal.appError = message;
      },
      toErrorMessage,
    });
  });

  // System monitor lifecycle: start polling, destroy on teardown.
  $effect(() => {
    systemMonitor.start();
    return () => {
      systemMonitor.destroy();
    };
  });

  $effect(() => {
    const result = setupVoiceController({
      getSettings: readVoiceInputSettingsForController,
      getFallbackTerminalPaneId: readVoiceFallbackTerminalPaneId,
      onStateChange: (cs: VoiceControllerState) => {
        syncControllerState(voice, cs);
      },
    });
    voiceController = result.controller;

    return () => {
      result.cleanup();
      if (voiceController === result.controller) {
        voiceController = null;
      }
      resetVoiceInputTransientState(voice);
    };
  });

  $effect(() => {
    function onSettingsUpdated(event: Event) {
      const detail = (event as CustomEvent<SettingsUpdatedPayload>).detail;
      if (!detail) return;
      if (typeof detail.uiFontSize === "number") {
        applyUiFontSizeRuntime(clampFontSizeAppearance(detail.uiFontSize));
      }
      if (typeof detail.terminalFontSize === "number") {
        applyTerminalFontSizeRuntime(clampFontSizeAppearance(detail.terminalFontSize));
      }
      if (typeof detail.uiFontFamily === "string") {
        applyUiFontFamilyRuntime(detail.uiFontFamily);
      }
      if (typeof detail.terminalFontFamily === "string") {
        applyTerminalFontFamilyRuntime(detail.terminalFontFamily);
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
      const payload = (event as CustomEvent<ProjectModeSpecIssuePayload>)
        .detail;
      if (!payload) return;
      openIssueSpecTab(payload);
    }
    window.addEventListener(
      "gwt-project-mode-open-spec-issue",
      onOpenIssueSpec,
    );
    return () =>
      window.removeEventListener(
        "gwt-project-mode-open-spec-issue",
        onOpenIssueSpec,
      );
  });

  $effect(() => {
    if (!import.meta.env.DEV) return;

    const hooks = createAppE2EHooks({
      getTabs: () => tabs,
      getActiveTabId: () => activeTabId,
      getProjectPath: () => projectPath,
      getToastMessage: () => toastMessage,
      isReportDialogOpen: () => modal.reportDialog.open,
      resetCurrentWindowLabelCache: () => {
        currentWindowLabel = null;
      },
      getDocsEditorAutoClosePaneIds: () => launch.docsEditorAutoClosePaneIds,
      forceActiveTabId: (tabId) => {
        activeTabId = tabId;
      },
      seedTab: (tab) => {
        tabs = [...tabs, tab];
      },
      reorderTabs: (dragTabId, overTabId, position) =>
        handleTabReorder("main", dragTabId, overTabId, position),
      toErrorMessage,
      clampFontSize,
      normalizeVoiceInputSettings,
      normalizeAppLanguage,
      normalizeUiFontFamily,
      normalizeTerminalFontFamily,
      parseE1004BranchName,
      terminalTabLabel,
      agentTabLabel,
      getAgentTabRestoreDelayMs: (attempt) =>
        getAgentTabRestoreDelayMs(
          attempt,
          AGENT_TAB_RESTORE_RETRY_DELAY_MS,
          AGENT_TAB_RESTORE_RETRY_MAX_DELAY_MS,
        ),
      readVoiceFallbackTerminalPaneId,
      persistAgentPasteHintDismissed,
      isTauriRuntimeAvailable,
      getActiveAgentPaneId: () => activeAgentPaneId(),
      showAvailableUpdateToast,
      showToastForE2E: showToast,
      handleToastClick,
      showReportDialog,
      applyAppearanceSettings,
      resolveCurrentWindowLabel,
      updateWindowSession,
      shouldHandleExternalLinkClick,
      openIssuesTab,
      setIssueCount: handleIssueCountChange,
      openPullRequestsTab,
      openSettingsTab,
      openBranchBrowserTab,
      openProjectIndexTab,
      openVersionHistoryTab,
      openIssueSpecTab: (issueNumber) =>
        openIssueSpecTab({ issueNumber, issueUrl: null }),
      restoreProjectTabs: triggerRestoreProjectAgentTabs,
      applyWindowSession: applyRestoredWindowSession,
      openProjectPath: handleProjectOpen,
      activateTab: (tabId) => handleTabSelect("main", tabId),
      closeTab: handleTabClose,
      requestCleanup: handleCleanupRequest,
      activateBranch: focusOrCreateWorktreeFromBranch,
      cancelLaunch: handleLaunchCancel,
      requestAgentLaunch,
      workOnIssue: handleWorkOnIssueFromTab,
      switchToWorktree: handleSwitchToWorktreeFromTab,
      selectCanvasSession: handleCanvasSessionSelect,
      openDocsEditor: handleOpenDocsEditor,
      openCiLog: handleOpenCiLog,
      branchDisplayNameChanged: handleBranchDisplayNameChanged,
      dismissAppError: () => {
        modal.appError = null;
      },
      dismissOsEnvDebug: () => {
        modal.osEnvDebug.open = false;
      },
      dismissTerminalDiagnostics: () => {
        modal.terminalDiagnostics.open = false;
      },
      dismissAbout: () => {
        modal.about.open = false;
      },
      clearToast: () => {
        toastMessage = null;
        toastAction = null;
      },
    });

    (
      window as unknown as {
        __GWT_E2E_APP__?: typeof hooks;
      }
    ).__GWT_E2E_APP__ = hooks;

    return () => {
      const globalWindow = window as unknown as {
        __GWT_E2E_APP__?: typeof hooks;
      };
      if (globalWindow.__GWT_E2E_APP__ === hooks) {
        delete globalWindow.__GWT_E2E_APP__;
      }
    };
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
      <MainArea
        {tabs}
        {activeTabId}
        projectPath={projectPath as string}
        {branchBrowserConfig}
        {currentBranch}
        {selectedCanvasSessionTabId}
        {selectedCanvasTileId}
        {canvasViewport}
        {canvasTileLayouts}
        disableSplit={true}
        branchBrowserState={branchBrowserState}
        onCanvasSessionSelect={handleCanvasSessionSelect}
        onCanvasViewportChange={(next) => {
          canvasViewport = next;
        }}
        onCanvasTileLayoutsChange={(next) => {
          canvasTileLayouts = next;
        }}
        onCanvasSelectedTileChange={(next) => {
          selectedCanvasTileId = next;
        }}
        onLaunchAgent={requestAgentLaunch}
        onQuickLaunch={handleAgentLaunch}
        onTabSelect={handleTabSelect}
        onTabClose={handleTabClose}
        onTabReorder={handleTabReorder}
        onWorkOnIssue={handleWorkOnIssueFromTab}
        onSwitchToWorktree={handleSwitchToWorktreeFromTab}
        onIssueCountChange={handleIssueCountChange}
        onOpenSettings={openSettingsTab}
        voiceInputEnabled={voice.settings.enabled}
        voiceInputListening={voice.listening}
        voiceInputPreparing={voice.preparing}
        voiceInputSupported={voice.supported}
        voiceInputAvailable={voice.available}
        voiceInputAvailabilityReason={voice.availabilityReason}
        voiceInputError={voice.error}
        canvasWorktrees={canvasWorktrees}
        selectedCanvasWorktreeBranch={selectedCanvasWorktreeBranch}
        onCanvasWorktreeSelect={(branchName) => {
          selectedCanvasWorktreeBranch = branchName;
          selectedCanvasWorktreePath = resolveCanvasWorktreePath(branchName);
        }}
      />
    </div>
    <StatusBar
      {projectPath}
      {currentBranch}
      {terminalCount}
      osEnvReady={appearance.osEnvReady}
      voiceInputEnabled={voice.settings.enabled}
      voiceInputListening={voice.listening}
      voiceInputPreparing={voice.preparing}
      voiceInputSupported={voice.supported}
      voiceInputAvailable={voice.available}
      voiceInputAvailabilityReason={voice.availabilityReason}
      voiceInputError={voice.error}
    />
  </div>
{/if}

{#if showAgentLaunch}
  <AgentLaunchForm
    projectPath={projectPath as string}
    selectedBranch={selectedBranch?.name ?? currentBranch}
    osEnvReady={appearance.osEnvReady}
    {prefillIssue}
    onLaunch={handleAgentLaunch}
    onClose={() => {
      showAgentLaunch = false;
      prefillIssue = null;
    }}
  />
{/if}

<QuitConfirmToast />

<CleanupModal
  open={modal.cleanup.open}
  preselectedBranch={modal.cleanup.preselectedBranch}
  refreshKey={worktreeInventoryRefreshKey}
  projectPath={projectPath ?? ""}
  {agentTabBranches}
  onClose={() => (modal.cleanup.open = false)}
/>

<AboutDialog
  open={modal.about.open}
  initialTab={modal.about.initialTab}
  cpuUsage={systemMonitor.cpuUsage}
  memUsed={systemMonitor.memUsed}
  memTotal={systemMonitor.memTotal}
  gpuInfos={systemMonitor.gpuInfos}
  onclose={() => (modal.about.open = false)}
/>

<TerminalDiagnosticsDialog
  open={modal.terminalDiagnostics.open}
  loading={modal.terminalDiagnostics.loading}
  error={modal.terminalDiagnostics.error}
  diagnostics={modal.terminalDiagnostics.data}
  onclose={() => (modal.terminalDiagnostics.open = false)}
/>

<MigrationModal
  open={migrationOpen}
  sourceRoot={migrationSourceRoot}
  onCompleted={async (p) => {
    migrationOpen = false;
    migrationSourceRoot = "";

    try {
      await openProjectAndApplyCurrentWindow(p);
    } catch (err) {
      modal.appError = `Failed to open migrated project: ${toErrorMessage(err)}`;
    }
  }}
  onDismiss={() => {
    migrationOpen = false;
    migrationSourceRoot = "";
  }}
/>

<LaunchProgressModal
  open={launch.progressOpen}
  step={launch.step}
  detail={launch.detail}
  status={launch.status}
  error={launch.error}
  onCancel={handleLaunchCancel}
  onClose={handleLaunchModalClose}
  onUseExisting={handleUseExistingBranch}
/>
<CapturedEnvironmentDialog
  open={modal.osEnvDebug.open}
  loading={modal.osEnvDebug.loading}
  error={modal.osEnvDebug.error}
  data={modal.osEnvDebug.data}
  onclose={() => (modal.osEnvDebug.open = false)}
/>

<AppErrorDialog
  message={modal.appError}
  onclose={() => (modal.appError = null)}
/>

{#if copyFlashActive}
  <div class="copy-flash"></div>
{/if}

<ToastBanner
  message={toastMessage}
  action={toastAction}
  onapply={handleToastClick}
  onreport={(error) => showReportDialog("bug", error)}
  onclose={() => {
    toastMessage = null;
    toastAction = null;
  }}
/>

<ReportDialog
  open={modal.reportDialog.open}
  mode={modal.reportDialog.mode}
  prefillError={modal.reportDialog.prefillError}
  projectPath={projectPath ?? ""}
  screenCaptureBranch={currentBranch}
  screenCaptureActiveTab={tabs.find((t) => t.id === activeTabId)?.label ??
    activeTabId}
  onclose={() => {
    modal.reportDialog.open = false;
  }}
  onsuccess={(result: { url: string; number: number }) => {
    modal.reportDialog.open = false;
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
    0% {
      opacity: 0;
    }
    30% {
      opacity: 0.12;
    }
    100% {
      opacity: 0;
    }
  }
</style>
