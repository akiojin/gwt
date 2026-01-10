/** @jsxImportSource @opentui/solid */
import { useKeyboard, useRenderer } from "@opentui/solid";
import {
  batch,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
} from "solid-js";
import type {
  BranchItem,
  BranchInfo,
  CodingAgentId,
  CleanupStatus,
  InferenceLevel,
  SelectedBranchState,
  Statistics,
  WorktreeInfo as UIWorktreeInfo,
} from "./types.js";
import type { FormattedLogEntry } from "../../logging/formatter.js";
import { BranchListScreen } from "./screens/solid/BranchListScreen.js";
import { HelpOverlay } from "./components/solid/HelpOverlay.js";
import {
  WizardController,
  type WizardResult,
} from "./components/solid/WizardController.js";
import {
  SelectorScreen,
  type SelectorItem,
} from "./screens/solid/SelectorScreen.js";
import { LogScreen } from "./screens/solid/LogScreen.js";
import { LogDetailScreen } from "./screens/solid/LogDetailScreen.js";
import { ErrorScreen } from "./screens/solid/ErrorScreen.js";
import { LoadingIndicatorScreen } from "./screens/solid/LoadingIndicator.js";
import { WorktreeCreateScreen } from "./screens/solid/WorktreeCreateScreen.js";
import { InputScreen } from "./screens/solid/InputScreen.js";
import { ConfirmScreen } from "./screens/solid/ConfirmScreen.js";
import { useTerminalSize } from "./hooks/solid/useTerminalSize.js";
import { EnvironmentScreen } from "./screens/solid/EnvironmentScreen.js";
import { ProfileScreen } from "./screens/solid/ProfileScreen.js";
import { ProfileEnvScreen } from "./screens/solid/ProfileEnvScreen.js";
import { calculateStatistics } from "./utils/statisticsCalculator.js";
import { formatBranchItems } from "./utils/branchFormatter.js";
import { createLogger } from "../../logging/logger.js";

const logger = createLogger({ category: "app" });
import {
  getDefaultInferenceForModel,
  getDefaultModelOption,
  normalizeModelId,
} from "./utils/modelOptions.js";
import {
  resolveBaseBranchLabel,
  resolveBaseBranchRef,
} from "./utils/baseBranch.js";
import {
  getAllBranches,
  getCurrentBranch,
  getLocalBranches,
  getRepositoryRoot,
  deleteBranch,
} from "../../git.js";
import {
  isProtectedBranchName,
  getCleanupStatus,
  listAdditionalWorktrees,
  removeWorktree,
  repairWorktrees,
  type WorktreeInfo as WorktreeEntry,
} from "../../worktree.js";
import {
  getConfig,
  getLastToolUsageMap,
  loadSession,
  type ToolSessionEntry,
} from "../../config/index.js";
import {
  findLatestBranchSessionsByTool,
  refreshQuickStartEntries,
} from "./utils/continueSession.js";
import { getAllCodingAgents } from "../../config/tools.js";
import {
  clearLogFiles,
  getTodayLogDate,
  readLogLinesForDate,
  resolveLogTarget,
  selectLogTargetByRecency,
  type LogTargetResolution,
} from "../../logging/reader.js";
import { parseLogLines } from "../../logging/formatter.js";
import { copyToClipboard } from "./utils/clipboard.js";
import { getPackageVersion } from "../../utils.js";
import {
  createProfile,
  deleteProfile,
  loadProfiles,
  setActiveProfile,
  updateProfile,
} from "../../config/profiles.js";
import {
  isValidProfileName,
  type ProfilesConfig,
} from "../../types/profiles.js";
import { BRANCH_PREFIXES } from "../../config/constants.js";
import { prefetchAgentVersions } from "./utils/versionCache.js";
import { getBunxAgentIds } from "./utils/versionFetcher.js";

export type ExecutionMode = "normal" | "continue" | "resume";

const UNSAFE_SELECTION_MESSAGE = "Unsafe branch selected. Select anyway?";
const SAFETY_PENDING_MESSAGE = "Safety check in progress. Select anyway?";

export interface SelectionResult {
  branch: string;
  displayName: string;
  branchType: "local" | "remote";
  remoteBranch?: string;
  baseBranch?: string;
  isNewBranch?: boolean;
  tool: CodingAgentId;
  mode: ExecutionMode;
  skipPermissions: boolean;
  model?: string | null;
  inferenceLevel?: InferenceLevel;
  sessionId?: string | null;
  toolVersion?: string | null;
}

export type AppScreen =
  | "branch-list"
  | "tool-select"
  | "mode-select"
  | "skip-permissions"
  | "log-list"
  | "log-detail"
  | "profile"
  | "profile-env"
  | "profile-input"
  | "profile-confirm"
  | "profile-os-env"
  | "profile-error"
  | "worktree-create"
  | "loading"
  | "error";

type ProfileInputMode = "create-profile" | "add-env" | "edit-env";
type ProfileConfirmMode = "delete-profile" | "delete-env";
type StatusColor = "cyan" | "green" | "yellow" | "red";

interface CleanupIndicator {
  icon: string;
  isSpinning?: boolean;
  color?: StatusColor;
}

interface CleanupTask {
  branch: string;
  worktreePath: string | null;
  cleanupType: "worktree-and-branch" | "branch-only";
  isAccessible?: boolean;
}

export interface AppSolidProps {
  onExit?: (result?: SelectionResult) => void;
  loadingIndicatorDelay?: number;
  initialScreen?: AppScreen;
  version?: string | null;
  workingDirectory?: string;
  branches?: BranchItem[];
  stats?: Statistics;
}

const DEFAULT_SCREEN: AppScreen = "branch-list";

const buildStats = (branches: BranchItem[]): Statistics =>
  calculateStatistics(branches);

const applyCleanupStatus = (
  items: BranchItem[],
  statusByBranch: Map<string, CleanupStatus>,
): BranchItem[] =>
  items.map((branch) => {
    const status = statusByBranch.get(branch.name);
    if (!status) {
      return { ...branch, safeToCleanup: false, isUnmerged: false };
    }
    const safeToCleanup =
      status.hasUpstream && status.reasons.includes("no-diff-with-base");
    const isUnmerged = status.hasUpstream && status.hasUniqueCommits;
    const worktree = branch.worktree
      ? {
          ...branch.worktree,
          ...(status.hasUncommittedChanges !== undefined
            ? { hasUncommittedChanges: status.hasUncommittedChanges }
            : {}),
        }
      : undefined;
    const base: BranchItem = {
      ...branch,
      safeToCleanup,
      isUnmerged,
      hasUnpushedCommits: status.hasUnpushedCommits,
    };
    return worktree ? { ...base, worktree } : base;
  });

const buildCleanupSafetyPending = (items: BranchItem[]): Set<string> => {
  const pending = new Set<string>();
  for (const branch of items) {
    if (branch.type === "remote") {
      continue;
    }
    if (branch.worktree) {
      pending.add(branch.name);
      continue;
    }
    if (isProtectedBranchName(branch.name)) {
      continue;
    }
    pending.add(branch.name);
  }
  return pending;
};

const toLocalBranchName = (name: string): string => {
  const segments = name.split("/");
  if (segments.length <= 1) {
    return name;
  }
  return segments.slice(1).join("/");
};

const toSelectedBranchState = (branch: BranchItem): SelectedBranchState => {
  const isRemote = branch.type === "remote";
  const baseName = isRemote ? toLocalBranchName(branch.name) : branch.name;
  return {
    name: baseName,
    displayName: branch.name,
    branchType: branch.type,
    branchCategory: branch.branchType,
    worktreePath: branch.worktree?.path ?? null,
    ...(isRemote ? { remoteBranch: branch.name } : {}),
  };
};

