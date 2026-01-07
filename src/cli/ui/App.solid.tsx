/** @jsxImportSource @opentui/solid */
import { useKeyboard, useRenderer } from "@opentui/solid";
import {
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
import { EnvironmentScreen } from "./screens/solid/EnvironmentScreen.js";
import { ProfileScreen } from "./screens/solid/ProfileScreen.js";
import { ProfileEnvScreen } from "./screens/solid/ProfileEnvScreen.js";
import { calculateStatistics } from "./utils/statisticsCalculator.js";
import { formatBranchItems } from "./utils/branchFormatter.js";
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
  getMergedPRWorktrees,
  isProtectedBranchName,
  listAdditionalWorktrees,
  removeWorktree,
  repairWorktrees,
  type WorktreeInfo as WorktreeEntry,
} from "../../worktree.js";
import { detectAllToolStatuses, type ToolStatus } from "../../utils/command.js";
import {
  getConfig,
  getLastToolUsageMap,
  loadSession,
  type ToolSessionEntry,
} from "../../config/index.js";
import { getAllCodingAgents } from "../../config/tools.js";
import {
  buildLogFilePath,
  getTodayLogDate,
  readLogFileLines,
  resolveLogDir,
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

export type ExecutionMode = "normal" | "continue" | "resume";

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
  toolStatuses?: ToolStatus[];
}

const DEFAULT_SCREEN: AppScreen = "branch-list";

const buildStats = (branches: BranchItem[]): Statistics =>
  calculateStatistics(branches);

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
    ...(isRemote ? { remoteBranch: branch.name } : {}),
  };
};

