import type { StructuredError } from "$lib/errorBus";
import type { TabDropPosition } from "./appTabs";
import type {
  BranchInfo,
  GitHubIssueInfo,
  SettingsData,
  Tab,
  UpdateState,
  VoiceInputSettings,
} from "./types";

type ToastAction =
  | { kind: "apply-update"; latest: string }
  | { kind: "report-error"; error: StructuredError }
  | null;

type AvailableUpdateState = Extract<UpdateState, { state: "available" }>;

export function createAppE2EHooks(args: {
  getTabs: () => Tab[];
  getActiveTabId: () => string;
  getProjectPath: () => string | null;
  getToastMessage: () => string | null;
  isReportDialogOpen: () => boolean;
  resetCurrentWindowLabelCache: () => void;
  getDocsEditorAutoClosePaneIds: () => string[];
  forceActiveTabId: (tabId: string) => void;
  seedTab: (tab: Tab) => void;
  reorderTabs: (
    dragTabId: string,
    overTabId: string,
    position: TabDropPosition,
  ) => void;
  toErrorMessage: (value: unknown) => string;
  clampFontSize: (size: number) => number;
  normalizeVoiceInputSettings: (
    value: Partial<VoiceInputSettings> | null | undefined,
  ) => VoiceInputSettings;
  normalizeAppLanguage: (
    value: string | null | undefined,
  ) => SettingsData["app_language"];
  normalizeUiFontFamily: (value: string | null | undefined) => string;
  normalizeTerminalFontFamily: (value: string | null | undefined) => string;
  parseE1004BranchName: (message: string) => string | null;
  terminalTabLabel: (pathLike: string | null | undefined, fallback?: string) => string;
  agentTabLabel: (agentId: string) => string;
  getAgentTabRestoreDelayMs: (attempt: number) => number;
  readVoiceFallbackTerminalPaneId: () => string | null;
  persistAgentPasteHintDismissed: () => void;
  isTauriRuntimeAvailable: () => boolean;
  getActiveAgentPaneId: () => string | null;
  showAvailableUpdateToast: (
    state: AvailableUpdateState,
    force?: boolean,
  ) => void;
  showToastForE2E: (
    message: string,
    durationMs?: number,
    action?: ToastAction,
  ) => void;
  handleToastClick: () => Promise<void>;
  showReportDialog: (
    mode: "bug" | "feature",
    prefillError?: StructuredError,
  ) => void;
  applyAppearanceSettings: () => Promise<void>;
  resolveCurrentWindowLabel: () => Promise<string | null>;
  updateWindowSession: (projectPathForWindow: string | null) => Promise<void>;
  shouldHandleExternalLinkClick: (
    event: MouseEvent,
    anchor: HTMLAnchorElement,
  ) => boolean;
  openIssuesTab: () => void;
  setIssueCount: (count: number) => void;
  openPullRequestsTab: () => void;
  openSettingsTab: () => void;
  openBranchBrowserTab: () => void;
  openProjectIndexTab: () => void;
  openVersionHistoryTab: () => void;
  openIssueSpecTab: (issueNumber: number) => void;
  restoreProjectTabs: (targetProjectPath: string) => void;
  applyWindowSession: (label: string) => Promise<void>;
  openProjectPath: (path: string) => void;
  activateTab: (tabId: string) => void;
  closeTab: (tabId: string) => Promise<void>;
  requestCleanup: (branchName?: string) => void;
  activateBranch: (branch: BranchInfo) => Promise<void> | void;
  cancelLaunch: () => Promise<void>;
  requestAgentLaunch: () => void;
  workOnIssue: (issue: GitHubIssueInfo) => void;
  switchToWorktree: (branchName: string) => void;
  selectCanvasSession: (tabId: string) => void;
  openDocsEditor: (worktreePath: string) => Promise<void>;
  openCiLog: (runId: number) => Promise<void>;
  branchDisplayNameChanged: () => void;
  dismissAppError: () => void;
  dismissOsEnvDebug: () => void;
  dismissTerminalDiagnostics: () => void;
  dismissAbout: () => void;
  clearToast: () => void;
}) {
  return {
    getTabs: () => args.getTabs().map((tab) => ({ ...tab })),
    getActiveTabId: () => args.getActiveTabId(),
    getProjectPath: () => args.getProjectPath(),
    getToastMessage: () => args.getToastMessage(),
    isReportDialogOpen: () => args.isReportDialogOpen(),
    resetCurrentWindowLabelCache: () => args.resetCurrentWindowLabelCache(),
    getDocsEditorAutoClosePaneIds: () => [...args.getDocsEditorAutoClosePaneIds()],
    forceActiveTabId: (tabId: string) => args.forceActiveTabId(tabId),
    seedTab: (tab: Tab) => args.seedTab(tab),
    reorderTabs: (
      dragTabId: string,
      overTabId: string,
      position: TabDropPosition,
    ) => args.reorderTabs(dragTabId, overTabId, position),
    toErrorMessage: (value: unknown) => args.toErrorMessage(value),
    clampFontSize: (size: number) => args.clampFontSize(size),
    normalizeVoiceInputSettings: (
      value: Partial<VoiceInputSettings> | null | undefined,
    ) => args.normalizeVoiceInputSettings(value),
    normalizeAppLanguage: (value: string | null | undefined) =>
      args.normalizeAppLanguage(value),
    normalizeUiFontFamily: (value: string | null | undefined) =>
      args.normalizeUiFontFamily(value),
    normalizeTerminalFontFamily: (value: string | null | undefined) =>
      args.normalizeTerminalFontFamily(value),
    parseE1004BranchName: (message: string) =>
      args.parseE1004BranchName(message),
    terminalTabLabel: (pathLike: string | null | undefined, fallback?: string) =>
      args.terminalTabLabel(pathLike, fallback),
    agentTabLabel: (agentId: string) => args.agentTabLabel(agentId),
    getAgentTabRestoreDelayMs: (attempt: number) =>
      args.getAgentTabRestoreDelayMs(attempt),
    readVoiceFallbackTerminalPaneId: () => args.readVoiceFallbackTerminalPaneId(),
    persistAgentPasteHintDismissed: () => args.persistAgentPasteHintDismissed(),
    isTauriRuntimeAvailable: () => args.isTauriRuntimeAvailable(),
    getActiveAgentPaneId: () => args.getActiveAgentPaneId(),
    showAvailableUpdateToast: (
      state: AvailableUpdateState,
      force?: boolean,
    ) => args.showAvailableUpdateToast(state, force),
    showToastForE2E: (
      message: string,
      durationMs?: number,
      action?: ToastAction,
    ) => args.showToastForE2E(message, durationMs, action),
    handleToastClick: async () => args.handleToastClick(),
    showReportDialog: (
      mode: "bug" | "feature",
      prefillError?: StructuredError,
    ) => args.showReportDialog(mode, prefillError),
    applyAppearanceSettings: async () => args.applyAppearanceSettings(),
    resolveCurrentWindowLabel: async () => args.resolveCurrentWindowLabel(),
    updateWindowSession: async (projectPathForWindow: string | null) =>
      args.updateWindowSession(projectPathForWindow),
    shouldHandleExternalLinkClickSample: (
      href: string,
      options?: {
        button?: number;
        metaKey?: boolean;
        ctrlKey?: boolean;
        shiftKey?: boolean;
        altKey?: boolean;
        defaultPrevented?: boolean;
        download?: boolean;
      },
    ) => {
      const anchor = document.createElement("a");
      anchor.setAttribute("href", href);
      if (options?.download) {
        anchor.setAttribute("download", "sample.txt");
      }
      const event = new MouseEvent("click", {
        button: options?.button ?? 0,
        metaKey: options?.metaKey ?? false,
        ctrlKey: options?.ctrlKey ?? false,
        shiftKey: options?.shiftKey ?? false,
        altKey: options?.altKey ?? false,
      });
      if (options?.defaultPrevented) {
        event.preventDefault();
      }
      return args.shouldHandleExternalLinkClick(event, anchor);
    },
    openIssuesTab: () => args.openIssuesTab(),
    setIssueCount: (count: number) => args.setIssueCount(count),
    openPullRequestsTab: () => args.openPullRequestsTab(),
    openSettingsTab: () => args.openSettingsTab(),
    openBranchBrowserTab: () => args.openBranchBrowserTab(),
    openProjectIndexTab: () => args.openProjectIndexTab(),
    openVersionHistoryTab: () => args.openVersionHistoryTab(),
    openIssueSpecTab: (issueNumber: number) => args.openIssueSpecTab(issueNumber),
    restoreProjectTabs: (targetProjectPath: string) =>
      args.restoreProjectTabs(targetProjectPath),
    applyWindowSession: async (label: string) => args.applyWindowSession(label),
    openProjectPath: (path: string) => args.openProjectPath(path),
    activateTab: (tabId: string) => args.activateTab(tabId),
    closeTab: async (tabId: string) => args.closeTab(tabId),
    requestCleanup: (branchName?: string) => args.requestCleanup(branchName),
    activateBranch: async (branch: BranchInfo) => args.activateBranch(branch),
    cancelLaunch: async () => args.cancelLaunch(),
    requestAgentLaunch: () => args.requestAgentLaunch(),
    workOnIssue: (issue: GitHubIssueInfo) => args.workOnIssue(issue),
    switchToWorktree: (branchName: string) => args.switchToWorktree(branchName),
    selectCanvasSession: (tabId: string) => args.selectCanvasSession(tabId),
    openDocsEditor: async (worktreePath: string) => args.openDocsEditor(worktreePath),
    openCiLog: async (runId: number) => args.openCiLog(runId),
    branchDisplayNameChanged: () => args.branchDisplayNameChanged(),
    dismissAppError: () => args.dismissAppError(),
    dismissOsEnvDebug: () => args.dismissOsEnvDebug(),
    dismissTerminalDiagnostics: () => args.dismissTerminalDiagnostics(),
    dismissAbout: () => args.dismissAbout(),
    clearToast: () => args.clearToast(),
  };
}
