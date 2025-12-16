import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useApp } from "ink";
import { ErrorBoundary } from "./common/ErrorBoundary.js";
import {
  BranchListScreen,
  type BranchListScreenProps,
} from "./screens/BranchListScreen.js";
import { BranchCreatorScreen } from "./screens/BranchCreatorScreen.js";
import { BranchActionSelectorScreen } from "../screens/BranchActionSelectorScreen.js";
import { AIToolSelectorScreen } from "./screens/AIToolSelectorScreen.js";
import { ExecutionModeSelectorScreen } from "./screens/ExecutionModeSelectorScreen.js";
import type { ExecutionMode } from "./screens/ExecutionModeSelectorScreen.js";
import { BranchQuickStartScreen } from "./screens/BranchQuickStartScreen.js";
import type { QuickStartAction } from "./screens/BranchQuickStartScreen.js";
import {
  ModelSelectorScreen,
  type ModelSelectionResult,
} from "./screens/ModelSelectorScreen.js";
import { EnvironmentProfileScreen } from "./screens/EnvironmentProfileScreen.js";
import { useGitData } from "../hooks/useGitData.js";
import { useProfiles } from "../hooks/useProfiles.js";
import { useScreenState } from "../hooks/useScreenState.js";
import { formatBranchItems } from "../utils/branchFormatter.js";
import { calculateStatistics } from "../utils/statisticsCalculator.js";
import type {
  AITool,
  BranchInfo,
  BranchItem,
  InferenceLevel,
  SelectedBranchState,
} from "../types.js";
import { getRepositoryRoot, deleteBranch } from "../../../git.js";
import { loadSession } from "../../../config/index.js";
import {
  createWorktree,
  generateWorktreePath,
  getMergedPRWorktrees,
  isProtectedBranchName,
  removeWorktree,
  switchToProtectedBranch,
} from "../../../worktree.js";
import { getPackageVersion } from "../../../utils.js";
import {
  resolveBaseBranchLabel,
  resolveBaseBranchRef,
} from "../utils/baseBranch.js";
import {
  getDefaultInferenceForModel,
  getDefaultModelOption,
} from "../utils/modelOptions.js";
import {
  resolveContinueSessionId,
  findLatestBranchSessionsByTool,
} from "../utils/continueSession.js";
import {
  findLatestCodexSession,
  findLatestCodexSessionId,
  findLatestClaudeSession,
  findLatestGeminiSession,
} from "../../../utils/session.js";
import type { ToolSessionEntry } from "../../../config/index.js";

const SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
const COMPLETION_HOLD_DURATION_MS = 3000;
const PROTECTED_BRANCH_WARNING =
  "Root branches operate directly in the repository root. Create a new branch if you need a dedicated worktree.";

const getSpinnerFrame = (index: number): string => {
  const frame = SPINNER_FRAMES[index];
  if (typeof frame === "string") {
    return frame;
  }
  return SPINNER_FRAMES[0] ?? "⠋";
};

export interface SelectionResult {
  branch: string; // Local branch name (without remote prefix)
  displayName: string; // Name that was selected in the UI (may include remote prefix)
  branchType: "local" | "remote";
  remoteBranch?: string; // Full remote ref when branchType === 'remote'
  tool: AITool;
  mode: ExecutionMode;
  skipPermissions: boolean;
  model?: string | null;
  inferenceLevel?: InferenceLevel;
  sessionId?: string | null;
}

export interface AppProps {
  onExit: (result?: SelectionResult) => void;
  loadingIndicatorDelay?: number;
}

/**
 * App - Top-level component for Ink.js UI
 * Integrates ErrorBoundary, data fetching, screen navigation, and all screens
 */
