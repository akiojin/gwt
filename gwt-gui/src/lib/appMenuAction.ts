export type AppMenuActionHandlers = {
  focusAgentTab: (tabId: string) => void;
  openRecentProjectPath: (path: string) => Promise<void>;
  openProjectViaDialog: () => Promise<void>;
  closeProject: () => Promise<void>;
  toggleSidebar: () => void;
  launchAgent: () => void;
  newTerminal: () => Promise<void>;
  cleanupWorktrees: () => void;
  openSettings: () => void;
  openVersionHistory: () => void;
  openIssues: () => void;
  openPullRequests: () => void;
  openProjectIndex: () => void;
  checkUpdates: () => Promise<void>;
  openAbout: () => void;
  reportIssue: () => void;
  suggestFeature: () => void;
  listTerminals: () => void;
  editCopy: () => void;
  editPaste: () => void;
  screenCopy: () => void;
  debugOsEnv: () => void;
  terminalDiagnostics: () => Promise<void>;
  previousTab: () => void;
  nextTab: () => void;
};

export async function runAppMenuAction(
  action: string,
  handlers: AppMenuActionHandlers,
): Promise<void> {
  if (action.startsWith("focus-agent-tab::")) {
    const tabId = action.slice("focus-agent-tab::".length).trim();
    if (tabId) {
      handlers.focusAgentTab(tabId);
    }
    return;
  }

  if (action.startsWith("open-recent-project::")) {
    const recentPath = action.slice("open-recent-project::".length);
    if (recentPath) {
      await handlers.openRecentProjectPath(recentPath);
    }
    return;
  }

  switch (action) {
    case "open-project":
      await handlers.openProjectViaDialog();
      break;
    case "close-project":
      await handlers.closeProject();
      break;
    case "toggle-sidebar":
      handlers.toggleSidebar();
      break;
    case "launch-agent":
      handlers.launchAgent();
      break;
    case "new-terminal":
      await handlers.newTerminal();
      break;
    case "cleanup-worktrees":
      handlers.cleanupWorktrees();
      break;
    case "open-settings":
      handlers.openSettings();
      break;
    case "version-history":
      handlers.openVersionHistory();
      break;
    case "git-issues":
      handlers.openIssues();
      break;
    case "git-pull-requests":
      handlers.openPullRequests();
      break;
    case "project-index":
      handlers.openProjectIndex();
      break;
    case "check-updates":
      await handlers.checkUpdates();
      break;
    case "about":
      handlers.openAbout();
      break;
    case "report-issue":
      handlers.reportIssue();
      break;
    case "suggest-feature":
      handlers.suggestFeature();
      break;
    case "list-terminals":
      handlers.listTerminals();
      break;
    case "edit-copy":
      handlers.editCopy();
      break;
    case "edit-paste":
      handlers.editPaste();
      break;
    case "screen-copy":
      handlers.screenCopy();
      break;
    case "debug-os-env":
      handlers.debugOsEnv();
      break;
    case "terminal-diagnostics":
      await handlers.terminalDiagnostics();
      break;
    case "previous-tab":
      handlers.previousTab();
      break;
    case "next-tab":
      handlers.nextTab();
      break;
  }
}
