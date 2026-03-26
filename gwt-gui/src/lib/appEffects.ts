import type {
  LaunchFinishedPayload,
  LaunchProgressPayload,
  TerminalInfo,
  UpdateState,
} from "./types";
import {
  STARTUP_UPDATE_INITIAL_DELAY_MS,
  STARTUP_UPDATE_MAX_RETRIES,
  STARTUP_UPDATE_RETRY_DELAY_MS,
  runStartupUpdateCheck,
} from "./update/startupUpdate";

type Cleanup = () => void;

export function setupStartupDiagnosticsEffect<TStartupDiagnostics>(args: {
  isTauriRuntimeAvailable: () => boolean;
  defaultStartupDiagnostics: TStartupDiagnostics;
  setStartupDiagnostics: (value: TStartupDiagnostics) => void;
}): Cleanup {
  let cancelled = false;
  (async () => {
    if (!args.isTauriRuntimeAvailable()) {
      args.setStartupDiagnostics(args.defaultStartupDiagnostics);
      return;
    }
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const diagnostics = await invoke<TStartupDiagnostics>("get_startup_diagnostics");
      if (!cancelled) {
        args.setStartupDiagnostics(diagnostics);
      }
    } catch {
      if (!cancelled) {
        args.setStartupDiagnostics(args.defaultStartupDiagnostics);
      }
    }
  })();
  return () => {
    cancelled = true;
  };
}

export function setupProfilingEffect(args: {
  startupDiagnostics: { disableProfiling: boolean } | null;
}): Cleanup {
  if (args.startupDiagnostics === null) return () => {};
  if (args.startupDiagnostics.disableProfiling) return () => {};
  let cancelled = false;
  (async () => {
    try {
      const { initProfiling } = await import("$lib/profiling.svelte");
      if (!cancelled) await initProfiling();
    } catch {
      // profiling init failure is non-fatal
    }
  })();
  return () => {
    cancelled = true;
    import("$lib/profiling.svelte")
      .then(({ teardownProfiling }) => teardownProfiling())
      .catch(() => {});
  };
}

export function setupOsEnvReadyPollingEffect(args: {
  getOsEnvReady: () => boolean;
  setOsEnvReady: (value: boolean) => void;
}): Cleanup {
  if (args.getOsEnvReady()) return () => {};
  let cancelled = false;
  const poll = async () => {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      while (!cancelled && !args.getOsEnvReady()) {
        const ready = await invoke<boolean>("is_os_env_ready");
        if (ready) {
          args.setOsEnvReady(true);
          return;
        }
        await new Promise((resolve) => setTimeout(resolve, 200));
      }
    } catch {
      // ignore
    }
  };
  void poll();
  return () => {
    cancelled = true;
  };
}

export function setupOsEnvFallbackListenerEffect(args: {
  showToast: (message: string) => void;
}): Cleanup {
  let unlisten: null | (() => void) = null;
  let cancelled = false;
  (async () => {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const unlistenFn = await listen<string>("os-env-fallback", (event) => {
        args.showToast(
          `Shell environment not loaded: ${event.payload}. Using process environment.`,
        );
      });
      if (cancelled) {
        unlistenFn();
        return;
      }
      unlisten = unlistenFn;
    } catch {
      // ignore
    }
  })();
  return () => {
    cancelled = true;
    if (unlisten) unlisten();
  };
}

export function setupStartupUpdateCheckEffect(args: {
  startupDiagnostics: { disableStartupUpdateCheck: boolean } | null;
  lastUpdateToastVersion: string | null;
  showAvailableUpdateToast: (state: Extract<UpdateState, { state: "available" }>) => void;
}): Cleanup {
  if (args.startupDiagnostics === null) return () => {};
  if (args.startupDiagnostics.disableStartupUpdateCheck) return () => {};
  if (args.lastUpdateToastVersion !== null) return () => {};
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
    onAvailable: (state) => {
      args.showAvailableUpdateToast(state);
    },
  });
  return () => {
    controller.abort();
  };
}

