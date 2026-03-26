import type {
  CapturedEnvInfo,
  ProbePathResult,
  Tab,
  TerminalAnsiProbe,
  UpdateState,
} from "./types";

export async function focusAgentTabInCanvas(args: {
  tabId: string;
  tabs: Tab[];
  setSelectedCanvasSessionTabId: (tabId: string) => void;
  setActiveTabId: (tabId: string) => void;
}) {
  if (
    args.tabs.some(
      (tab) =>
        tab.id === args.tabId && (tab.type === "agent" || tab.type === "terminal"),
    )
  ) {
    args.setSelectedCanvasSessionTabId(args.tabId);
    args.setActiveTabId("agentCanvas");
  }
}

export async function openRecentProjectPath(args: {
  recentPath: string;
  openProjectAndApplyCurrentWindow: (projectPath: string) => Promise<unknown>;
  setMigrationSourceRoot: (value: string) => void;
  setMigrationOpen: (value: boolean) => void;
  setAppError: (message: string) => void;
  toErrorMessage: (err: unknown) => string;
}) {
  try {
    const { invoke } = await import("$lib/tauriInvoke");
    const probe = await invoke<ProbePathResult>("probe_path", {
      path: args.recentPath,
    });
    if (probe.kind === "gwtProject" && probe.projectPath) {
      await args.openProjectAndApplyCurrentWindow(probe.projectPath);
      return;
    }
    if (probe.kind === "migrationRequired" && probe.migrationSourceRoot) {
      args.setMigrationSourceRoot(probe.migrationSourceRoot);
      args.setMigrationOpen(true);
      return;
    }
    args.setAppError(probe.message || "Failed to open recent project.");
  } catch (err) {
    args.setAppError(`Failed to open project: ${args.toErrorMessage(err)}`);
  }
}

export async function openProjectViaDialog(args: {
  openProjectAndApplyCurrentWindow: (projectPath: string) => Promise<unknown>;
  setMigrationSourceRoot: (value: string) => void;
  setMigrationOpen: (value: boolean) => void;
  setAppError: (message: string) => void;
  toErrorMessage: (err: unknown) => string;
}) {
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({ directory: true, multiple: false });
    if (!selected) return;
    const { invoke } = await import("$lib/tauriInvoke");
    const probe = await invoke<ProbePathResult>("probe_path", {
      path: selected as string,
    });
    if (probe.kind === "gwtProject" && probe.projectPath) {
      await args.openProjectAndApplyCurrentWindow(probe.projectPath);
      return;
    }
    if (probe.kind === "migrationRequired" && probe.migrationSourceRoot) {
      args.setMigrationSourceRoot(probe.migrationSourceRoot);
      args.setMigrationOpen(true);
      return;
    }
    if (probe.kind === "emptyDir") {
      args.setAppError(
        "Selected folder is empty. Use New Project on the start screen.",
      );
      return;
    }
    args.setAppError(
      probe.message ||
        (probe.kind === "notFound"
          ? "Path does not exist."
          : probe.kind === "invalid"
            ? "Invalid path."
            : "Not a gwt project."),
    );
  } catch (err) {
    args.setAppError(`Failed to open project: ${args.toErrorMessage(err)}`);
  }
}

export async function closeProjectAndTerminals(args: {
  tabs: Tab[];
  updateWindowSession: (projectPathForWindow: string | null) => Promise<void>;
  resetProjectState: () => void;
}) {
  const terminalPanes = args.tabs
    .filter((tab) => tab.type === "terminal" && tab.paneId)
    .map((tab) => tab.paneId as string);
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
  try {
    const { invoke } = await import("$lib/tauriInvoke");
    await invoke("close_project");
  } catch {
    // Ignore: not available outside Tauri runtime.
  }
  await args.updateWindowSession(null);
  args.resetProjectState();
}

export async function checkForUpdates(args: {
  showToast: (message: string) => void;
  showAvailableUpdateToast: (
    state: Extract<UpdateState, { state: "available" }>,
    force?: boolean,
  ) => void;
  toErrorMessage: (err: unknown) => string;
}) {
  try {
    const { invoke } = await import("$lib/tauriInvoke");
    const state = await invoke<UpdateState>("check_app_update", {
      force: true,
    });
    switch (state.state) {
      case "up_to_date":
        args.showToast("Up to date.");
        break;
      case "available":
        args.showAvailableUpdateToast(state, true);
        break;
      case "failed":
        args.showToast(`Update check failed: ${state.message}`);
        break;
    }
  } catch (err) {
    args.showToast(`Update check failed: ${args.toErrorMessage(err)}`);
  }
}

export function listTerminalSessions(args: {
  tabs: Tab[];
  setSelectedCanvasSessionTabId: (tabId: string) => void;
  openAgentCanvasTab: () => void;
}) {
  const firstAgent = args.tabs.find(
    (tab) => tab.type === "agent" || tab.type === "terminal",
  );
  if (firstAgent) {
    args.setSelectedCanvasSessionTabId(firstAgent.id);
    args.openAgentCanvasTab();
  }
}

export async function openOsEnvDebug(args: {
  setShowOsEnvDebug: (value: boolean) => void;
  setOsEnvDebugLoading: (value: boolean) => void;
  setOsEnvDebugError: (value: string | null) => void;
  setOsEnvDebugData: (value: CapturedEnvInfo | null) => void;
}) {
  args.setShowOsEnvDebug(true);
  args.setOsEnvDebugLoading(true);
  args.setOsEnvDebugError(null);
  try {
    const { invoke } = await import("$lib/tauriInvoke");
    const data = await invoke<CapturedEnvInfo>("get_captured_environment");
    args.setOsEnvDebugData(data);
  } catch (err) {
    args.setOsEnvDebugError(String(err));
  } finally {
    args.setOsEnvDebugLoading(false);
  }
}

export async function openTerminalDiagnostics(args: {
  activePaneId: string | null;
  setAppError: (message: string) => void;
  setShowTerminalDiagnostics: (value: boolean) => void;
  setTerminalDiagnosticsLoading: (value: boolean) => void;
  setTerminalDiagnosticsError: (value: string | null) => void;
  setTerminalDiagnostics: (value: TerminalAnsiProbe | null) => void;
  toErrorMessage: (err: unknown) => string;
}) {
  const paneId = args.activePaneId ?? "";
  if (!paneId) {
    args.setAppError("No active terminal tab.");
    return;
  }
  args.setShowTerminalDiagnostics(true);
  args.setTerminalDiagnosticsLoading(true);
  args.setTerminalDiagnosticsError(null);
  args.setTerminalDiagnostics(null);
  try {
    const { invoke } = await import("$lib/tauriInvoke");
    const diagnostics = await invoke<TerminalAnsiProbe>("probe_terminal_ansi", {
      paneId,
    });
    args.setTerminalDiagnostics(diagnostics);
  } catch (err) {
    args.setTerminalDiagnosticsError(
      `Failed to probe terminal: ${args.toErrorMessage(err)}`,
    );
  } finally {
    args.setTerminalDiagnosticsLoading(false);
  }
}
