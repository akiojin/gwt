import type { OpenProjectResult } from "./types";

export async function resolveCurrentWindowLabelRuntime(args: {
  cachedLabel: string | null;
  setCachedLabel: (label: string | null) => void;
}): Promise<string | null> {
  if (args.cachedLabel) return args.cachedLabel;

  try {
    const { invoke } = await import("$lib/tauriInvoke");
    const label = await invoke<string>("get_current_window_label");
    const next = label?.trim();
    if (!next) return null;
    args.setCachedLabel(next);
    return next;
  } catch {
    return null;
  }
}

export async function updateWindowSessionRuntime(args: {
  projectPathForWindow: string | null;
  resolveCurrentWindowLabel: () => Promise<string | null>;
  upsertWindowSession: (label: string, projectPath: string) => void;
  removeWindowSession: (label: string) => void;
}): Promise<void> {
  const label = await args.resolveCurrentWindowLabel();
  if (!label) return;

  if (args.projectPathForWindow) {
    args.upsertWindowSession(label, args.projectPathForWindow);
    return;
  }
  args.removeWindowSession(label);
}

export function applyRestoredWindowSessionRuntime(args: {
  result: Awaited<ReturnType<typeof import("./windowSessionRestore").restoreCurrentWindowSession>>;
  handleOpenedProjectPath: (path: string, startupToken: string | null) => void;
  startupToken: string;
  discardStartupToken: (token: string) => void;
  setMigrationSourceRoot: (sourceRoot: string) => void;
  setMigrationOpen: (open: boolean) => void;
  setAppError: (message: string) => void;
}): void {
  if (args.result.kind === "opened") {
    args.handleOpenedProjectPath(args.result.result.info.path, args.startupToken);
    return;
  }
  args.discardStartupToken(args.startupToken);
  if (args.result.kind === "migrationRequired") {
    args.setMigrationSourceRoot(args.result.sourceRoot);
    args.setMigrationOpen(true);
    return;
  }
  if (args.result.kind === "focusedExisting") {
    args.setAppError(
      args.result.focusedWindowLabel
        ? `Project is already open in window ${args.result.focusedWindowLabel}.`
        : "Project is already open in another window.",
    );
    return;
  }
  if (args.result.kind === "error") {
    args.setAppError(`Failed to restore project: ${args.result.message}`);
  }
}

export function handleOpenedProjectPathRuntime(args: {
  path: string;
  startupToken: string | null;
  setActiveStartupProfileToken: (token: string | null) => void;
  setProjectPath: (path: string) => void;
  bumpProjectHydrationToken: () => number;
  fetchCurrentBranch: (
    path: string,
    hydrationToken: number,
    startupToken?: string | null,
  ) => Promise<void>;
  refreshCanvasWorktrees: (
    path: string,
    hydrationToken: number,
    startupToken?: string | null,
  ) => Promise<void>;
  updateWindowSession: (projectPathForWindow: string | null) => Promise<void>;
  scheduleIssueCacheWarmup: (path: string) => void;
}): void {
  args.setActiveStartupProfileToken(args.startupToken);
  args.setProjectPath(args.path);
  const hydrationToken = args.bumpProjectHydrationToken();
  void args.fetchCurrentBranch(args.path, hydrationToken, args.startupToken);
  void args.refreshCanvasWorktrees(args.path, hydrationToken, args.startupToken);
  void args.updateWindowSession(args.path);
  args.scheduleIssueCacheWarmup(args.path);
}

export async function openProjectAndApplyCurrentWindowRuntime(args: {
  path: string;
  startStartupProfile: () => string;
  discardStartupProfile: (token: string) => void;
  invokeOpenProject: (path: string) => Promise<OpenProjectResult>;
  handleOpenedProjectPath: (path: string, startupToken: string | null) => void;
}): Promise<OpenProjectResult> {
  const startupToken = args.startStartupProfile();
  const result = await args.invokeOpenProject(args.path);
  if (result.action === "opened") {
    args.handleOpenedProjectPath(result.info.path, startupToken);
  } else {
    args.discardStartupProfile(startupToken);
  }
  return result;
}