export function setupAppUpdateStateListenerEffect(args: {
  startupDiagnostics: { disableStartupUpdateCheck: boolean } | null;
  showAvailableUpdateToast: (state: Extract<UpdateState, { state: "available" }>) => void;
}): Cleanup {
  if (args.startupDiagnostics === null) return () => {};
  if (args.startupDiagnostics.disableStartupUpdateCheck) return () => {};
  let unlisten: null | (() => void) = null;
  let cancelled = false;
  (async () => {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const unlistenFn = await listen<UpdateState>("app-update-state", (event) => {
        const state = event.payload;
        if (state.state !== "available") return;
        args.showAvailableUpdateToast(state);
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
  return () => {
    cancelled = true;
    if (unlisten) unlisten();
  };
}

export function setupExternalUrlHandlerEffect(args: {
  nearestAnchor: (target: EventTarget | null) => HTMLAnchorElement | null;
  shouldHandleExternalLinkClick: (
    event: MouseEvent,
    anchor: HTMLAnchorElement,
  ) => boolean;
  openExternalUrl: (url: string) => Promise<unknown>;
}): Cleanup {
  if (typeof document === "undefined") return () => {};
  const handleDocumentClick = (event: MouseEvent) => {
    const anchor = args.nearestAnchor(event.target);
    if (!anchor) return;
    if (!args.shouldHandleExternalLinkClick(event, anchor)) return;
    const rawHref = anchor.getAttribute("href");
    if (!rawHref) return;
    event.preventDefault();
    void args.openExternalUrl(rawHref);
  };
  document.addEventListener("click", handleDocumentClick, true);
  return () => {
    document.removeEventListener("click", handleDocumentClick, true);
  };
}

export function setupWorktreesChangedListenerEffect(args: {
  getProjectPath: () => string | null;
  refreshCanvasWorktrees: (projectPath: string) => Promise<void>;
  incrementWorktreeInventoryRefreshKey: () => void;
  setWorktreesEventAvailable: (value: boolean) => void;
}): Cleanup {
  let unlisten: null | (() => void) = null;
  let cancelled = false;
  (async () => {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const unlistenFn = await listen<unknown>("worktrees-changed", (event) => {
        const projectPath = args.getProjectPath();
        if (!projectPath) return;
        const payload = (event as { payload?: unknown }).payload;
        if (payload && typeof payload === "object" && "project_path" in payload) {
          const raw = (payload as { project_path?: unknown }).project_path;
          if (typeof raw === "string" && raw && raw !== projectPath) return;
        }
        args.incrementWorktreeInventoryRefreshKey();
        void args.refreshCanvasWorktrees(projectPath);
      });
      if (cancelled) {
        unlistenFn();
        return;
      }
      unlisten = unlistenFn;
      args.setWorktreesEventAvailable(true);
    } catch {
      args.setWorktreesEventAvailable(false);
    }
  })();
  return () => {
    cancelled = true;
    if (unlisten) unlisten();
  };
}

export function setupWindowWillHideListenerEffect(args: {
  onHide: () => Promise<void>;
}): Cleanup {
  let unlisten: null | (() => void) = null;
  let cancelled = false;
  (async () => {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const unlistenFn = await listen("window-will-hide", async () => {
        await args.onHide();
      });
      if (cancelled) {
        unlistenFn();
        return;
      }
      unlisten = unlistenFn;
    } catch {
      // not in Tauri runtime
    }
  })();
  return () => {
    cancelled = true;
    if (unlisten) unlisten();
  };
}

export function setupTerminalClosedListenerEffect(args: {
  removeTabLocal: (tabId: string) => void;
  onError: (message: string, err: unknown) => void;
}): Cleanup {
  let unlisten: null | (() => void) = null;
  let cancelled = false;
  (async () => {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const unlistenFn = await listen<{ pane_id: string }>("terminal-closed", (event) => {
        args.removeTabLocal(`agent-${event.payload.pane_id}`);
        args.removeTabLocal(`terminal-${event.payload.pane_id}`);
      });
      if (cancelled) {
        unlistenFn();
        return;
      }
      unlisten = unlistenFn;
    } catch (err) {
      args.onError("Failed to setup terminal closed listener:", err);
    }
  })();
  return () => {
    cancelled = true;
    if (unlisten) unlisten();
  };
}

export function setupTerminalCwdChangedListenerEffect(args: {
  updateTerminalCwd: (paneId: string, cwd: string) => void;
  onError: (message: string, err: unknown) => void;
}): Cleanup {
  let unlisten: null | (() => void) = null;
  let cancelled = false;
  (async () => {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const unlistenFn = await listen<{ pane_id: string; cwd: string }>(
        "terminal-cwd-changed",
        (event) => {
          args.updateTerminalCwd(event.payload.pane_id, event.payload.cwd);
        },
      );
      if (cancelled) {
        unlistenFn();
        return;
      }
      unlisten = unlistenFn;
    } catch (err) {
      args.onError("Failed to setup terminal-cwd-changed listener:", err);
    }
  })();
  return () => {
    cancelled = true;
    if (unlisten) unlisten();
  };
}