export function AppSolid(props: AppSolidProps) {
  const renderer = useRenderer();
  const terminal = useTerminalSize();
  let hasExited = false;

  const exitApp = (result?: SelectionResult) => {
    if (hasExited) return;
    hasExited = true;
    props.onExit?.(result);
    renderer.destroy();
  };

  const [currentScreen, setCurrentScreen] = createSignal<AppScreen>(
    props.initialScreen ?? DEFAULT_SCREEN,
  );
  const [screenStack, setScreenStack] = createSignal<AppScreen[]>([]);
  const [helpVisible, setHelpVisible] = createSignal(false);
  const [wizardVisible, setWizardVisible] = createSignal(false);

  const [branchItems, setBranchItems] = createSignal<BranchItem[]>(
    props.branches ?? [],
  );
  const [stats, setStats] = createSignal<Statistics>(
    props.stats ?? buildStats(props.branches ?? []),
  );
  const [loading, setLoading] = createSignal(!props.branches);
  const [error, setError] = createSignal<Error | null>(null);

  // ブランチ一覧のカーソル位置（グローバル管理で再マウント時もリセットされない）
  const [branchCursorPosition, setBranchCursorPosition] = createSignal(0);

  const [toolItems, setToolItems] = createSignal<SelectorItem[]>([]);
  const [toolError, setToolError] = createSignal<Error | null>(null);

  const [version, setVersion] = createSignal<string | null>(
    props.version ?? null,
  );

  const workingDirectory = createMemo(
    () => props.workingDirectory ?? process.cwd(),
  );

  const [selectedBranch, setSelectedBranch] =
    createSignal<SelectedBranchState | null>(null);
  const [selectedTool, setSelectedTool] = createSignal<CodingAgentId | null>(
    null,
  );
  const [selectedMode, setSelectedMode] = createSignal<ExecutionMode>("normal");
  const [selectedBranches, setSelectedBranches] = createSignal<string[]>([]);
  const [unsafeSelectionConfirmVisible, setUnsafeSelectionConfirmVisible] =
    createSignal(false);
  const [unsafeConfirmInputLocked, setUnsafeConfirmInputLocked] =
    createSignal(false);
  const [unsafeSelectionTarget, setUnsafeSelectionTarget] = createSignal<
    string | null
  >(null);
  const [unsafeSelectionMessage, setUnsafeSelectionMessage] = createSignal(
    UNSAFE_SELECTION_MESSAGE,
  );
  const [branchFooterMessage, setBranchFooterMessage] = createSignal<{
    text: string;
    isSpinning?: boolean;
    color?: StatusColor;
  } | null>(null);
  const [branchInputLocked, setBranchInputLocked] = createSignal(false);
  const [cleanupIndicators, setCleanupIndicators] = createSignal<
    Record<string, CleanupIndicator>
  >({});
  const [cleanupStatusByBranch, setCleanupStatusByBranch] = createSignal<
    Map<string, CleanupStatus>
  >(new Map());
  const [cleanupSafetyLoading, setCleanupSafetyLoading] = createSignal(false);
  const [cleanupSafetyPending, setCleanupSafetyPending] = createSignal<
    Set<string>
  >(new Set());
  const [isNewBranch, setIsNewBranch] = createSignal(false);
  const [newBranchBaseRef, setNewBranchBaseRef] = createSignal<string | null>(
    null,
  );
  const [creationSource, setCreationSource] =
    createSignal<SelectedBranchState | null>(null);
  const [createBranchName, setCreateBranchName] = createSignal("", {
    equals: false,
  });
  const [suppressCreateKey, setSuppressCreateKey] = createSignal<string | null>(
    null,
  );
  const [defaultBaseBranch, setDefaultBaseBranch] = createSignal("main");

  const suppressBranchInputOnce = () => {
    setUnsafeConfirmInputLocked(true);
    queueMicrotask(() => {
      setUnsafeConfirmInputLocked(false);
    });
  };

  const unsafeConfirmBoxWidth = createMemo(() => {
    const columns = terminal().columns || 80;
    return Math.max(1, Math.floor(columns * 0.6));
  });
  const unsafeConfirmContentWidth = createMemo(() =>
    Math.max(0, unsafeConfirmBoxWidth() - 4),
  );

  // セッション履歴（最終使用エージェントなど）
  const [sessionHistory, setSessionHistory] = createSignal<ToolSessionEntry[]>(
    [],
  );
  const [quickStartHistory, setQuickStartHistory] = createSignal<
    ToolSessionEntry[]
  >([]);

  // 選択中ブランチの履歴をフィルタリング
  const historyForBranch = createMemo(() => {
    const history = sessionHistory();
    const branch = selectedBranch();
    if (!branch) return [];
    return findLatestBranchSessionsByTool(
      history,
      branch.name,
      branch.worktreePath ?? null,
    );
  });

  createEffect(() => {
    setQuickStartHistory(historyForBranch());
  });

  createEffect(() => {
    const branch = selectedBranch();
    const baseHistory = historyForBranch();
    if (!wizardVisible() || !branch || baseHistory.length === 0) {
      return;
    }

    const worktreePath = branch.worktreePath ?? null;
    if (!worktreePath) {
      return;
    }

    const branchName = branch.name;
    void (async () => {
      const refreshed = await refreshQuickStartEntries(baseHistory, {
        branch: branchName,
        worktreePath,
      });
      if (selectedBranch()?.name !== branchName) {
        return;
      }
      setQuickStartHistory(refreshed);
    })();
  });

  const [logEntries, setLogEntries] = createSignal<FormattedLogEntry[]>([]);
  const [logLoading, setLogLoading] = createSignal(false);
  const [logError, setLogError] = createSignal<string | null>(null);
  const [logSelectedEntry, setLogSelectedEntry] =
    createSignal<FormattedLogEntry | null>(null);
  const [logSelectedDate, setLogSelectedDate] = createSignal<string | null>(
    getTodayLogDate(),
  );
  const [logNotification, setLogNotification] = createSignal<{
    message: string;
    tone: "success" | "error";
  } | null>(null);
  const [logTailEnabled, setLogTailEnabled] = createSignal(false);
  const [logTargetBranch, setLogTargetBranch] = createSignal<BranchItem | null>(
    null,
  );

  const [profileItems, setProfileItems] = createSignal<
    { name: string; displayName?: string; isActive?: boolean }[]
  >([]);
  const [activeProfile, setActiveProfileName] = createSignal<string | null>(
    null,
  );
  const [profileError, setProfileError] = createSignal<Error | null>(null);
  const [profilesConfig, setProfilesConfig] =
    createSignal<ProfilesConfig | null>(null);
  const [profileActionError, setProfileActionError] =
    createSignal<Error | null>(null);
  const [profileActionHint, setProfileActionHint] = createSignal<string | null>(
    null,
  );
  const [selectedProfileName, setSelectedProfileName] = createSignal<
    string | null
  >(null);
  const [profileInputValue, setProfileInputValue] = createSignal("", {
    equals: false,
  });
  const [profileInputMode, setProfileInputMode] =
    createSignal<ProfileInputMode>("create-profile");
  const [profileInputSuppressKey, setProfileInputSuppressKey] = createSignal<
    string | null
  >(null);
  const [profileEnvKey, setProfileEnvKey] = createSignal<string | null>(null);
  const [profileConfirmMode, setProfileConfirmMode] =
    createSignal<ProfileConfirmMode>("delete-profile");

  const [logEffectiveTarget, setLogEffectiveTarget] =
    createSignal<LogTargetResolution | null>(null);
  const logPrimaryTarget = createMemo(() =>
    resolveLogTarget(logTargetBranch(), workingDirectory()),
  );
  const logFallbackTarget = createMemo(() =>
    resolveLogTarget(null, workingDirectory()),
  );
  const logActiveTarget = createMemo(
    () => logEffectiveTarget() ?? logPrimaryTarget(),
  );
  const logBranchLabel = createMemo(() => logTargetBranch()?.label ?? null);
  const logSourceLabel = createMemo(() => {
    const target = logActiveTarget();
    if (!target.sourcePath) {
      return "(none)";
    }
    if (
      target.reason === "current-working-directory" ||
      target.reason === "working-directory"
    ) {
      return `${target.sourcePath} (cwd)`;
    }
    if (target.reason === "working-directory-fallback") {
      return `${target.sourcePath} (cwd fallback)`;
    }
    if (target.reason === "worktree-inaccessible") {
      return `${target.sourcePath} (inaccessible)`;
    }
    return target.sourcePath;
  });
  createEffect(() => {
    logPrimaryTarget();
    setLogEffectiveTarget(null);
  });
  const selectedProfileConfig = createMemo(() => {
    const name = selectedProfileName();
    const config = profilesConfig();
    if (!name || !config?.profiles?.[name]) {
      return null;
    }
    return { name, profile: config.profiles[name] };
  });
  const profileEnvVariables = createMemo(() => {
    const entry = selectedProfileConfig();
    if (!entry) {
      return [];
    }
    return Object.entries(entry.profile.env ?? {})
      .map(([key, value]) => ({ key, value }))
      .sort((a, b) => a.key.localeCompare(b.key));
  });
  const osEnvVariables = createMemo(() =>
    Object.entries(process.env)
      .filter(([, value]) => typeof value === "string")
      .map(([key, value]) => ({ key, value: String(value) }))
      .sort((a, b) => a.key.localeCompare(b.key)),
  );

  let cleanupSafetyRequestId = 0;
  const refreshCleanupSafety = async () => {
    const requestId = ++cleanupSafetyRequestId;
    const pendingBranches = buildCleanupSafetyPending(branchItems());
    setCleanupSafetyPending(pendingBranches);
    setCleanupSafetyLoading(pendingBranches.size > 0);
    const statusByBranch = new Map<string, CleanupStatus>();
    const applyProgress = (status: CleanupStatus) => {
      if (requestId !== cleanupSafetyRequestId) {
        return;
      }
      statusByBranch.set(status.branch, status);
      batch(() => {
        setCleanupStatusByBranch(new Map(statusByBranch));
        setBranchItems((items) => applyCleanupStatus(items, statusByBranch));
        setCleanupSafetyPending((prev) => {
          if (!prev.has(status.branch)) {
            return prev;
          }
          const next = new Set(prev);
          next.delete(status.branch);
          return next;
        });
      });
    };
    try {
      const cleanupStatuses = await getCleanupStatus({
        onProgress: applyProgress,
      });
      if (requestId !== cleanupSafetyRequestId) {
        return;
      }
      if (cleanupStatuses.length > statusByBranch.size) {
        cleanupStatuses.forEach((status) => {
          if (!statusByBranch.has(status.branch)) {
            applyProgress(status);
          }
        });
      }
    } catch (err) {
      if (requestId !== cleanupSafetyRequestId) {
        return;
      }
      logger.warn({ err }, "Failed to refresh cleanup safety indicators");
      const empty = new Map<string, CleanupStatus>();
      batch(() => {
        setCleanupStatusByBranch(empty);
        setBranchItems((items) => applyCleanupStatus(items, empty));
      });
    } finally {
      if (requestId === cleanupSafetyRequestId) {
        setCleanupSafetyLoading(false);
        setCleanupSafetyPending(new Set<string>());
      }
    }
  };

  let logNotificationTimer: ReturnType<typeof setTimeout> | null = null;
  let logTailTimer: ReturnType<typeof setInterval> | null = null;
  let branchFooterTimer: ReturnType<typeof setTimeout> | null = null;
  const BRANCH_LOAD_TIMEOUT_MS = 3000;
  const BRANCH_FULL_LOAD_TIMEOUT_MS = 8000;

  const isHelpKey = (key: {
    name: string;
    sequence: string;
    ctrl: boolean;
    meta: boolean;
    super?: boolean;
    hyper?: boolean;
    option?: boolean;
  }) => {
    if (key.ctrl || key.meta || key.super || key.hyper || key.option) {
      return false;
    }
    return key.name === "h" || key.sequence === "h" || key.sequence === "?";
  };

  useKeyboard((key) => {
    if (key.repeated) {
      return;
    }

    if (helpVisible()) {
      if (key.name === "escape" || isHelpKey(key)) {
        setHelpVisible(false);
      }
      return;
    }

    if (isHelpKey(key)) {
      setHelpVisible(true);
    }
  });

  const navigateTo = (screen: AppScreen) => {
    setScreenStack((prev) => [...prev, currentScreen()]);
    setCurrentScreen(screen);
  };

  const goBack = () => {
    const stack = screenStack();
    if (stack.length === 0) {
      return;
    }
    const previous = stack[stack.length - 1] ?? DEFAULT_SCREEN;
    setScreenStack(stack.slice(0, -1));

    // branch-list に戻る場合は選択状態をリセット
    if (previous === "branch-list") {
      setSelectedBranch(null);
      setSelectedTool(null);
      setSelectedMode("normal");
      setIsNewBranch(false);
      setNewBranchBaseRef(null);
      setCreationSource(null);
    }

    setCurrentScreen(previous);
  };

  const openProfileError = (err: unknown, hint: string) => {
    setProfileActionError(err instanceof Error ? err : new Error(String(err)));
    setProfileActionHint(hint);
    navigateTo("profile-error");
  };

  const showLogNotification = (message: string, tone: "success" | "error") => {
    setLogNotification({ message, tone });
    if (logNotificationTimer) {
      clearTimeout(logNotificationTimer);
    }
    logNotificationTimer = setTimeout(() => {
      setLogNotification(null);
    }, 2000);
  };

  const loadLogEntries = async (date: string | null) => {
    const targetDate = date ?? getTodayLogDate();
    setLogLoading(true);
    setLogError(null);
    try {
      const primaryTarget = logPrimaryTarget();
      const fallbackTarget = logFallbackTarget();
      const target = await selectLogTargetByRecency(
        primaryTarget,
        fallbackTarget,
      );
      setLogEffectiveTarget(target);
      if (!target.logDir) {
        setLogEntries([]);
        setLogSelectedDate(targetDate);
        return;
      }
      const result = await readLogLinesForDate(target.logDir, targetDate);
      if (!result) {
        setLogEntries([]);
        setLogSelectedDate(targetDate);
        return;
      }
      setLogSelectedDate(result.date);
      const parsed = parseLogLines(result.lines, { limit: 100 });
      setLogEntries(parsed);
    } catch (err) {
      setLogEntries([]);
      setLogError(err instanceof Error ? err.message : "Failed to load logs");
    } finally {
      setLogLoading(false);
    }
  };

  const clearLogTailTimer = () => {
    if (logTailTimer) {
      clearInterval(logTailTimer);
      logTailTimer = null;
    }
  };

  const toggleLogTail = () => {
    setLogTailEnabled((prev) => !prev);
  };

  const resetLogFiles = async () => {
    const target = logActiveTarget();
    if (!target.logDir) {
      showLogNotification("No logs available.", "error");
      return;
    }
    try {
      const cleared = await clearLogFiles(target.logDir);
      if (cleared === 0) {
        showLogNotification("No logs to reset.", "error");
      } else {
        showLogNotification("Logs cleared.", "success");
      }
      await loadLogEntries(logSelectedDate());
    } catch (err) {
      logger.warn({ err }, "Failed to clear log files");
      showLogNotification("Failed to reset logs.", "error");
    }
  };

  const refreshBranches = async () => {
    setLoading(true);
    setError(null);
    try {
      const repoRoot = await getRepositoryRoot();
      const worktreesPromise = listAdditionalWorktrees();
      const localBranchesPromise = getLocalBranches(repoRoot);
      const currentBranchPromise = getCurrentBranch(repoRoot);
      const lastToolUsagePromise = getLastToolUsageMap(repoRoot);

      const [localBranches, currentBranch, worktrees, lastToolUsageMap] =
        await Promise.all([
          withTimeout(localBranchesPromise, BRANCH_LOAD_TIMEOUT_MS).catch(
            () => [],
          ),
          withTimeout(currentBranchPromise, BRANCH_LOAD_TIMEOUT_MS).catch(
            () => null,
          ),
          withTimeout(worktreesPromise, BRANCH_LOAD_TIMEOUT_MS).catch(() => []),
          withTimeout(lastToolUsagePromise, BRANCH_LOAD_TIMEOUT_MS).catch(
            () => new Map<string, ToolSessionEntry>(),
          ),
        ]);

      if (currentBranch) {
        localBranches.forEach((branch) => {
          if (branch.name === currentBranch) {
            branch.isCurrent = true;
          }
        });
      }

      const initial = buildBranchList(
        localBranches,
        worktrees,
        lastToolUsageMap,
      );
      const initialItems = applyCleanupStatus(
        initial.items,
        cleanupStatusByBranch(),
      );
      setBranchItems(initialItems);
      setStats(buildStats(initialItems));
      void refreshCleanupSafety();

      void (async () => {
        const [branches, latestWorktrees] = await Promise.all([
          withTimeout(
            getAllBranches(repoRoot),
            BRANCH_FULL_LOAD_TIMEOUT_MS,
          ).catch(() => localBranches),
          withTimeout(
            listAdditionalWorktrees(),
            BRANCH_FULL_LOAD_TIMEOUT_MS,
          ).catch(() => worktrees),
        ]);

        const full = buildBranchList(
          branches,
          latestWorktrees,
          lastToolUsageMap,
        );
        const fullItems = applyCleanupStatus(
          full.items,
          cleanupStatusByBranch(),
        );
        setBranchItems(fullItems);
        setStats(buildStats(fullItems));
      })();
    } catch (err) {
      setError(err instanceof Error ? err : new Error(String(err)));
    } finally {
      setLoading(false);
    }
  };

  const refreshProfiles = async () => {
    try {
      const config = await loadProfiles();
      setProfilesConfig(config);
      const items = Object.entries(config.profiles ?? {}).map(
        ([name, profile]) => ({
          name,
          displayName: profile.displayName,
          isActive: config.activeProfile === name,
        }),
      );
      items.sort((a, b) => a.name.localeCompare(b.name));
      setProfileItems(items);
      setActiveProfileName(config.activeProfile ?? null);
      setProfileError(null);
    } catch (err) {
      setProfilesConfig(null);
      setProfileItems([]);
      setActiveProfileName(null);
      setProfileError(err instanceof Error ? err : new Error(String(err)));
    }
  };

  createEffect(() => {
    if (props.branches) {
      const statusByBranch = cleanupStatusByBranch();
      const nextItems = applyCleanupStatus(props.branches, statusByBranch);
      setBranchItems(nextItems);
      setStats(props.stats ?? buildStats(nextItems));
      setLoading(false);
    }
  });

  onMount(() => {
    if (!props.branches) {
      void refreshBranches();
    }
  });

  onMount(() => {
    if (props.branches) {
      void refreshCleanupSafety();
    }
  });

  onMount(() => {
    if (props.version === undefined) {
      void getPackageVersion()
        .then((value) => setVersion(value ?? null))
        .catch(() => setVersion(null));
    }
  });

  onMount(() => {
    void refreshProfiles();
  });

  onMount(() => {
    void getConfig()
      .then((config) => setDefaultBaseBranch(config.defaultBaseBranch))
      .catch(() => setDefaultBaseBranch("main"));
  });

  // セッション履歴をロード（最終使用エージェントなど）
  onMount(() => {
    void getRepositoryRoot()
      .then((repoRoot) => loadSession(repoRoot))
      .then((session) => {
        if (session?.history) {
          setSessionHistory(session.history);
        }
      })
      .catch(() => setSessionHistory([]));
  });

  onMount(() => {
    void getAllCodingAgents()
      .then((agents) => {
        setToolItems(
          agents.map((agent) => ({
            label: agent.displayName,
            value: agent.id,
          })),
        );
      })
      .catch((err) => {
        setToolItems([]);
        setToolError(err instanceof Error ? err : new Error(String(err)));
      });
  });

  // FR-028: Prefetch npm versions for all bunx-type agents at startup (background)
  onMount(() => {
    const bunxAgentIds = getBunxAgentIds();
    void prefetchAgentVersions(bunxAgentIds).catch(() => {
      // Silently handle errors - cache will return null and UI will show "latest" only
    });
  });

  createEffect(() => {
    if (currentScreen() === "log-list") {
      logPrimaryTarget();
      void loadLogEntries(logSelectedDate());
    }
  });

  createEffect(() => {
    if (currentScreen() !== "log-list" || !logTailEnabled()) {
      clearLogTailTimer();
      return;
    }
    clearLogTailTimer();
    logTailTimer = setInterval(() => {
      void loadLogEntries(logSelectedDate());
    }, 1500);
  });

  onCleanup(() => {
    if (logNotificationTimer) {
      clearTimeout(logNotificationTimer);
    }
    clearLogTailTimer();
    if (branchFooterTimer) {
      clearTimeout(branchFooterTimer);
    }
  });

  const showBranchFooterMessage = (
    text: string,
    color: StatusColor,
    options?: { spinning?: boolean; timeoutMs?: number },
  ) => {
    if (branchFooterTimer) {
      clearTimeout(branchFooterTimer);
      branchFooterTimer = null;
    }
    setBranchFooterMessage({
      text,
      color,
      ...(options?.spinning ? { isSpinning: true } : {}),
    });
    const timeout = options?.timeoutMs ?? 2000;
    if (timeout > 0) {
      branchFooterTimer = setTimeout(() => {
        setBranchFooterMessage(null);
      }, timeout);
    }
  };

  const setCleanupIndicator = (
    branchName: string,
    indicator: CleanupIndicator | null,
  ) => {
    setCleanupIndicators((prev) => {
      const next = { ...prev };
      if (indicator) {
        next[branchName] = indicator;
      } else {
        delete next[branchName];
      }
      return next;
    });
  };

  const handleBranchSelect = (branch: BranchItem) => {
    setSelectedBranch(toSelectedBranchState(branch));
    setIsNewBranch(false);
    setNewBranchBaseRef(null);
    setCreationSource(null);
    setSelectedTool(null);
    setSelectedMode("normal");
    // FR-044: ウィザードポップアップをレイヤー表示
    setWizardVisible(true);
  };

  const handleQuickCreate = (branch: BranchItem | null) => {
    // 選択中のブランチをベースにウィザードを開始
    if (branch) {
      setSelectedBranch(toSelectedBranchState(branch));
    }
    setCreationSource(branch ? toSelectedBranchState(branch) : null);
    setCreateBranchName("");
    setSuppressCreateKey("n");
    // FR-044: ウィザードポップアップをレイヤー表示（アクション選択から開始）
    setWizardVisible(true);
  };

  // FR-049: Escapeキーでウィザードをキャンセル
  const handleWizardClose = () => {
    setWizardVisible(false);
  };

  // ウィザード完了時の処理
  const handleWizardComplete = (result: WizardResult) => {
    setWizardVisible(false);

    const branch = selectedBranch();
    if (!branch) {
      return;
    }

    // 新規ブランチ作成の場合、ブランチ名を設定
    const isCreatingNew = result.isNewBranch ?? false;
    const finalBranchName = result.branchName
      ? `${result.branchType ?? ""}${result.branchName}`
      : branch.name;

    // 新規作成時は選択中のブランチがベースとなる
    const baseBranchRef = isCreatingNew ? branch.name : null;
    const normalizedModel = normalizeModelId(result.tool, result.model);

    exitApp({
      branch: finalBranchName,
      displayName: branch.displayName,
      branchType: branch.branchType,
      ...(branch.remoteBranch ? { remoteBranch: branch.remoteBranch } : {}),
      ...(isCreatingNew
        ? {
            isNewBranch: true,
            ...(baseBranchRef ? { baseBranch: baseBranchRef } : {}),
          }
        : {}),
      tool: result.tool,
      mode: result.mode,
      skipPermissions: result.skipPermissions,
      ...(normalizedModel !== undefined ? { model: normalizedModel } : {}),
      ...(result.reasoningLevel !== undefined
        ? { inferenceLevel: result.reasoningLevel }
        : {}),
      ...(result.toolVersion !== undefined
        ? { toolVersion: result.toolVersion }
        : {}),
    });
  };

  // FR-010: クイックスタートからのResume（前回設定で続きから）
  const handleWizardResume = (entry: ToolSessionEntry) => {
    if (!entry.sessionId) {
      handleWizardStartNew(entry);
      return;
    }

    setWizardVisible(false);

    const branch = selectedBranch();
    if (!branch) {
      return;
    }

    const normalizedModel = normalizeModelId(
      entry.toolId as CodingAgentId,
      entry.model ?? undefined,
    );

    exitApp({
      branch: branch.name,
      displayName: branch.displayName,
      branchType: branch.branchType,
      ...(branch.remoteBranch ? { remoteBranch: branch.remoteBranch } : {}),
      tool: entry.toolId as CodingAgentId,
      mode: "continue",
      skipPermissions: entry.skipPermissions ?? false,
      ...(normalizedModel !== undefined ? { model: normalizedModel } : {}),
      ...(entry.reasoningLevel
        ? { inferenceLevel: entry.reasoningLevel as InferenceLevel }
        : {}),
      ...(entry.sessionId ? { sessionId: entry.sessionId } : {}),
      ...(entry.toolVersion !== undefined
        ? { toolVersion: entry.toolVersion }
        : {}),
    });
  };

  // FR-010: クイックスタートからのStartNew（前回設定で新規）
  const handleWizardStartNew = (entry: ToolSessionEntry) => {
    setWizardVisible(false);

    const branch = selectedBranch();
    if (!branch) {
      return;
    }

    const normalizedModel = normalizeModelId(
      entry.toolId as CodingAgentId,
      entry.model ?? undefined,
    );

    exitApp({
      branch: branch.name,
      displayName: branch.displayName,
      branchType: branch.branchType,
      ...(branch.remoteBranch ? { remoteBranch: branch.remoteBranch } : {}),
      tool: entry.toolId as CodingAgentId,
      mode: "normal",
      skipPermissions: entry.skipPermissions ?? false,
      ...(normalizedModel !== undefined ? { model: normalizedModel } : {}),
      ...(entry.reasoningLevel
        ? { inferenceLevel: entry.reasoningLevel as InferenceLevel }
        : {}),
      ...(entry.toolVersion !== undefined
        ? { toolVersion: entry.toolVersion }
        : {}),
    });
  };

  const buildSkipNotice = (
    skip: {
      unsafe: number;
      protected: number;
      remote: number;
      current: number;
    },
    unsafeLabel: string,
  ): string | null => {
    const parts: string[] = [];
    if (skip.unsafe > 0) {
      parts.push(`${skip.unsafe} ${unsafeLabel}`);
    }
    if (skip.protected > 0) {
      parts.push(`${skip.protected} protected`);
    }
    if (skip.remote > 0) {
      parts.push(`${skip.remote} remote-only`);
    }
    if (skip.current > 0) {
      parts.push(`${skip.current} current`);
    }
    if (parts.length === 0) {
      return null;
    }
    return `Skipped branches: ${parts.join(", ")}.`;
  };

  const handleCleanupCommand = async () => {
    if (branchInputLocked()) {
      return;
    }

    const selection = selectedBranches();
    const hasSelection = selection.length > 0;
    const skipCounts = {
      unsafe: 0,
      protected: 0,
      remote: 0,
      current: 0,
    };

    setBranchInputLocked(true);
    setCleanupIndicators({});
    showBranchFooterMessage(
      hasSelection
        ? "Preparing cleanup..."
        : "Scanning for cleanup candidates...",
      "yellow",
      { spinning: true, timeoutMs: 0 },
    );

    try {
      const tasks: CleanupTask[] = [];

      if (hasSelection) {
        const branchMap = new Map(
          branchItems().map((branch) => [branch.name, branch]),
        );
        for (const branchName of selection) {
          const branch = branchMap.get(branchName);
          if (!branch) {
            continue;
          }
          if (branch.type === "remote") {
            skipCounts.remote += 1;
            continue;
          }
          if (branch.isCurrent) {
            skipCounts.current += 1;
            continue;
          }
          const worktreePath = branch.worktree?.path ?? null;
          const cleanupType: CleanupTask["cleanupType"] = worktreePath
            ? "worktree-and-branch"
            : "branch-only";
          const baseTask = {
            branch: branch.name,
            worktreePath,
            cleanupType,
          };
          const isAccessible = branch.worktree?.isAccessible;
          tasks.push(
            isAccessible === undefined
              ? baseTask
              : { ...baseTask, isAccessible },
          );
        }
      } else {
        // FR-028: 選択が0件の場合は警告を表示して処理をスキップ
        setBranchInputLocked(false);
        showBranchFooterMessage("No branches selected.", "yellow");
        return;
      }

      const skipNotice = buildSkipNotice(
        skipCounts,
        hasSelection
          ? "unsafe (uncommitted/unpushed, unmerged, or missing upstream)"
          : "with uncommitted or unpushed changes",
      );

      if (tasks.length === 0) {
        const baseMessage = hasSelection
          ? "No eligible branches selected for cleanup."
          : "No cleanup candidates found.";
        showBranchFooterMessage(
          skipNotice ? `${baseMessage} ${skipNotice}` : baseMessage,
          "yellow",
        );
        return;
      }

      setCleanupIndicators(() => {
        const next: Record<string, CleanupIndicator> = {};
        tasks.forEach((task) => {
          next[task.branch] = {
            icon: "-",
            isSpinning: true,
            color: "yellow",
          };
        });
        return next;
      });

      showBranchFooterMessage(
        `Cleaning up ${tasks.length} branch(es)...`,
        "yellow",
        { spinning: true, timeoutMs: 0 },
      );

      let successCount = 0;
      let failedCount = 0;

      for (const task of tasks) {
        try {
          if (task.cleanupType === "worktree-and-branch" && task.worktreePath) {
            await removeWorktree(
              task.worktreePath,
              task.isAccessible === false,
            );
          }
          await deleteBranch(task.branch, true);
          setCleanupIndicator(task.branch, { icon: "v", color: "green" });
          successCount += 1;
        } catch {
          setCleanupIndicator(task.branch, { icon: "x", color: "red" });
          failedCount += 1;
        }
      }

      setSelectedBranches([]);
      await refreshBranches();

      const skippedTotal =
        skipCounts.unsafe +
        skipCounts.protected +
        skipCounts.remote +
        skipCounts.current;
      const summaryParts = [`${successCount} cleaned`];
      if (failedCount > 0) {
        summaryParts.push(`${failedCount} failed`);
      }
      if (skippedTotal > 0) {
        summaryParts.push(`${skippedTotal} skipped`);
      }
      const summary = `Cleanup finished: ${summaryParts.join(", ")}.`;
      const message = skipNotice ? `${summary} ${skipNotice}` : summary;
      showBranchFooterMessage(message, failedCount > 0 ? "red" : "green");
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      showBranchFooterMessage(`Cleanup failed: ${errorMessage}`, "red");
    } finally {
      setBranchInputLocked(false);
    }
  };

  const handleRepairWorktrees = async () => {
    if (branchInputLocked()) {
      return;
    }

    // FR-002/FR-007: 選択済みブランチのみを対象とする
    const selection = selectedBranches();
    if (selection.length === 0) {
      showBranchFooterMessage("No branches selected.", "yellow");
      return;
    }

    // 選択されたローカルブランチのうち、Worktreeを持つものを修復対象とする
    const selectionSet = new Set(selection);
    const targets = branchItems()
      .filter(
        (branch) =>
          selectionSet.has(branch.name) &&
          branch.type !== "remote" &&
          branch.worktreeStatus !== undefined,
      )
      .map((branch) => branch.name);

    if (targets.length === 0) {
      showBranchFooterMessage("No worktrees to repair.", "yellow");
      return;
    }

    setBranchInputLocked(true);
    showBranchFooterMessage("Repairing worktrees...", "yellow", {
      spinning: true,
      timeoutMs: 0,
    });
    try {
      const result = await repairWorktrees(targets);
      await refreshBranches();

      // エラー詳細をログに出力
      if (result.failedCount > 0) {
        logger.error(
          { failures: result.failures, targets },
          "Worktree repair failed for some branches",
        );
      }

      const message =
        result.failedCount > 0
          ? `Repair finished: ${result.repairedCount} repaired, ${result.failedCount} failed.`
          : result.repairedCount > 0
            ? `Repaired ${result.repairedCount} worktree(s).`
            : "No worktrees repaired.";
      showBranchFooterMessage(
        message,
        result.failedCount > 0 ? "red" : "green",
      );
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      logger.error({ error: err, targets }, "Worktree repair threw an error");
      showBranchFooterMessage(`Repair failed: ${errorMessage}`, "red");
    } finally {
      setBranchInputLocked(false);
    }
  };

  const openProfileCreate = () => {
    setProfileInputMode("create-profile");
    setProfileInputValue("");
    setProfileEnvKey(null);
    setProfileInputSuppressKey("n");
    navigateTo("profile-input");
  };

  const openProfileDelete = (profile: { name: string }) => {
    setSelectedProfileName(profile.name);
    setProfileConfirmMode("delete-profile");
    navigateTo("profile-confirm");
  };

  const openProfileEnv = (profile: { name: string }) => {
    setSelectedProfileName(profile.name);
    navigateTo("profile-env");
  };

  const openProfileEnvAdd = () => {
    setProfileInputMode("add-env");
    setProfileInputValue("");
    setProfileEnvKey(null);
    setProfileInputSuppressKey("a");
    navigateTo("profile-input");
  };

  const openProfileEnvEdit = (variable: { key: string; value: string }) => {
    setProfileInputMode("edit-env");
    setProfileEnvKey(variable.key);
    setProfileInputValue(variable.value);
    setProfileInputSuppressKey("e");
    navigateTo("profile-input");
  };

  const openProfileEnvDelete = (variable: { key: string }) => {
    setProfileConfirmMode("delete-env");
    setProfileEnvKey(variable.key);
    navigateTo("profile-confirm");
  };

  const handleProfileInputChange = (value: string) => {
    const suppressKey = profileInputSuppressKey();
    if (suppressKey && profileInputValue() === "" && value === suppressKey) {
      setProfileInputSuppressKey(null);
      setProfileInputValue("");
      return;
    }
    setProfileInputSuppressKey(null);
    setProfileInputValue(value);
  };

  const submitProfileInput = async (value: string) => {
    const mode = profileInputMode();
    const trimmed = value.trim();

    if (mode === "create-profile") {
      if (!trimmed) {
        openProfileError(
          new Error("Profile name is required."),
          "Invalid name.",
        );
        return;
      }
      if (!isValidProfileName(trimmed)) {
        openProfileError(
          new Error(`Invalid profile name: "${trimmed}".`),
          "Profile names must use lowercase letters, numbers, and hyphens.",
        );
        return;
      }
      try {
        await createProfile(trimmed, { displayName: trimmed, env: {} });
        await refreshProfiles();
        setSelectedProfileName(trimmed);
        setProfileInputValue("");
        setProfileInputSuppressKey(null);
        goBack();
      } catch (err) {
        openProfileError(err, "Unable to create profile.");
      }
      return;
    }

    const entry = selectedProfileConfig();
    if (!entry) {
      openProfileError(
        new Error("Profile not selected."),
        "Profile not found.",
      );
      return;
    }

    if (mode === "add-env") {
      const separatorIndex = trimmed.indexOf("=");
      if (separatorIndex <= 0) {
        openProfileError(
          new Error("Environment variable must be in KEY=VALUE format."),
          "Invalid environment variable.",
        );
        return;
      }
      const key = trimmed.slice(0, separatorIndex).trim();
      const valuePart = trimmed.slice(separatorIndex + 1);
      if (!key) {
        openProfileError(
          new Error("Environment variable key is required."),
          "Invalid environment variable.",
        );
        return;
      }
      const nextEnv = { ...(entry.profile.env ?? {}) };
      nextEnv[key] = valuePart;
      try {
        await updateProfile(entry.name, { env: nextEnv });
        await refreshProfiles();
        setProfileInputValue("");
        setProfileInputSuppressKey(null);
        goBack();
      } catch (err) {
        openProfileError(err, "Unable to update profile.");
      }
      return;
    }

    if (mode === "edit-env") {
      const envKey = profileEnvKey();
      if (!envKey) {
        openProfileError(
          new Error("Environment variable not selected."),
          "Select a variable to edit.",
        );
        return;
      }
      const nextEnv = { ...(entry.profile.env ?? {}) };
      nextEnv[envKey] = value;
      try {
        await updateProfile(entry.name, { env: nextEnv });
        await refreshProfiles();
        setProfileInputValue("");
        setProfileInputSuppressKey(null);
        goBack();
      } catch (err) {
        openProfileError(err, "Unable to update profile.");
      }
    }
  };

  const confirmProfileAction = async (confirmed: boolean) => {
    if (!confirmed) {
      goBack();
      return;
    }

    const mode = profileConfirmMode();
    const entry = selectedProfileConfig();

    if (mode === "delete-profile") {
      const name = selectedProfileName();
      if (!name) {
        openProfileError(
          new Error("Profile not selected."),
          "Profile not found.",
        );
        return;
      }
      try {
        await deleteProfile(name);
        await refreshProfiles();
        setProfileEnvKey(null);
        goBack();
      } catch (err) {
        openProfileError(err, "Unable to delete profile.");
      }
      return;
    }

    if (mode === "delete-env") {
      if (!entry) {
        openProfileError(
          new Error("Profile not selected."),
          "Profile not found.",
        );
        return;
      }
      const envKey = profileEnvKey();
      if (!envKey) {
        openProfileError(
          new Error("Environment variable not selected."),
          "Select a variable to delete.",
        );
        return;
      }
      const nextEnv = { ...(entry.profile.env ?? {}) };
      delete nextEnv[envKey];
      try {
        await updateProfile(entry.name, { env: nextEnv });
        await refreshProfiles();
        setProfileEnvKey(null);
        goBack();
      } catch (err) {
        openProfileError(err, "Unable to update profile.");
      }
    }
  };

  const toggleSelectedBranch = (branchName: string) => {
    if (unsafeSelectionConfirmVisible()) {
      return;
    }
    const currentSelection = new Set(selectedBranches());
    if (currentSelection.has(branchName)) {
      currentSelection.delete(branchName);
      setSelectedBranches(Array.from(currentSelection));
      return;
    }

    const branch = branchItems().find((item) => item.name === branchName);
    const pending = cleanupSafetyPending();
    const hasSafetyPending = pending.has(branchName);
    const hasUncommitted = branch?.worktree?.hasUncommittedChanges === true;
    const hasUnpushed = branch?.hasUnpushedCommits === true;
    const isUnmerged = branch?.isUnmerged === true;
    const safeToCleanup = branch?.safeToCleanup === true;
    const isRemoteBranch = branch?.type === "remote";
    const isUnsafe =
      Boolean(branch) &&
      !isRemoteBranch &&
      !hasSafetyPending &&
      (hasUncommitted || hasUnpushed || isUnmerged || !safeToCleanup);

    if (branch && hasSafetyPending) {
      setUnsafeSelectionTarget(branch.name);
      setUnsafeSelectionMessage(SAFETY_PENDING_MESSAGE);
      setUnsafeSelectionConfirmVisible(true);
      return;
    }
    if (branch && isUnsafe) {
      setUnsafeSelectionTarget(branch.name);
      setUnsafeSelectionMessage(UNSAFE_SELECTION_MESSAGE);
      setUnsafeSelectionConfirmVisible(true);
      return;
    }

    currentSelection.add(branchName);
    setSelectedBranches(Array.from(currentSelection));
  };

  const confirmUnsafeSelection = (confirmed: boolean) => {
    suppressBranchInputOnce();
    const target = unsafeSelectionTarget();
    setUnsafeSelectionConfirmVisible(false);
    setUnsafeSelectionTarget(null);
    setUnsafeSelectionMessage(UNSAFE_SELECTION_MESSAGE);
    if (!confirmed || !target) {
      return;
    }
    setSelectedBranches((prev) =>
      prev.includes(target) ? prev : [...prev, target],
    );
  };

  const handleToolSelect = (item: SelectorItem) => {
    setSelectedTool(item.value as CodingAgentId);
    navigateTo("mode-select");
  };

  const handleModeSelect = (item: SelectorItem) => {
    setSelectedMode(item.value as ExecutionMode);
    navigateTo("skip-permissions");
  };

  const finalizeSelection = (skipPermissions: boolean) => {
    const branch = selectedBranch();
    const tool = selectedTool();
    if (!branch || !tool) {
      return;
    }

    const defaultModel = getDefaultModelOption(tool);
    const resolvedModel = defaultModel?.id ?? null;
    const normalizedModel = normalizeModelId(tool, resolvedModel);
    const resolvedInference = getDefaultInferenceForModel(defaultModel);
    const baseRef = newBranchBaseRef();

    exitApp({
      branch: branch.name,
      displayName: branch.displayName,
      branchType: branch.branchType,
      ...(branch.remoteBranch ? { remoteBranch: branch.remoteBranch } : {}),
      ...(isNewBranch()
        ? {
            isNewBranch: true,
            ...(baseRef ? { baseBranch: baseRef } : {}),
          }
        : {}),
      tool,
      mode: selectedMode(),
      skipPermissions,
      ...(normalizedModel !== undefined ? { model: normalizedModel } : {}),
      ...(resolvedInference !== undefined
        ? { inferenceLevel: resolvedInference }
        : {}),
    });
  };

  const renderCurrentScreen = () => {
    const screen = currentScreen();

    if (screen === "branch-list") {
      const cleanupUI = {
        indicators: cleanupIndicators(),
        footerMessage: branchFooterMessage(),
        inputLocked: branchInputLocked() || unsafeConfirmInputLocked(),
        safetyLoading: cleanupSafetyLoading(),
        safetyPendingBranches: cleanupSafetyPending(),
      };
      return (
        <BranchListScreen
          branches={branchItems()}
          stats={stats()}
          onSelect={handleBranchSelect}
          onQuit={() => exitApp(undefined)}
          onCleanupCommand={handleCleanupCommand}
          onRefresh={refreshBranches}
          onRepairWorktrees={handleRepairWorktrees}
          loading={loading()}
          error={error()}
          loadingIndicatorDelay={props.loadingIndicatorDelay ?? 0}
          lastUpdated={stats().lastUpdated}
          version={version()}
          workingDirectory={workingDirectory()}
          activeProfile={activeProfile()}
          onOpenLogs={(branch) => {
            setLogTargetBranch(branch);
            setLogSelectedEntry(null);
            setLogSelectedDate(getTodayLogDate());
            setLogTailEnabled(false);
            navigateTo("log-list");
          }}
          onOpenProfiles={() => navigateTo("profile")}
          selectedBranches={selectedBranches()}
          onToggleSelect={toggleSelectedBranch}
          onCreateBranch={handleQuickCreate}
          cleanupUI={cleanupUI}
          helpVisible={helpVisible()}
          wizardVisible={wizardVisible()}
          confirmVisible={unsafeSelectionConfirmVisible()}
          cursorPosition={branchCursorPosition()}
          onCursorPositionChange={setBranchCursorPosition}
        />
      );
    }

    if (screen === "tool-select") {
      if (toolError()) {
        return (
          <ErrorScreen
            error={toolError() as Error}
            onBack={goBack}
            hint="Unable to load available tools."
          />
        );
      }
      return (
        <SelectorScreen
          title="Select tool"
          items={toolItems()}
          onSelect={handleToolSelect}
          onBack={goBack}
          helpVisible={helpVisible()}
        />
      );
    }

    if (screen === "mode-select") {
      return (
        <SelectorScreen
          title="Execution mode"
          items={[
            { label: "Normal", value: "normal" },
            { label: "Continue", value: "continue" },
            { label: "Resume", value: "resume" },
          ]}
          onSelect={handleModeSelect}
          onBack={goBack}
          helpVisible={helpVisible()}
        />
      );
    }

    if (screen === "skip-permissions") {
      return (
        <SelectorScreen
          title="Skip permission prompts?"
          items={[
            { label: "Yes", value: "true" },
            { label: "No", value: "false" },
          ]}
          onSelect={(item) => finalizeSelection(item.value === "true")}
          onBack={goBack}
          helpVisible={helpVisible()}
        />
      );
    }

    if (screen === "log-list") {
      return (
        <LogScreen
          entries={logEntries()}
          loading={logLoading()}
          error={logError()}
          onBack={goBack}
          onSelect={(entry) => {
            setLogSelectedEntry(entry);
            navigateTo("log-detail");
          }}
          onCopy={async (entry) => {
            try {
              await copyToClipboard(entry.json);
              showLogNotification("Copied to clipboard.", "success");
            } catch {
              showLogNotification("Failed to copy to clipboard.", "error");
            }
          }}
          onReload={() => void loadLogEntries(logSelectedDate())}
          onToggleTail={toggleLogTail}
          onReset={() => void resetLogFiles()}
          notification={logNotification()}
          version={version()}
          selectedDate={logSelectedDate()}
          branchLabel={logBranchLabel()}
          sourceLabel={logSourceLabel()}
          tailing={logTailEnabled()}
          helpVisible={helpVisible()}
        />
      );
    }

    if (screen === "log-detail") {
      return (
        <LogDetailScreen
          entry={logSelectedEntry()}
          onBack={goBack}
          onCopy={async (entry) => {
            try {
              await copyToClipboard(entry.json);
              showLogNotification("Copied to clipboard.", "success");
            } catch {
              showLogNotification("Failed to copy to clipboard.", "error");
            }
          }}
          notification={logNotification()}
          version={version()}
          helpVisible={helpVisible()}
        />
      );
    }

    if (screen === "profile") {
      if (profileError()) {
        return (
          <ErrorScreen
            error={profileError() as Error}
            onBack={goBack}
            hint="Unable to load profiles."
          />
        );
      }
      return (
        <ProfileScreen
          profiles={profileItems()}
          version={version()}
          helpVisible={helpVisible()}
          onCreate={openProfileCreate}
          onDelete={openProfileDelete}
          onEdit={openProfileEnv}
          onSelect={(profile) => {
            void setActiveProfile(profile.name)
              .then(() => {
                void refreshProfiles();
              })
              .catch((err) => {
                openProfileError(err, "Unable to set active profile.");
              });
          }}
          onBack={goBack}
        />
      );
    }

    if (screen === "profile-env") {
      const entry = selectedProfileConfig();
      if (!entry) {
        return (
          <ErrorScreen
            error="Profile not found."
            onBack={goBack}
            hint="Select a profile before editing."
          />
        );
      }
      return (
        <ProfileEnvScreen
          profileName={entry.name}
          variables={profileEnvVariables()}
          onAdd={openProfileEnvAdd}
          onEdit={openProfileEnvEdit}
          onDelete={openProfileEnvDelete}
          onViewOsEnv={() => navigateTo("profile-os-env")}
          onBack={goBack}
          version={version()}
          helpVisible={helpVisible()}
        />
      );
    }

    if (screen === "profile-input") {
      const mode = profileInputMode();
      const envKey = profileEnvKey();
      const message =
        mode === "create-profile"
          ? "New profile name"
          : mode === "add-env"
            ? "Add environment variable"
            : `Edit value for ${envKey ?? "(unknown)"}`;
      const label =
        mode === "create-profile"
          ? "Profile name"
          : mode === "add-env"
            ? "KEY=VALUE"
            : "Value";
      const placeholder =
        mode === "create-profile"
          ? "development"
          : mode === "add-env"
            ? "MY_VAR=value"
            : undefined;

      return (
        <InputScreen
          message={message}
          value={profileInputValue()}
          onChange={handleProfileInputChange}
          onSubmit={(value) => void submitProfileInput(value)}
          onCancel={() => {
            setProfileInputSuppressKey(null);
            goBack();
          }}
          label={label}
          {...(placeholder !== undefined ? { placeholder } : {})}
          width={32}
          helpVisible={helpVisible()}
        />
      );
    }

    if (screen === "profile-confirm") {
      const mode = profileConfirmMode();
      const profileName = selectedProfileName();
      const envKey = profileEnvKey();
      const message =
        mode === "delete-profile"
          ? `Delete profile ${profileName ?? "(unknown)"}?`
          : `Delete ${envKey ?? "(unknown)"}?`;

      return (
        <ConfirmScreen
          message={message}
          onConfirm={(confirmed) => void confirmProfileAction(confirmed)}
          defaultNo
          helpVisible={helpVisible()}
        />
      );
    }

    if (screen === "profile-os-env") {
      const highlightKeys = profileEnvVariables().map(
        (variable) => variable.key,
      );
      return (
        <EnvironmentScreen
          variables={osEnvVariables()}
          highlightKeys={highlightKeys}
          onBack={goBack}
          version={version()}
          helpVisible={helpVisible()}
        />
      );
    }

    if (screen === "profile-error") {
      return (
        <ErrorScreen
          error={profileActionError() ?? "Profile error"}
          onBack={() => {
            setProfileActionError(null);
            setProfileActionHint(null);
            goBack();
          }}
          {...(profileActionHint()
            ? { hint: profileActionHint() as string }
            : {})}
          helpVisible={helpVisible()}
        />
      );
    }

    if (screen === "worktree-create") {
      const baseBranchRef = resolveBaseBranchRef(creationSource(), null, () =>
        defaultBaseBranch(),
      );
      const baseBranchLabel = resolveBaseBranchLabel(
        creationSource(),
        null,
        () => defaultBaseBranch(),
      );

      return (
        <WorktreeCreateScreen
          branchName={createBranchName()}
          baseBranch={baseBranchLabel}
          version={version()}
          helpVisible={helpVisible()}
          onChange={(value) => {
            const suppressKey = suppressCreateKey();
            if (
              suppressKey &&
              createBranchName() === "" &&
              value === suppressKey
            ) {
              setSuppressCreateKey(null);
              setCreateBranchName("");
              return;
            }
            setSuppressCreateKey(null);
            setCreateBranchName(value);
          }}
          onSubmit={(value, branchType) => {
            const trimmed = value.trim();
            if (!trimmed) {
              return;
            }
            // Add prefix based on selected branch type
            const prefixKey =
              branchType.toUpperCase() as keyof typeof BRANCH_PREFIXES;
            const prefix = BRANCH_PREFIXES[prefixKey] ?? "";
            const fullBranchName = `${prefix}${trimmed}`;

            setSelectedBranch({
              name: fullBranchName,
              displayName: fullBranchName,
              branchType: "local",
              branchCategory: branchType,
            });
            setIsNewBranch(true);
            setNewBranchBaseRef(baseBranchRef);
            setSelectedTool(null);
            setSelectedMode("normal");
            setSuppressCreateKey(null);
            navigateTo("tool-select");
          }}
          onCancel={() => {
            setSuppressCreateKey(null);
            goBack();
          }}
        />
      );
    }

    if (screen === "loading") {
      return (
        <LoadingIndicatorScreen
          message="Loading..."
          delay={props.loadingIndicatorDelay ?? 0}
        />
      );
    }

    if (screen === "error") {
      return (
        <ErrorScreen
          error={error() ?? "Unknown error"}
          helpVisible={helpVisible()}
        />
      );
    }

    return (
      <ErrorScreen
        error={`Unknown screen: ${screen}`}
        onBack={goBack}
        helpVisible={helpVisible()}
      />
    );
  };

  return (
    <>
      {renderCurrentScreen()}
      {unsafeSelectionConfirmVisible() && (
        <box
          position="absolute"
          top="30%"
          left="20%"
          width={unsafeConfirmBoxWidth()}
          zIndex={110}
          border
          borderStyle="single"
          borderColor="yellow"
          backgroundColor="black"
          padding={1}
        >
          <ConfirmScreen
            message={unsafeSelectionMessage()}
            onConfirm={confirmUnsafeSelection}
            yesLabel="OK"
            noLabel="Cancel"
            defaultNo
            helpVisible={helpVisible()}
            width={unsafeConfirmContentWidth()}
          />
        </box>
      )}
      <HelpOverlay visible={helpVisible()} context={currentScreen()} />
      {/* FR-044: ウィザードポップアップをレイヤー表示 */}
      <WizardController
        visible={wizardVisible()}
        selectedBranchName={selectedBranch()?.name ?? ""}
        history={quickStartHistory()}
        onClose={handleWizardClose}
        onComplete={handleWizardComplete}
        onResume={handleWizardResume}
        onStartNew={handleWizardStartNew}
      />
    </>
  );
}
const withTimeout = async <T,>(
  promise: Promise<T>,
  timeoutMs: number,
): Promise<T> =>
  new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error("timeout"));
    }, timeoutMs);

    promise
      .then((value) => {
        clearTimeout(timer);
        resolve(value);
      })
      .catch((err) => {
        clearTimeout(timer);
        reject(err);
      });
  });

