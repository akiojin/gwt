import type { StructuredError } from "./errorBus";
import type { TerminalAnsiProbe, CapturedEnvInfo } from "./types";

export interface ModalState {
  about: { open: boolean; initialTab: "general" | "system" | "statistics" };
  cleanup: { open: boolean; preselectedBranch: string | null };
  reportDialog: {
    open: boolean;
    mode: "bug" | "feature";
    prefillError?: StructuredError;
  };
  terminalDiagnostics: {
    open: boolean;
    loading: boolean;
    data: TerminalAnsiProbe | null;
    error: string | null;
  };
  osEnvDebug: {
    open: boolean;
    data: CapturedEnvInfo | null;
    loading: boolean;
    error: string | null;
  };
  appError: string | null;
}

export function createModalState(): ModalState {
  return {
    about: { open: false, initialTab: "general" },
    cleanup: { open: false, preselectedBranch: null },
    reportDialog: { open: false, mode: "bug", prefillError: undefined },
    terminalDiagnostics: {
      open: false,
      loading: false,
      data: null,
      error: null,
    },
    osEnvDebug: { open: false, data: null, loading: false, error: null },
    appError: null,
  };
}

export function showReportDialogRuntime(
  state: ModalState,
  mode: "bug" | "feature",
  prefillError?: StructuredError,
): void {
  state.reportDialog.mode = mode;
  state.reportDialog.prefillError = prefillError;
  state.reportDialog.open = true;
}

export function openAboutDialogRuntime(
  state: ModalState,
  tab?: "general" | "system" | "statistics",
): void {
  state.about.initialTab = tab ?? "general";
  state.about.open = true;
}

export function openCleanupModalRuntime(
  state: ModalState,
  branch?: string | null,
): void {
  state.cleanup.preselectedBranch = branch ?? null;
  state.cleanup.open = true;
}
