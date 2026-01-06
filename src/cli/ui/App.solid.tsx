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
  WorktreeInfo,
} from "./types.js";
import type { FormattedLogEntry } from "../../logging/formatter.js";
import { BranchListScreen } from "./screens/solid/BranchListScreen.js";
import { HelpOverlay } from "./components/solid/HelpOverlay.js";
import {
  SelectorScreen,
  type SelectorItem,
} from "./screens/solid/SelectorScreen.js";
import { LogScreen } from "./screens/solid/LogScreen.js";
import { LogDetailScreen } from "./screens/solid/LogDetailScreen.js";
import { ErrorScreen } from "./screens/solid/ErrorScreen.js";
import { LoadingIndicatorScreen } from "./screens/solid/LoadingIndicator.js";
import { WorktreeCreateScreen } from "./screens/solid/WorktreeCreateScreen.js";
import { ProfileScreen } from "./screens/solid/ProfileScreen.js";
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
} from "../../git.js";
import { listAdditionalWorktrees } from "../../worktree.js";
import { detectAllToolStatuses, type ToolStatus } from "../../utils/command.js";
import { getConfig } from "../../config/index.js";
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
import { loadProfiles, setActiveProfile } from "../../config/profiles.js";

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
  | "worktree-create"
  | "loading"
  | "error";

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

const inferBranchCategory = (branchName: string): BranchItem["branchType"] => {
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

  const logDir = createMemo(() => resolveLogDir(workingDirectory()));
  let logNotificationTimer: ReturnType<typeof setTimeout> | null = null;
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
    setCurrentScreen(previous);
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

      const localBranches = await withTimeout(
        localBranchesPromise,
        BRANCH_LOAD_TIMEOUT_MS,
      ).catch(() => []);
      const currentBranch = await withTimeout(
        currentBranchPromise,
        BRANCH_LOAD_TIMEOUT_MS,
      ).catch(() => null);

      if (currentBranch) {
        localBranches.forEach((branch) => {
          if (branch.name === currentBranch) {
            branch.isCurrent = true;
          }
        });
      }

      const worktrees = await withTimeout(
        worktreesPromise,
        BRANCH_LOAD_TIMEOUT_MS,
      ).catch(() => []);

      const initial = buildBranchList(localBranches, worktrees);
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

        const full = buildBranchList(branches, latestWorktrees);
        setBranchItems(full.items);
        setStats(buildStats(full.items));
      })();
    } catch (err) {
      setError(err instanceof Error ? err : new Error(String(err)));
    } finally {
      setLoading(false);
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
    void loadProfiles()
      .then((config) => {
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
      })
      .catch((err) => {
        setProfileItems([]);
        setActiveProfileName(null);
        setProfileError(err instanceof Error ? err : new Error(String(err)));
      });
  });

  onMount(() => {
    void getConfig()
      .then((config) => setDefaultBaseBranch(config.defaultBaseBranch))
      .catch(() => setDefaultBaseBranch("main"));
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
  });

  const handleBranchSelect = (branch: BranchItem) => {
    setSelectedBranch(toSelectedBranchState(branch));
    setIsNewBranch(false);
    setNewBranchBaseRef(null);
    setCreationSource(null);
    setSelectedTool(null);
    setSelectedMode("normal");
    navigateTo("tool-select");
  };

  const handleQuickCreate = (branch: BranchItem | null) => {
    setCreationSource(branch ? toSelectedBranchState(branch) : null);
    setCreateBranchName("");
    setSuppressCreateKey("n");
    navigateTo("worktree-create");
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
      return (
        <BranchListScreen
          branches={branchItems()}
          stats={stats()}
          onSelect={handleBranchSelect}
          onQuit={() => exitApp(undefined)}
          onRefresh={refreshBranches}
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
          helpVisible={helpVisible()}
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
          onSelect={(profile) => {
            void setActiveProfile(profile.name)
              .then(() => {
                setActiveProfileName(profile.name);
                setProfileItems((prev) =>
                  prev.map((item) => ({
                    ...item,
                    isActive: item.name === profile.name,
                  })),
                );
              })
              .catch((err) => {
                setProfileError(
                  err instanceof Error ? err : new Error(String(err)),
                );
              });
          }}
          onBack={goBack}
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
          onSubmit={(value) => {
            const trimmed = value.trim();
            if (!trimmed) {
              return;
            }
            setSelectedBranch({
              name: trimmed,
              displayName: trimmed,
              branchType: "local",
              branchCategory: inferBranchCategory(trimmed),
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

const buildBranchList = (branches: BranchInfo[], worktrees: WorktreeInfo[]) => {
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

  const worktreeMap = new Map<string, WorktreeInfo>();
  for (const worktree of worktrees) {
    worktreeMap.set(worktree.branch, {
      path: worktree.path,
      locked: false,
      prunable: worktree.isAccessible === false,
      isAccessible: worktree.isAccessible ?? true,
      ...(worktree.hasUncommittedChanges !== undefined
        ? { hasUncommittedChanges: worktree.hasUncommittedChanges }
        : {}),
    });
  }

  const enriched = filtered.map((branch) => {
    if (branch.type === "local") {
      const worktree = worktreeMap.get(branch.name);
      if (worktree) {
        return { ...branch, worktree };
      }
    }
    return branch;
  });

  const items = formatBranchItems(enriched, worktreeMap);
  return { items, worktreeMap };
};