const buildBranchList = (
  branches: BranchInfo[],
  worktrees: WorktreeEntry[],
  lastToolUsageMap?: Map<string, ToolSessionEntry>,
) => {
  const localBranchNames = new Set(
    branches.filter((branch) => branch.type === "local").map((b) => b.name),
  );

  const filtered = branches.filter((branch) => {
    if (branch.type === "remote") {
      const remoteName = branch.name.replace(/^origin\//, "");
      return !localBranchNames.has(remoteName);
    }
    return true;
  });

  const worktreeMap = new Map<string, UIWorktreeInfo>();
  for (const worktree of worktrees) {
    worktreeMap.set(worktree.branch, {
      path: worktree.path,
      locked: worktree.locked ?? false,
      prunable: worktree.prunable ?? false,
      isAccessible: worktree.isAccessible ?? true,
      ...(worktree.hasUncommittedChanges !== undefined
        ? { hasUncommittedChanges: worktree.hasUncommittedChanges }
        : {}),
    });
  }

  const enriched = filtered.map((branch) => {
    const lastToolUsage = lastToolUsageMap?.get(branch.name);
    const baseBranch = lastToolUsage ? { ...branch, lastToolUsage } : branch;
    if (branch.type === "local") {
      const worktree = worktreeMap.get(branch.name);
      if (worktree) {
        return { ...baseBranch, worktree };
      }
    }
    return baseBranch;
  });

  const items = formatBranchItems(enriched, worktreeMap);
  return { items, worktreeMap };
};