export function setupLaunchProgressListenerEffect(args: {
  getLaunchJobId: () => string;
  getLaunchJobStartPending: () => boolean;
  applyLaunchProgressPayload: (payload: LaunchProgressPayload) => void;
  bufferLaunchProgressEvent: (payload: LaunchProgressPayload) => void;
  debugLaunchEvent: (message: string, payload: unknown) => void;
  onError: (message: string, err: unknown) => void;
}): Cleanup {
  let unlisten: null | (() => void) = null;
  let cancelled = false;
  (async () => {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const unlistenFn = await listen<LaunchProgressPayload>("launch-progress", (event) => {
        const payload = event.payload;
        const launchJobId = args.getLaunchJobId();
        if (launchJobId) {
          if (payload.jobId !== launchJobId) {
            args.debugLaunchEvent("Ignored launch-progress for different job", payload);
            return;
          }
          args.applyLaunchProgressPayload(payload);
          return;
        }
        if (args.getLaunchJobStartPending()) {
          args.bufferLaunchProgressEvent(payload);
          args.debugLaunchEvent(
            "Buffered launch-progress before jobId assignment",
            payload,
          );
          return;
        }
        args.debugLaunchEvent("Ignored launch-progress without active job", payload);
      });
      if (cancelled) {
        unlistenFn();
        return;
      }
      unlisten = unlistenFn;
    } catch (err) {
      args.onError("Failed to setup launch progress listener:", err);
    }
  })();
  return () => {
    cancelled = true;
    if (unlisten) unlisten();
  };
}

export function setupLaunchFinishedListenerEffect(args: {
  getLaunchJobId: () => string;
  getLaunchJobStartPending: () => boolean;
  applyLaunchFinishedPayload: (payload: LaunchFinishedPayload) => void;
  bufferLaunchFinishedEvent: (payload: LaunchFinishedPayload) => void;
  debugLaunchEvent: (message: string, payload: unknown) => void;
  onError: (message: string, err: unknown) => void;
}): Cleanup {
  let unlisten: null | (() => void) = null;
  let cancelled = false;
  (async () => {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const unlistenFn = await listen<LaunchFinishedPayload>("launch-finished", (event) => {
        const payload = event.payload;
        const launchJobId = args.getLaunchJobId();
        if (launchJobId) {
          if (payload.jobId !== launchJobId) {
            args.debugLaunchEvent("Ignored launch-finished for different job", payload);
            return;
          }
          args.applyLaunchFinishedPayload(payload);
          return;
        }
        if (args.getLaunchJobStartPending()) {
          args.bufferLaunchFinishedEvent(payload);
          args.debugLaunchEvent(
            "Buffered launch-finished before jobId assignment",
            payload,
          );
          return;
        }
        args.debugLaunchEvent("Ignored launch-finished without active job", payload);
      });
      if (cancelled) {
        unlistenFn();
        return;
      }
      unlisten = unlistenFn;
    } catch (err) {
      args.onError("Failed to setup launch-finished listener:", err);
    }
  })();
  return () => {
    cancelled = true;
    if (unlisten) unlisten();
  };
}

export function setupLaunchPollEffect(args: {
  launchProgressOpen: boolean;
  launchJobId: string;
  launchStatus: "running" | "ok" | "error" | "cancelled";
  pollIntervalMs: number;
  applyLaunchFinishedPayload: (payload: LaunchFinishedPayload) => void;
  setUnexpectedLaunchError: () => void;
}): Cleanup {
  if (!args.launchProgressOpen || !args.launchJobId || args.launchStatus !== "running") {
    return () => {};
  }
  const jobId = args.launchJobId;
  const timer = window.setInterval(async () => {
    if (args.launchJobId !== jobId || args.launchStatus !== "running") return;
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const result = await invoke<{
        running: boolean;
        finished: LaunchFinishedPayload | null;
      }>("poll_launch_job", { jobId });

      if (result.running) return;
      if (args.launchJobId !== jobId || args.launchStatus !== "running") return;

      if (result.finished) {
        args.applyLaunchFinishedPayload(result.finished);
      } else {
        args.setUnexpectedLaunchError();
      }
    } catch {
      // ignore polling errors
    }
  }, args.pollIntervalMs);
  return () => window.clearInterval(timer);
}