const _inferBranchCategory = (branchName: string): BranchItem["branchType"] => {
  const normalized = branchName.replace(/^origin\//, "");
  if (normalized === "main") return "main";
  if (normalized === "develop") return "develop";
  const prefix = normalized.split("/")[0] ?? "";
  switch (prefix) {
    case "feature":
    case "bugfix":
    case "hotfix":
    case "release":
      return prefix;
    default:
      return "other";
  }
};

export function AppSolid(props: AppSolidProps) {
  const renderer = useRenderer();
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

  const [toolItems, setToolItems] = createSignal<SelectorItem[]>([]);
  const [toolError, setToolError] = createSignal<Error | null>(null);
  const [toolStatuses, setToolStatuses] = createSignal<ToolStatus[]>(
    props.toolStatuses ?? [],
  );

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
  const [branchFooterMessage, setBranchFooterMessage] = createSignal<{
    text: string;
    isSpinning?: boolean;
    color?: StatusColor;
  } | null>(null);
  const [branchInputLocked, setBranchInputLocked] = createSignal(false);
  const [cleanupIndicators, setCleanupIndicators] = createSignal<
    Record<string, CleanupIndicator>
  >({});
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

  // セッション履歴（最終使用エージェントなど）
  const [sessionHistory, setSessionHistory] = createSignal<ToolSessionEntry[]>(
    [],
  );

  // 選択中ブランチの履歴をフィルタリング
  const historyForBranch = createMemo(() => {
    const history = sessionHistory();
    const branch = selectedBranch();
    if (!branch) return [];
    // 選択中ブランチにマッチする履歴エントリを新しい順で返す
    return history
      .filter((entry) => entry.branch === branch.name)
      .sort((a, b) => (b.timestamp ?? 0) - (a.timestamp ?? 0));
  });

  const [logEntries, setLogEntries] = createSignal<FormattedLogEntry[]>([]);
  const [logLoading, setLogLoading] = createSignal(false);
  const [logError, setLogError] = createSignal<string | null>(null);
  const [logSelectedEntry, setLogSelectedEntry] =
    createSignal<FormattedLogEntry | null>(null);
  const [logSelectedDate, _setLogSelectedDate] = createSignal<string | null>(
    getTodayLogDate(),
  );
  const [logNotification, setLogNotification] = createSignal<{
    message: string;
    tone: "success" | "error";
  } | null>(null);

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

  const logDir = createMemo(() => resolveLogDir(workingDirectory()));
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
  let logNotificationTimer: ReturnType<typeof setTimeout> | null = null;
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
      const filePath = buildLogFilePath(logDir(), targetDate);
      const lines = await readLogFileLines(filePath);
      const parsed = parseLogLines(lines, { limit: 100 });
      setLogEntries(parsed);
    } catch (err) {
      setLogEntries([]);
      setLogError(err instanceof Error ? err.message : "Failed to load logs");
    } finally {
      setLogLoading(false);
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
      setBranchItems(initial.items);
      setStats(buildStats(initial.items));

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
        setBranchItems(full.items);
        setStats(buildStats(full.items));
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
      setBranchItems(props.branches);
      setStats(props.stats ?? buildStats(props.branches));
      setLoading(false);
    }
  });

  onMount(() => {
    if (!props.branches) {
      void refreshBranches();
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
    if (!props.toolStatuses) {
      void detectAllToolStatuses()
        .then((statuses) => setToolStatuses(statuses))
        .catch(() => setToolStatuses([]));
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

  createEffect(() => {
    if (currentScreen() === "log-list") {
      void loadLogEntries(logSelectedDate());
    }
  });

  onCleanup(() => {
    if (logNotificationTimer) {
      clearTimeout(logNotificationTimer);
    }
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
    });
  };

  // FR-010: クイックスタートからのResume（前回設定で続きから）
  const handleWizardResume = (entry: ToolSessionEntry) => {
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
          if (isProtectedBranchName(branch.name)) {
            skipCounts.protected += 1;
            continue;
          }
          if (branch.isCurrent) {
            skipCounts.current += 1;
            continue;
          }
          const isWarning =
            Boolean(branch.hasUnpushedCommits) || !branch.mergedPR;
          if (isWarning) {
            skipCounts.unsafe += 1;
            continue;
          }
          const worktreePath = branch.worktree?.path ?? null;
          tasks.push({
            branch: branch.name,
            worktreePath,
            cleanupType: worktreePath ? "worktree-and-branch" : "branch-only",
            isAccessible: branch.worktree?.isAccessible,
          });
        }
      } else {
        const targets = await getMergedPRWorktrees();
        for (const target of targets) {
          if (isProtectedBranchName(target.branch)) {
            skipCounts.protected += 1;
            continue;
          }
          if (target.hasUncommittedChanges || target.hasUnpushedCommits) {
            skipCounts.unsafe += 1;
            continue;
          }
          tasks.push({
            branch: target.branch,
            worktreePath: target.worktreePath,
            cleanupType: target.cleanupType,
            isAccessible: target.isAccessible,
          });
        }
      }

      const skipNotice = buildSkipNotice(
        skipCounts,
        hasSelection
          ? "with unpushed commits or unmerged PRs"
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
    const targets = branchItems()
      .filter((branch) => branch.worktreeStatus === "inaccessible")
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
    setSelectedBranches((prev) => {
      const next = new Set(prev);
      if (next.has(branchName)) {
        next.delete(branchName);
      } else {
        next.add(branchName);
      }
      return Array.from(next);
    });
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
        inputLocked: branchInputLocked(),
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
          toolStatuses={toolStatuses()}
          activeProfile={activeProfile()}
          onOpenLogs={() => navigateTo("log-list")}
          onOpenProfiles={() => navigateTo("profile")}
          selectedBranches={selectedBranches()}
          onToggleSelect={toggleSelectedBranch}
          onCreateBranch={handleQuickCreate}
          cleanupUI={cleanupUI}
          helpVisible={helpVisible()}
          wizardVisible={wizardVisible()}
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
          notification={logNotification()}
          version={version()}
          selectedDate={logSelectedDate()}
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
      <HelpOverlay visible={helpVisible()} context={currentScreen()} />
      {/* FR-044: ウィザードポップアップをレイヤー表示 */}
      <WizardController
        visible={wizardVisible()}
        selectedBranchName={selectedBranch()?.name ?? ""}
        history={historyForBranch()}
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
