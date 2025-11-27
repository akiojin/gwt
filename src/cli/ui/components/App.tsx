import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useApp } from "ink";
import { ErrorBoundary } from "./common/ErrorBoundary.js";
import { BranchListScreen } from "./screens/BranchListScreen.js";
import { WorktreeManagerScreen } from "./screens/WorktreeManagerScreen.js";
import { BranchCreatorScreen } from "./screens/BranchCreatorScreen.js";
import { BranchActionSelectorScreen } from "../screens/BranchActionSelectorScreen.js";
import { AIToolSelectorScreen } from "./screens/AIToolSelectorScreen.js";
import { SessionSelectorScreen } from "./screens/SessionSelectorScreen.js";
import { ExecutionModeSelectorScreen } from "./screens/ExecutionModeSelectorScreen.js";
import type { ExecutionMode } from "./screens/ExecutionModeSelectorScreen.js";
import {
  ModelSelectorScreen,
  type ModelSelectionResult,
} from "./screens/ModelSelectorScreen.js";
import type { WorktreeItem } from "./screens/WorktreeManagerScreen.js";
import { useGitData } from "../hooks/useGitData.js";
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

  // Version state
  const [version, setVersion] = useState<string | null>(null);

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
  const spinnerFrameIndexRef = useRef(0);
  const [spinnerFrameIndex, setSpinnerFrameIndex] = useState(0);
  const completionTimerRef = useRef<NodeJS.Timeout | null>(null);

  // Fetch version on mount
  useEffect(() => {
    getPackageVersion()
      .then(setVersion)
      .catch(() => setVersion(null));
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
      });
    }
    return formatBranchItems(visibleBranches, worktreeMap);
  }, [branchHash, worktreeHash, visibleBranches, worktrees]);

  // Calculate statistics (memoized for performance)
  const stats = useMemo(
    () => calculateStatistics(visibleBranches),
    [visibleBranches],
  );

  // Format worktrees to WorktreeItems
  const worktreeItems: WorktreeItem[] = useMemo(
    () =>
      worktrees.map(
        (wt): WorktreeItem => ({
          branch: wt.branch,
          path: wt.path,
          isAccessible: wt.isAccessible ?? true,
        }),
      ),
    [worktrees],
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

  // Handle navigation
  const handleNavigate = useCallback(
    (screen: string) => {
      navigateTo(screen as any);
    },
    [navigateTo],
  );

  const handleWorktreeSelect = useCallback(
    (worktree: WorktreeItem) => {
      const lastTool = branches.find((b) => b.name === worktree.branch)
        ?.lastToolUsage?.toolId;
      setSelectedBranch({
        name: worktree.branch,
        displayName: worktree.branch,
        branchType: "local",
        branchCategory: inferBranchCategory(worktree.branch),
      });
      setSelectedTool(null);
      setSelectedModel(null);
      setCreationSourceBranch(null);
      setPreferredToolId(lastTool ?? null);
      setCleanupFooterMessage(null);
      navigateTo("ai-tool-selector");
    },
    [
      inferBranchCategory,
      navigateTo,
      setCleanupFooterMessage,
      setCreationSourceBranch,
      branches,
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
      navigateTo("ai-tool-selector");
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setCleanupFooterMessage({
        text: `Failed to switch root branch: ${message}`,
        color: "red",
      });
      console.error("Failed to switch protected branch:", error);
    }
  }, [navigateTo, refresh, selectedBranch, setCleanupFooterMessage]);

  const handleUseExistingBranch = useCallback(() => {
    if (selectedBranch && isProtectedSelection(selectedBranch)) {
      void handleProtectedBranchSwitch();
      return;
    }
    navigateTo("ai-tool-selector");
  }, [
    handleProtectedBranchSwitch,
    isProtectedSelection,
    navigateTo,
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

  // Handle session selection
  const handleSessionSelect = useCallback(
    (_session: string) => {
      // TODO: Load selected session and navigate to next screen
      // For now, just go back to branch list
      goBack();
    },
    [goBack],
  );

  // Handle execution mode and skipPermissions selection
  const handleModeSelect = useCallback(
    (result: { mode: ExecutionMode; skipPermissions: boolean }) => {
      // All selections complete - exit with result
      if (selectedBranch && selectedTool) {
        const defaultModel = getDefaultModelOption(selectedTool);
        const resolvedModel =
          selectedModel?.model ?? defaultModel?.id ?? null;
        const resolvedInference =
          selectedModel?.inferenceLevel ??
          getDefaultInferenceForModel(defaultModel ?? undefined);

        const modelForPayload = resolvedModel;

        const payload: SelectionResult = {
          branch: selectedBranch.name,
          displayName: selectedBranch.displayName,
          branchType: selectedBranch.branchType,
          tool: selectedTool,
          mode: result.mode,
          skipPermissions: result.skipPermissions,
          ...(modelForPayload !== undefined ? { model: modelForPayload } : {}),
          ...(resolvedInference !== undefined
            ? { inferenceLevel: resolvedInference }
            : {}),
          ...(selectedBranch.remoteBranch
            ? { remoteBranch: selectedBranch.remoteBranch }
            : {}),
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

  // Render screen based on currentScreen
  const renderScreen = () => {
    switch (currentScreen) {
      case "branch-list":
        return (
          <BranchListScreen
            branches={branchItems}
            stats={stats}
            onSelect={handleSelect}
            onNavigate={handleNavigate}
            onQuit={handleQuit}
            onCleanupCommand={handleCleanupCommand}
            onRefresh={refresh}
            loading={loading}
            error={error}
            lastUpdated={lastUpdated}
            loadingIndicatorDelay={loadingIndicatorDelay}
            cleanupUI={{
              indicators: cleanupIndicators,
              footerMessage: cleanupFooterMessage,
              inputLocked: cleanupInputLocked,
            }}
            version={version}
            workingDirectory={workingDirectory}
          />
        );

      case "worktree-manager":
        return (
          <WorktreeManagerScreen
            worktrees={worktreeItems}
            onBack={goBack}
            onSelect={handleWorktreeSelect}
            version={version}
          />
        );

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

      case "session-selector":
        // TODO: Implement session data fetching
        return (
          <SessionSelectorScreen
            sessions={[]}
            onBack={goBack}
            onSelect={handleSessionSelect}
            version={version}
          />
        );

      case "execution-mode-selector":
        return (
          <ExecutionModeSelectorScreen
            onBack={goBack}
            onSelect={handleModeSelect}
            version={version}
          />
        );

      default:
        return (
          <BranchListScreen
            branches={branchItems}
            stats={stats}
            onSelect={handleSelect}
            onNavigate={handleNavigate}
            onQuit={handleQuit}
            onRefresh={refresh}
            loading={loading}
            error={error}
            lastUpdated={lastUpdated}
            loadingIndicatorDelay={loadingIndicatorDelay}
            version={version}
            workingDirectory={workingDirectory}
          />
        );
    }
  };

  return <ErrorBoundary>{renderScreen()}</ErrorBoundary>;
}