export function setupDocsEditorAutoCloseEffect(args: {
  paneIds: string[];
  pollIntervalMs: number;
  isTerminalProcessEnded: (status: string) => boolean;
  removeDocsEditorAutoClosePane: (paneId: string) => void;
}): Cleanup {
  if (args.paneIds.length === 0) return () => {};
  let polling = false;
  const timer = window.setInterval(() => {
    if (polling) return;
    polling = true;
    void (async () => {
      const paneIds = [...args.paneIds];
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
            args.removeDocsEditorAutoClosePane(paneId);
            continue;
          }
          if (!args.isTerminalProcessEnded(status)) continue;
          try {
            await invoke("close_terminal", { paneId });
          } catch {
            // Ignore if already closed.
          }
          args.removeDocsEditorAutoClosePane(paneId);
        }
      } catch {
        // Ignore polling errors.
      }
    })().finally(() => {
      polling = false;
    });
  }, args.pollIntervalMs);
  return () => window.clearInterval(timer);
}

export function setupWindowSessionRestoreEffect(args: {
  startupDiagnostics: { disableWindowSessionRestore: boolean } | null;
  windowSessionRestoreStarted: boolean;
  markWindowSessionRestoreStarted: () => void;
  releaseDelayMs: number;
  pruneWindowSessions: () => void;
  resolveCurrentWindowLabel: () => Promise<string | null>;
  tryAcquireWindowSessionRestoreLead: (label: string) => Promise<boolean>;
  loadWindowSessions: () => Array<{ label: string; projectPath: string }>;
  deduplicateByProjectPath: (
    sessions: Array<{ label: string; projectPath: string }>,
  ) => Array<{ label: string; projectPath: string }>;
  openAndNormalizeRestoredWindowSession: (label: string) => Promise<unknown>;
  applyRestoredWindowSession: (label: string) => Promise<unknown>;
  releaseWindowSessionRestoreLead: (label: string) => Promise<unknown>;
}): Cleanup {
  if (args.startupDiagnostics === null) return () => {};
  if (args.startupDiagnostics.disableWindowSessionRestore) return () => {};
  if (args.windowSessionRestoreStarted) return () => {};
  args.markWindowSessionRestoreStarted();
  void (async () => {
    args.pruneWindowSessions();
    const label = await args.resolveCurrentWindowLabel();
    if (!label) return;
    const isRestoreLeader = await args.tryAcquireWindowSessionRestoreLead(label);
    const sessions = args.loadWindowSessions();
    const normalizedSessions = args.deduplicateByProjectPath(
      sessions.filter((entry) => entry.label !== label && entry.projectPath),
    );
    if (isRestoreLeader) {
      try {
        for (const entry of normalizedSessions) {
          await args.openAndNormalizeRestoredWindowSession(entry.label);
        }
        await new Promise((resolve) => setTimeout(resolve, args.releaseDelayMs));
        await args.applyRestoredWindowSession(label);
      } finally {
        await args.releaseWindowSessionRestoreLead(label);
      }
    } else {
      await args.applyRestoredWindowSession(label);
    }
  })();
  return () => {};
}

export function setupMenuActionListenerEffect(args: {
  isTauriRuntimeAvailable: () => boolean;
  handleMenuAction: (action: string) => Promise<void>;
  setAppError: (message: string) => void;
  toErrorMessage: (err: unknown) => string;
}): Cleanup {
  let unlisten: null | (() => void) = null;
  let cancelled = false;
  (async () => {
    if (!args.isTauriRuntimeAvailable()) {
      return;
    }
    try {
      const { setupMenuActionListener } = await import("./menuAction");
      const unlistenFn = await setupMenuActionListener((action) => {
        void args.handleMenuAction(action);
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
      args.setAppError(`Menu integration failed: ${args.toErrorMessage(err)}`);
    }
  })();
  return () => {
    cancelled = true;
    if (unlisten) {
      unlisten();
    }
  };
}