export function App({ onExit, loadingIndicatorDelay = 300 }: AppProps) {
  const { exit } = useApp();

  // 起動ディレクトリの取得
  const workingDirectory = process.cwd();

  const { branches, worktrees, loading, error, refresh, lastUpdated } =
    useGitData({
      enableAutoRefresh: false, // Manual refresh with 'r' key
    });
  const { currentScreen, navigateTo, goBack } = useScreenState();

  // Profile state
  const { activeProfileName, refresh: refreshProfiles } = useProfiles();

  // Version state
  const [version, setVersion] = useState<string | null>(null);
  const [repoRoot, setRepoRoot] = useState<string | null>(null);
  const [continueSessionId, setContinueSessionId] = useState<string | null>(
    null,
  );
  const [branchQuickStart, setBranchQuickStart] = useState<
    {
      toolId: AITool;
      toolLabel: string;
      model?: string | null;
      sessionId?: string | null;
      inferenceLevel?: InferenceLevel | null;
      skipPermissions?: boolean | null;
      timestamp?: number | null;
    }[]
  >([]);
  const [branchQuickStartLoading, setBranchQuickStartLoading] = useState(false);

  // Selection state (for branch → tool → mode flow)
  const [selectedBranch, setSelectedBranch] =
    useState<SelectedBranchState | null>(null);
  const [creationSourceBranch, setCreationSourceBranch] =
    useState<SelectedBranchState | null>(null);
  const [selectedTool, setSelectedTool] = useState<AITool | null>(null);
  const [selectedModel, setSelectedModel] =
    useState<ModelSelectionResult | null>(null);
  const [lastModelByTool, setLastModelByTool] = useState<
    Record<AITool, ModelSelectionResult | undefined>
  >({});
  const [preferredToolId, setPreferredToolId] = useState<AITool | null>(null);

  // PR cleanup feedback
  const [cleanupIndicators, setCleanupIndicators] = useState<
    Record<
      string,
      { icon: string; color?: "cyan" | "green" | "yellow" | "red" }
    >
  >({});
  const [cleanupProcessingBranch, setCleanupProcessingBranch] = useState<
    string | null
  >(null);
  const [cleanupInputLocked, setCleanupInputLocked] = useState(false);
  const [cleanupFooterMessage, setCleanupFooterMessage] = useState<{
    text: string;
    color?: "cyan" | "green" | "yellow" | "red";
  } | null>(null);
  const [hiddenBranches, setHiddenBranches] = useState<string[]>([]);
  const [selectedBranches, setSelectedBranches] = useState<string[]>([]);
  const [safeBranches, setSafeBranches] = useState<Set<string>>(new Set());
  const spinnerFrameIndexRef = useRef(0);
  const [spinnerFrameIndex, setSpinnerFrameIndex] = useState(0);
  const completionTimerRef = useRef<NodeJS.Timeout | null>(null);

  // Fetch version on mount
  useEffect(() => {
    getPackageVersion()
      .then(setVersion)
      .catch(() => setVersion(null));
  }, []);

  // Fetch repository root once for session lookups
  useEffect(() => {
    getRepositoryRoot()
      .then(setRepoRoot)
      .catch(() => setRepoRoot(null));
  }, []);

  useEffect(() => {
    if (!cleanupInputLocked) {
      spinnerFrameIndexRef.current = 0;
      setSpinnerFrameIndex(0);
      return undefined;
    }

    const interval = setInterval(() => {
      spinnerFrameIndexRef.current =
        (spinnerFrameIndexRef.current + 1) % SPINNER_FRAMES.length;
      setSpinnerFrameIndex(spinnerFrameIndexRef.current);
    }, 120);

    return () => {
      clearInterval(interval);
      spinnerFrameIndexRef.current = 0;
      setSpinnerFrameIndex(0);
    };
  }, [cleanupInputLocked]);

  useEffect(() => {
    if (!cleanupInputLocked) {
      return;
    }

    const frame = getSpinnerFrame(spinnerFrameIndex);

    if (cleanupProcessingBranch) {
      setCleanupIndicators((prev) => {
        const current = prev[cleanupProcessingBranch];
        if (current && current.icon === frame && current.color === "cyan") {
          return prev;
        }

        const next: Record<
          string,
          { icon: string; color?: "cyan" | "green" | "yellow" | "red" }
        > = {
          ...prev,
          [cleanupProcessingBranch]: { icon: frame, color: "cyan" },
        };

        return next;
      });
    }

    setCleanupFooterMessage({ text: `Processing... ${frame}`, color: "cyan" });
  }, [cleanupInputLocked, cleanupProcessingBranch, spinnerFrameIndex]);

  useEffect(() => {
    if (!hiddenBranches.length) {
      return;
    }

    const existing = new Set(branches.map((branch) => branch.name));
    const filtered = hiddenBranches.filter((name) => existing.has(name));

    if (filtered.length !== hiddenBranches.length) {
      setHiddenBranches(filtered);
    }
  }, [branches, hiddenBranches]);

  // Remove selections that no longer exist (hidden or disappeared)
  useEffect(() => {
    setSelectedBranches((prev) =>
      prev.filter(
        (name) =>
          branches.some((b) => b.name === name) &&
          !hiddenBranches.includes(name),
      ),
    );
  }, [branches, hiddenBranches]);

  // Precompute safe-to-clean branches using cleanup candidate logic
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const targets = await getMergedPRWorktrees();
        if (cancelled) return;
        const safe = new Set(
          targets
            .filter((t) => !t.hasUncommittedChanges && !t.hasUnpushedCommits)
            .map((t) => t.branch),
        );
        setSafeBranches(safe);
      } catch {
        if (cancelled) return;
        setSafeBranches(new Set());
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [branches, worktrees]);

  // Load quick start options for selected branch (latest per tool)
  useEffect(() => {
    if (!selectedBranch) {
      setBranchQuickStart([]);
      return;
    }
    let cancelled = false;
    setBranchQuickStartLoading(true);
    (async () => {
      try {
        const root = repoRoot ?? (await getRepositoryRoot());
        if (!repoRoot && root) {
          setRepoRoot(root);
        }
        if (!root) {
          if (!cancelled) setBranchQuickStart([]);
          return;
        }
        const sessionData = await loadSession(root);
        const history = sessionData?.history ?? [];

        const combinedHistory = [...history];
        if (
          sessionData?.lastSessionId &&
          sessionData.lastBranch === selectedBranch.name &&
          sessionData.lastUsedTool
        ) {
          const synthetic: ToolSessionEntry = {
            branch: sessionData.lastBranch,
            worktreePath: sessionData.lastWorktreePath ?? null,
            toolId: sessionData.lastUsedTool,
            toolLabel: sessionData.toolLabel ?? sessionData.lastUsedTool,
            sessionId: sessionData.lastSessionId,
            mode: sessionData.mode ?? null,
            model: sessionData.model ?? null,
            reasoningLevel: sessionData.reasoningLevel ?? null,
            skipPermissions: sessionData.skipPermissions ?? null,
            timestamp: sessionData.timestamp ?? Date.now(),
          };
          combinedHistory.push(synthetic);
        }
        const latestPerTool = findLatestBranchSessionsByTool(
          combinedHistory,
          selectedBranch.name,
          selectedWorktreePath,
        );

        const mapped = await Promise.all(
          latestPerTool.map(async (entry) => {
            let sessionId = entry.sessionId ?? null;
            const worktree = selectedWorktreePath ?? workingDirectory;

            // For Codex, prefer a newer filesystem session over stale history
            if (!sessionId && entry.toolId === "codex-cli") {
              try {
                const historyTs = entry.timestamp ?? null;
                const latestCodex = await findLatestCodexSession({
                  ...(historyTs
                    ? {
                        since: historyTs - 60_000,
                        preferClosestTo: historyTs,
                        windowMs: 60 * 60 * 1000,
                      }
                    : {}),
                  cwd: worktree,
                });
                sessionId =
                  latestCodex?.id ??
                  (await findLatestCodexSessionId({ cwd: worktree })) ??
                  null;
              } catch {
                // ignore lookup failure
              }
            }

            // For Claude Code, prefer the newest session file in the worktree even if history is stale.
            if (!sessionId && entry.toolId === "claude-code") {
              try {
                // Always resolve freshest on-disk session for this worktree (no window restriction)
                const latestAny = await findLatestClaudeSession(worktree);
                sessionId = latestAny?.id ?? null;
              } catch {
                // ignore lookup failure
              }
            }

            // For Gemini, prefer newest session file (Gemini keeps per-project chats)
            if (!sessionId && entry.toolId === "gemini-cli") {
              try {
                const gemOptions: Parameters<
                  typeof findLatestGeminiSession
                >[0] = {
                  windowMs: 60 * 60 * 1000,
                  cwd: worktree,
                };
                if (entry.timestamp !== null && entry.timestamp !== undefined) {
                  gemOptions.since = entry.timestamp - 60_000;
                  gemOptions.preferClosestTo = entry.timestamp;
                }
                const gemSession = await findLatestGeminiSession(gemOptions);
                sessionId = gemSession?.id ?? null;
              } catch {
                // ignore
              }
            }

            return {
              toolId: entry.toolId as AITool,
              toolLabel: entry.toolLabel,
              model: entry.model ?? null,
              inferenceLevel: (entry.reasoningLevel ??
                sessionData?.reasoningLevel ??
                null) as InferenceLevel | null,
              sessionId,
              skipPermissions:
                entry.skipPermissions ?? sessionData?.skipPermissions ?? null,
              timestamp: entry.timestamp ?? null,
            };
          }),
        );

        if (!cancelled) {
          setBranchQuickStart(mapped);
        }
      } catch {
        if (!cancelled) setBranchQuickStart([]);
      } finally {
        if (!cancelled) setBranchQuickStartLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [selectedBranch, repoRoot]);

  // Load last session ID for "Continue" label when entering execution mode selector
  useEffect(() => {
    if (currentScreen !== "execution-mode-selector") {
      return;
    }
    if (!selectedTool || !selectedBranch) {
      setContinueSessionId(null);
      return;
    }
    (async () => {
      try {
        const root = repoRoot ?? (await getRepositoryRoot());
        if (!repoRoot && root) {
          setRepoRoot(root);
        }
        const sessionData = root ? await loadSession(root) : null;
        const history = sessionData?.history ?? [];

        const found = await resolveContinueSessionId({
          history,
          sessionData,
          branch: selectedBranch.name,
          toolId: selectedTool,
          repoRoot: root,
        });

        setContinueSessionId(found ?? null);
      } catch {
        setContinueSessionId(null);
      }
    })();
  }, [currentScreen, selectedTool, selectedBranch, repoRoot]);

  // Update preferred tool when branch or data changes
  useEffect(() => {
    if (!selectedBranch) return;
    const branchMatch =
      branches.find((b) => b.name === selectedBranch.name) ||
      branches.find(
        (b) =>
          selectedBranch.branchType === "remote" &&
          b.name === selectedBranch.displayName,
      );
    setPreferredToolId(branchMatch?.lastToolUsage?.toolId ?? null);
  }, [branches, selectedBranch]);

  useEffect(
    () => () => {
      if (completionTimerRef.current) {
        clearTimeout(completionTimerRef.current);
        completionTimerRef.current = null;
      }
    },
    [],
  );

  const visibleBranches = useMemo(
    () => branches.filter((branch) => !hiddenBranches.includes(branch.name)),
    [branches, hiddenBranches],
  );

  const selectedWorktreePath = useMemo(() => {
    if (!selectedBranch) return null;
    const wt = worktrees.find((w) => w.branch === selectedBranch.name);
    return wt?.path ?? null;
  }, [selectedBranch, worktrees]);

  // Helper function to create content-based hash for branches
  const branchHash = useMemo(
    () =>
      visibleBranches
        .map((b) => `${b.name}-${b.type}-${b.isCurrent}`)
        .join(","),
    [visibleBranches],
  );

  // Helper function to create content-based hash for worktrees
  const worktreeHash = useMemo(
    () => worktrees.map((w) => `${w.branch}-${w.path}`).join(","),
    [worktrees],
  );

  // Format branches to BranchItems (memoized for performance with content-based dependencies)
  const branchItems: BranchItem[] = useMemo(() => {
    // Build worktreeMap for sorting
    const worktreeMap = new Map();
    for (const wt of worktrees) {
      worktreeMap.set(wt.branch, {
        path: wt.path,
        locked: false,
        prunable: wt.isAccessible === false,
        isAccessible: wt.isAccessible ?? true,
        ...(wt.hasUncommittedChanges !== undefined
          ? { hasUncommittedChanges: wt.hasUncommittedChanges }
          : {}),
      });
    }
    const baseItems = formatBranchItems(visibleBranches, worktreeMap);
    return baseItems.map((item) => ({
      ...item,
      safeToCleanup: safeBranches.has(item.name),
    }));
  }, [branchHash, worktreeHash, visibleBranches, worktrees, safeBranches]);

  const selectedBranchSet = useMemo(
    () => new Set(selectedBranches),
    [selectedBranches],
  );

  // Calculate statistics (memoized for performance)
  const stats = useMemo(
    () => calculateStatistics(visibleBranches),
    [visibleBranches],
  );

  const resolveBaseBranch = useCallback(() => {
    const localMain = branches.find(
      (branch) =>
        branch.type === "local" &&
        (branch.name === "main" || branch.name === "master"),
    );
    if (localMain) {
      return localMain.name;
    }

    const develop = branches.find(
      (branch) =>
        branch.type === "local" &&
        (branch.name === "develop" || branch.name === "dev"),
    );
    if (develop) {
      return develop.name;
    }

    return "main";
  }, [branches]);

  const baseBranchLabel = useMemo(
    () =>
      resolveBaseBranchLabel(
        creationSourceBranch,
        selectedBranch,
        resolveBaseBranch,
      ),
    [creationSourceBranch, resolveBaseBranch, selectedBranch],
  );

  // Handle branch selection
  const toLocalBranchName = useCallback((remoteName: string) => {
    const segments = remoteName.split("/");
    if (segments.length <= 1) {
      return remoteName;
    }
    return segments.slice(1).join("/");
  }, []);

  const inferBranchCategory = useCallback(
    (branchName: string): BranchInfo["branchType"] => {
      const matched = branches.find((branch) => branch.name === branchName);
      if (matched) {
        return matched.branchType;
      }
      if (branchName === "main" || branchName === "master") {
        return "main";
      }
      if (branchName === "develop" || branchName === "dev") {
        return "develop";
      }
      if (branchName.startsWith("feature/")) {
        return "feature";
      }
      if (branchName.startsWith("hotfix/")) {
        return "hotfix";
      }
      if (branchName.startsWith("release/")) {
        return "release";
      }
      return "other";
    },
    [branches],
  );

  const isProtectedSelection = useCallback(
    (branch: SelectedBranchState | null): boolean => {
      if (!branch) {
        return false;
      }
      return (
        isProtectedBranchName(branch.name) ||
        isProtectedBranchName(branch.displayName) ||
        (branch.remoteBranch
          ? isProtectedBranchName(branch.remoteBranch)
          : false) ||
        branch.branchCategory === "main" ||
        branch.branchCategory === "develop"
      );
    },
    [isProtectedBranchName],
  );

  const toggleBranchSelection = useCallback((branchName: string) => {
    setSelectedBranches((prev) => {
      const set = new Set(prev);
      if (set.has(branchName)) {
        set.delete(branchName);
      } else {
        set.add(branchName);
      }
      return Array.from(set);
    });
  }, []);

  const protectedBranchInfo = useMemo(() => {
    if (!selectedBranch) {
      return null;
    }
    if (!isProtectedSelection(selectedBranch)) {
      return null;
    }
    const label = selectedBranch.displayName ?? selectedBranch.name;
    return {
      label,
      message: `${label} is a root branch. Switch within the repository root instead of creating a worktree.`,
    };
  }, [selectedBranch, isProtectedSelection]);

  const handleSelect = useCallback(
    (item: BranchItem) => {
      const selection: SelectedBranchState =
        item.type === "remote"
          ? {
              name: toLocalBranchName(item.name),
              displayName: item.name,
              branchType: "remote",
              branchCategory: item.branchType,
              remoteBranch: item.name,
            }
          : {
              name: item.name,
              displayName: item.name,
              branchType: "local",
              branchCategory: item.branchType,
            };

      const protectedSelected = isProtectedSelection(selection);

      setSelectedBranch(selection);
      setSelectedTool(null);
      setSelectedModel(null);
      setCreationSourceBranch(null);
      setPreferredToolId(item.lastToolUsage?.toolId ?? null);

      if (protectedSelected) {
        setCleanupFooterMessage({
          text: PROTECTED_BRANCH_WARNING,
          color: "yellow",
        });
      } else {
        setCleanupFooterMessage(null);
      }

      navigateTo("branch-action-selector");
    },
    [
      isProtectedSelection,
      navigateTo,
      setCleanupFooterMessage,
      setCreationSourceBranch,
      setSelectedTool,
      toLocalBranchName,
    ],
  );

  // Handle branch action selection
  const handleProtectedBranchSwitch = useCallback(async () => {
    if (!selectedBranch) {
      return;
    }

    try {
      setCleanupFooterMessage({
        text: `Preparing root branch '${selectedBranch.displayName ?? selectedBranch.name}'...`,
        color: "cyan",
      });
      const repoRoot = await getRepositoryRoot();
      const remoteRef =
        selectedBranch.remoteBranch ??
        (selectedBranch.branchType === "remote"
          ? (selectedBranch.displayName ?? selectedBranch.name)
          : null);

      const result = await switchToProtectedBranch({
        branchName: selectedBranch.name,
        repoRoot,
        remoteRef: remoteRef ?? null,
      });

      let successMessage = `'${selectedBranch.displayName ?? selectedBranch.name}' will use the repository root.`;
      if (result === "remote") {
        successMessage = `Created a local tracking branch for '${selectedBranch.displayName ?? selectedBranch.name}' and switched to the protected branch.`;
      } else if (result === "local") {
        successMessage = `Checked out '${selectedBranch.displayName ?? selectedBranch.name}' in the repository root.`;
      }

      setCleanupFooterMessage({
        text: successMessage,
        color: "green",
      });
      refresh();
      const nextScreen =
        branchQuickStart.length || branchQuickStartLoading
          ? "branch-quick-start"
          : "ai-tool-selector";
      navigateTo(nextScreen);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setCleanupFooterMessage({
        text: `Failed to switch root branch: ${message}`,
        color: "red",
      });
      console.error("Failed to switch protected branch:", error);
    }
  }, [
    branchQuickStart,
    branchQuickStartLoading,
    navigateTo,
    refresh,
    selectedBranch,
    setCleanupFooterMessage,
  ]);

  const handleUseExistingBranch = useCallback(() => {
    if (selectedBranch && isProtectedSelection(selectedBranch)) {
      void handleProtectedBranchSwitch();
      return;
    }
    if (branchQuickStart.length) {
      navigateTo("branch-quick-start");
    } else {
      navigateTo("ai-tool-selector");
    }
  }, [
    handleProtectedBranchSwitch,
    isProtectedSelection,
    navigateTo,
    branchQuickStart.length,
    selectedBranch,
  ]);

  const handleCreateNewBranch = useCallback(() => {
    setCreationSourceBranch(selectedBranch);
    navigateTo("branch-creator");
  }, [navigateTo, selectedBranch]);

  // Handle quit
  const handleQuit = useCallback(() => {
    onExit();
    exit();
  }, [onExit, exit]);

  // Handle branch creation
  const handleCreate = useCallback(
    async (branchName: string) => {
      try {
        const repoRoot = await getRepositoryRoot();
        const worktreePath = await generateWorktreePath(repoRoot, branchName);
        // Use selectedBranch as base if available, otherwise resolve from repo
        const baseBranch = resolveBaseBranchRef(
          creationSourceBranch,
          selectedBranch,
          resolveBaseBranch,
        );

        await createWorktree({
          branchName,
          worktreePath,
          repoRoot,
          isNewBranch: true,
          baseBranch,
        });

        refresh();
        setCreationSourceBranch(null);
        setSelectedBranch({
          name: branchName,
          displayName: branchName,
          branchType: "local",
          branchCategory: inferBranchCategory(branchName),
        });
        setSelectedTool(null);
        setSelectedModel(null);
        setPreferredToolId(null);
        setCleanupFooterMessage(null);

        navigateTo("ai-tool-selector");
      } catch (error) {
        // On error, go back to branch list
        console.error("Failed to create branch:", error);
        goBack();
        refresh();
      }
    },
    [
      navigateTo,
      goBack,
      refresh,
      resolveBaseBranch,
      selectedBranch,
      creationSourceBranch,
      inferBranchCategory,
      setCleanupFooterMessage,
    ],
  );

  const handleCleanupCommand = useCallback(async () => {
    if (cleanupInputLocked) {
      return;
    }

    if (completionTimerRef.current) {
      clearTimeout(completionTimerRef.current);
      completionTimerRef.current = null;
    }

    const succeededBranches: string[] = [];

    const resetAfterWait = () => {
      setCleanupIndicators({});
      setCleanupInputLocked(false);
      setCleanupFooterMessage(null);
      if (succeededBranches.length > 0) {
        setHiddenBranches((prev) => {
          const merged = new Set(prev);
          succeededBranches.forEach((branch) => merged.add(branch));
          return Array.from(merged);
        });
        setSelectedBranches((prev) =>
          prev.filter((name) => !succeededBranches.includes(name)),
        );
      }
      refresh();
      completionTimerRef.current = null;
    };

    // Provide immediate feedback before fetching targets
    setCleanupInputLocked(true);
    setCleanupIndicators({});
    const initialFrame = getSpinnerFrame(0);
    setCleanupFooterMessage({
      text: `Processing... ${initialFrame}`,
      color: "cyan",
    });
    setCleanupProcessingBranch(null);
    spinnerFrameIndexRef.current = 0;
    setSpinnerFrameIndex(0);

    let targets;
    try {
      targets = await getMergedPRWorktrees();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setCleanupIndicators({});
      setCleanupFooterMessage({ text: `❌ ${message}`, color: "red" });
      setCleanupInputLocked(false);
      completionTimerRef.current = setTimeout(() => {
        setCleanupFooterMessage(null);
        completionTimerRef.current = null;
      }, COMPLETION_HOLD_DURATION_MS);
      return;
    }

    if (targets.length === 0) {
      setCleanupIndicators({});
      setCleanupFooterMessage({
        text: "✅ Nothing to clean up.",
        color: "green",
      });
      setCleanupInputLocked(false);
      completionTimerRef.current = setTimeout(() => {
        setCleanupFooterMessage(null);
        completionTimerRef.current = null;
      }, COMPLETION_HOLD_DURATION_MS);
      return;
    }

    // Manual selection: restrict targets when選択がある
    if (selectedBranchSet.size > 0) {
      targets = targets.filter((t) => selectedBranchSet.has(t.branch));
      if (targets.length === 0) {
        setCleanupIndicators({});
        setCleanupFooterMessage({
          text: "⚠️ No cleanup candidates among selected branches.",
          color: "yellow",
        });
        setCleanupInputLocked(false);
        completionTimerRef.current = setTimeout(() => {
          setCleanupFooterMessage(null);
          completionTimerRef.current = null;
        }, COMPLETION_HOLD_DURATION_MS);
        return;
      }
    }

    // Reset hidden branches that may already be gone
    setHiddenBranches((prev) =>
      prev.filter(
        (name) => targets.find((t) => t.branch === name) === undefined,
      ),
    );

    const initialIndicators = targets.reduce<
      Record<
        string,
        { icon: string; color?: "cyan" | "green" | "yellow" | "red" }
      >
    >((acc, target, index) => {
      const icon = index === 0 ? getSpinnerFrame(0) : "⏳";
      const color: "cyan" | "green" | "yellow" | "red" =
        index === 0 ? "cyan" : "yellow";
      acc[target.branch] = { icon, color };
      return acc;
    }, {});

    setCleanupIndicators(initialIndicators);
    const firstTarget = targets.length > 0 ? targets[0] : undefined;
    setCleanupProcessingBranch(firstTarget ? firstTarget.branch : null);
    spinnerFrameIndexRef.current = 0;
    setSpinnerFrameIndex(0);
    setCleanupFooterMessage({
      text: `Processing... ${getSpinnerFrame(0)}`,
      color: "cyan",
    });

    for (let index = 0; index < targets.length; index += 1) {
      const currentTarget = targets[index];
      if (!currentTarget) {
        continue;
      }
      const target = currentTarget;

      setCleanupProcessingBranch(target.branch);
      spinnerFrameIndexRef.current = 0;
      setSpinnerFrameIndex(0);

      setCleanupIndicators((prev) => {
        const updated = { ...prev };
        updated[target.branch] = { icon: getSpinnerFrame(0), color: "cyan" };
        for (const pending of targets.slice(index + 1)) {
          const current = updated[pending.branch];
          if (!current || current.icon !== "⏳") {
            updated[pending.branch] = { icon: "⏳", color: "yellow" };
          }
        }
        return updated;
      });

      const shouldSkip =
        target.hasUncommittedChanges ||
        target.hasUnpushedCommits ||
        (target.cleanupType === "worktree-and-branch" &&
          (!target.worktreePath || target.isAccessible === false));

      if (shouldSkip) {
        setCleanupIndicators((prev) => ({
          ...prev,
          [target.branch]: { icon: "⏭️", color: "yellow" },
        }));
        setCleanupProcessingBranch(null);
        continue;
      }

      try {
        if (
          target.cleanupType === "worktree-and-branch" &&
          target.worktreePath
        ) {
          await removeWorktree(target.worktreePath, true);
        }

        await deleteBranch(target.branch, true);

        // 自動クリーンアップではリモートブランチは削除しない
        // リモートブランチはユーザーが明示的に削除する必要がある

        succeededBranches.push(target.branch);
        setCleanupIndicators((prev) => ({
          ...prev,
          [target.branch]: { icon: "✅", color: "green" },
        }));
      } catch {
        const icon = "❌";
        setCleanupIndicators((prev) => ({
          ...prev,
          [target.branch]: { icon, color: "red" },
        }));
      }

      setCleanupProcessingBranch(null);
    }

    setCleanupProcessingBranch(null);
    setCleanupInputLocked(false);
    setCleanupFooterMessage({
      text: "Cleanup completed. Finalizing...",
      color: "green",
    });

    const holdDuration =
      typeof process !== "undefined" && process.env?.NODE_ENV === "test"
        ? 0
        : COMPLETION_HOLD_DURATION_MS;

    completionTimerRef.current = setTimeout(resetAfterWait, holdDuration);
  }, [
    cleanupInputLocked,
    deleteBranch,
    getMergedPRWorktrees,
    refresh,
    removeWorktree,
    selectedBranchSet,
  ]);

  // Handle AI tool selection
  const handleToolSelect = useCallback(
    (tool: AITool) => {
      setSelectedTool(tool);
      setSelectedModel(lastModelByTool[tool] ?? null);
      navigateTo("model-selector");
    },
    [lastModelByTool, navigateTo],
  );

  const handleModelSelect = useCallback(
    (selection: ModelSelectionResult) => {
      setSelectedModel(selection);
      setLastModelByTool((prev) => ({
        ...prev,
        ...(selectedTool ? { [selectedTool]: selection } : {}),
      }));
      navigateTo("execution-mode-selector");
    },
    [navigateTo, selectedTool],
  );

  const completeSelection = useCallback(
    (
      executionMode: ExecutionMode,
      skip: boolean,
      sessionId?: string | null,
    ) => {
      if (selectedBranch && selectedTool) {
        const defaultModel = getDefaultModelOption(selectedTool);
        const resolvedModel = selectedModel?.model ?? defaultModel?.id ?? null;
        const resolvedInference =
          selectedModel?.inferenceLevel ??
          getDefaultInferenceForModel(defaultModel ?? undefined);

        const payload: SelectionResult = {
          branch: selectedBranch.name,
          displayName: selectedBranch.displayName,
          branchType: selectedBranch.branchType,
          tool: selectedTool,
          mode: executionMode,
          skipPermissions: skip,
          ...(resolvedModel !== undefined ? { model: resolvedModel } : {}),
          ...(resolvedInference !== undefined
            ? { inferenceLevel: resolvedInference }
            : {}),
          ...(selectedBranch.remoteBranch
            ? { remoteBranch: selectedBranch.remoteBranch }
            : {}),
          ...(sessionId ? { sessionId } : {}),
        };

        onExit(payload);
        exit();
      }
    },
    [
      selectedBranch,
      selectedTool,
      selectedModel,
      onExit,
      exit,
      getDefaultModelOption,
      getDefaultInferenceForModel,
    ],
  );

  const handleQuickStartSelect = useCallback(
    (action: QuickStartAction, toolId?: AITool | null) => {
      if (action === "manual" || !branchQuickStart.length) {
        navigateTo("ai-tool-selector");
        return;
      }

      const selected =
        branchQuickStart.find((opt) => opt.toolId === toolId) ??
        branchQuickStart[0];
      if (!selected) {
        navigateTo("ai-tool-selector");
        return;
      }

      setSelectedTool(selected.toolId);
      setPreferredToolId(selected.toolId);
      setSelectedModel(
        selected.model
          ? ({
              model: selected.model,
              inferenceLevel: selected.inferenceLevel ?? undefined,
            } as ModelSelectionResult)
          : null,
      );

      const skip = selected.skipPermissions ?? false;

      if (action === "reuse-continue") {
        const hasSession = Boolean(selected.sessionId);
        const mode: ExecutionMode = hasSession ? "resume" : "continue";
        completeSelection(mode, skip, selected.sessionId ?? null);
        return;
      }

      // "Start new with previous settings" skips the execution mode screen and launches immediately
      completeSelection("normal", skip, null);
    },
    [
      branchQuickStart,
      navigateTo,
      setPreferredToolId,
      setSelectedModel,
      setSelectedTool,
      completeSelection,
    ],
  );

  // Handle execution mode and skipPermissions selection
  const handleModeSelect = useCallback(
    (result: { mode: ExecutionMode; skipPermissions: boolean }) => {
      completeSelection(result.mode, result.skipPermissions, null);
    },
    [completeSelection],
  );

  // Render screen based on currentScreen
  const renderScreen = () => {
    const renderBranchListScreen = (
      additionalProps?: Partial<BranchListScreenProps>,
    ) => (
      <BranchListScreen
        branches={branchItems}
        stats={stats}
        onSelect={handleSelect}
        onQuit={handleQuit}
        onRefresh={refresh}
        loading={loading}
        error={error}
        lastUpdated={lastUpdated}
        loadingIndicatorDelay={loadingIndicatorDelay}
        version={version}
        workingDirectory={workingDirectory}
        activeProfile={activeProfileName}
        onOpenProfiles={() => navigateTo("environment-profile")}
        {...additionalProps}
      />
    );

    switch (currentScreen) {
      case "branch-list":
        return renderBranchListScreen({
          onCleanupCommand: handleCleanupCommand,
          cleanupUI: {
            indicators: cleanupIndicators,
            footerMessage: cleanupFooterMessage,
            inputLocked: cleanupInputLocked,
          },
          selectedBranches,
          onToggleSelect: toggleBranchSelection,
        });

      case "branch-creator":
        return (
          <BranchCreatorScreen
            onBack={goBack}
            onCreate={handleCreate}
            baseBranch={baseBranchLabel}
            version={version}
          />
        );

      case "branch-action-selector": {
        const isProtected = Boolean(protectedBranchInfo);
        const baseProps = {
          selectedBranch: selectedBranch?.displayName ?? "",
          onUseExisting: handleUseExistingBranch,
          onCreateNew: handleCreateNewBranch,
          onBack: goBack,
          canCreateNew: Boolean(selectedBranch),
        };

        if (isProtected) {
          return (
            <BranchActionSelectorScreen
              {...baseProps}
              mode="protected"
              infoMessage={protectedBranchInfo?.message ?? null}
              primaryLabel="Use root branch (no worktree)"
              secondaryLabel="Create new branch from this branch"
            />
          );
        }

        return <BranchActionSelectorScreen {...baseProps} />;
      }

      case "branch-quick-start":
        return (
          <BranchQuickStartScreen
            branchName={selectedBranch?.displayName ?? ""}
            previousOptions={branchQuickStart.map((opt) => ({
              toolId: opt.toolId,
              toolLabel: opt.toolLabel,
              model: opt.model ?? null,
              inferenceLevel: opt.inferenceLevel ?? null,
              skipPermissions: opt.skipPermissions ?? null,
              sessionId: opt.sessionId ?? null,
            }))}
            loading={branchQuickStartLoading}
            onBack={goBack}
            onSelect={handleQuickStartSelect}
            version={version}
          />
        );

      case "ai-tool-selector":
        return (
          <AIToolSelectorScreen
            onBack={goBack}
            onSelect={handleToolSelect}
            version={version}
            initialToolId={selectedTool ?? preferredToolId ?? null}
          />
        );

      case "model-selector":
        if (!selectedTool) {
          goBack();
          return null;
        }
        return (
          <ModelSelectorScreen
            tool={selectedTool}
            onBack={goBack}
            onSelect={handleModelSelect}
            version={version}
            initialSelection={selectedModel}
          />
        );

      case "execution-mode-selector":
        return (
          <ExecutionModeSelectorScreen
            onBack={goBack}
            onSelect={handleModeSelect}
            version={version}
            continueSessionId={continueSessionId}
          />
        );

      case "environment-profile":
        return (
          <EnvironmentProfileScreen
            onBack={() => {
              void refreshProfiles();
              goBack();
            }}
            version={version}
          />
        );

      default:
        return renderBranchListScreen();
    }
  };

  return <ErrorBoundary>{renderScreen()}</ErrorBoundary>;
}
