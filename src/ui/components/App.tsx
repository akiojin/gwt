import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useApp } from 'ink';
import { ErrorBoundary } from './common/ErrorBoundary.js';
import { BranchListScreen } from './screens/BranchListScreen.js';
import { WorktreeManagerScreen } from './screens/WorktreeManagerScreen.js';
import { BranchCreatorScreen } from './screens/BranchCreatorScreen.js';
import { BranchActionSelectorScreen } from '../screens/BranchActionSelectorScreen.js';
import { AIToolSelectorScreen } from './screens/AIToolSelectorScreen.js';
import { SessionSelectorScreen } from './screens/SessionSelectorScreen.js';
import { ExecutionModeSelectorScreen } from './screens/ExecutionModeSelectorScreen.js';
import type { AITool } from './screens/AIToolSelectorScreen.js';
import type { ExecutionMode } from './screens/ExecutionModeSelectorScreen.js';
import type { WorktreeItem } from './screens/WorktreeManagerScreen.js';
import { useGitData } from '../hooks/useGitData.js';
import { useScreenState } from '../hooks/useScreenState.js';
import { formatBranchItems } from '../utils/branchFormatter.js';
import { calculateStatistics } from '../utils/statisticsCalculator.js';
import type { BranchItem } from '../types.js';
import { getRepositoryRoot, deleteBranch } from '../../git.js';
import {
  createWorktree,
  generateWorktreePath,
  getMergedPRWorktrees,
  removeWorktree,
} from '../../worktree.js';

const SPINNER_FRAMES = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];
const COMPLETION_HOLD_DURATION_MS = 3000;

const getSpinnerFrame = (index: number): string => {
  const frame = SPINNER_FRAMES[index];
  if (typeof frame === 'string') {
    return frame;
  }
  return SPINNER_FRAMES[0] ?? '⠋';
};

export interface SelectionResult {
  branch: string; // Local branch name (without remote prefix)
  displayName: string; // Name that was selected in the UI (may include remote prefix)
  branchType: 'local' | 'remote';
  remoteBranch?: string; // Full remote ref when branchType === 'remote'
  tool: AITool;
  mode: ExecutionMode;
  skipPermissions: boolean;
}

export interface AppProps {
  onExit: (result?: SelectionResult) => void;
  loadingIndicatorDelay?: number;
}

interface SelectedBranchState {
  name: string;
  displayName: string;
  branchType: 'local' | 'remote';
  remoteBranch?: string;
}

/**
 * App - Top-level component for Ink.js UI
 * Integrates ErrorBoundary, data fetching, screen navigation, and all screens
 */
export function App({ onExit, loadingIndicatorDelay = 300 }: AppProps) {
  const { exit } = useApp();
  const { branches, worktrees, loading, error, refresh, lastUpdated } = useGitData();
  const { currentScreen, navigateTo, goBack, reset } = useScreenState();

  // Selection state (for branch → tool → mode flow)
  const [selectedBranch, setSelectedBranch] = useState<SelectedBranchState | null>(null);
  const [selectedTool, setSelectedTool] = useState<AITool | null>(null);

  // PR cleanup feedback
  const [cleanupIndicators, setCleanupIndicators] = useState<Record<string, { icon: string; color?: 'cyan' | 'green' | 'yellow' | 'red' }>>({});
  const [cleanupProcessingBranch, setCleanupProcessingBranch] = useState<string | null>(null);
  const [cleanupInputLocked, setCleanupInputLocked] = useState(false);
  const [cleanupFooterMessage, setCleanupFooterMessage] = useState<{ text: string; color?: 'cyan' | 'green' | 'yellow' | 'red' } | null>(null);
  const [hiddenBranches, setHiddenBranches] = useState<string[]>([]);
  const spinnerFrameIndexRef = useRef(0);
  const [spinnerFrameIndex, setSpinnerFrameIndex] = useState(0);
  const completionTimerRef = useRef<NodeJS.Timeout | null>(null);

  useEffect(() => {
    if (!cleanupInputLocked) {
      spinnerFrameIndexRef.current = 0;
      setSpinnerFrameIndex(0);
      return undefined;
    }

    const interval = setInterval(() => {
      spinnerFrameIndexRef.current = (spinnerFrameIndexRef.current + 1) % SPINNER_FRAMES.length;
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
        if (current && current.icon === frame && current.color === 'cyan') {
          return prev;
        }

        const next: Record<string, { icon: string; color?: 'cyan' | 'green' | 'yellow' | 'red' }> = {
          ...prev,
          [cleanupProcessingBranch]: { icon: frame, color: 'cyan' },
        };

        return next;
      });
    }

    setCleanupFooterMessage({ text: `Processing... ${frame}`, color: 'cyan' });
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

  useEffect(() => () => {
    if (completionTimerRef.current) {
      clearTimeout(completionTimerRef.current);
      completionTimerRef.current = null;
    }
  }, []);

  const visibleBranches = useMemo(
    () => branches.filter((branch) => !hiddenBranches.includes(branch.name)),
    [branches, hiddenBranches]
  );

  // Format branches to BranchItems (memoized for performance)
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
  }, [visibleBranches, worktrees]);

  // Calculate statistics (memoized for performance)
  const stats = useMemo(() => calculateStatistics(visibleBranches), [visibleBranches]);

  // Format worktrees to WorktreeItems
  const worktreeItems: WorktreeItem[] = useMemo(
    () =>
      worktrees.map((wt): WorktreeItem => ({
        branch: wt.branch,
        path: wt.path,
        isAccessible: wt.isAccessible ?? true,
      })),
    [worktrees]
  );

  const resolveBaseBranch = useCallback(() => {
    const localMain = branches.find(
      (branch) =>
        branch.type === 'local' && (branch.name === 'main' || branch.name === 'master')
    );
    if (localMain) {
      return localMain.name;
    }

    const develop = branches.find(
      (branch) => branch.type === 'local' && (branch.name === 'develop' || branch.name === 'dev')
    );
    if (develop) {
      return develop.name;
    }

    return 'main';
  }, [branches]);

  // Handle branch selection
  const toLocalBranchName = useCallback((remoteName: string) => {
    const segments = remoteName.split('/');
    if (segments.length <= 1) {
      return remoteName;
    }
    return segments.slice(1).join('/');
  }, []);

  const handleSelect = useCallback(
    (item: BranchItem) => {
      const selection: SelectedBranchState =
        item.type === 'remote'
          ? {
              name: toLocalBranchName(item.name),
              displayName: item.name,
              branchType: 'remote',
              remoteBranch: item.name,
            }
          : {
              name: item.name,
              displayName: item.name,
              branchType: 'local',
            };

      setSelectedBranch(selection);
      setSelectedTool(null);
      navigateTo('branch-action-selector');
    },
    [navigateTo, setSelectedTool, toLocalBranchName]
  );

  // Handle navigation
  const handleNavigate = useCallback(
    (screen: string) => {
      navigateTo(screen as any);
    },
    [navigateTo]
  );

  const handleWorktreeSelect = useCallback(
    (worktree: WorktreeItem) => {
      setSelectedBranch({
        name: worktree.branch,
        displayName: worktree.branch,
        branchType: 'local',
      });
      setSelectedTool(null);
      navigateTo('ai-tool-selector');
    },
    [navigateTo]
  );

  // Handle branch action selection
  const handleUseExistingBranch = useCallback(() => {
    navigateTo('ai-tool-selector');
  }, [navigateTo]);

  const handleCreateNewBranch = useCallback(() => {
    navigateTo('branch-creator');
  }, [navigateTo]);

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
        const baseBranch = selectedBranch?.name ?? resolveBaseBranch();

        await createWorktree({
          branchName,
          worktreePath,
          repoRoot,
          isNewBranch: true,
          baseBranch,
        });

        refresh();
        setSelectedBranch({
          name: branchName,
          displayName: branchName,
          branchType: 'local',
        });
        setSelectedTool(null);

        navigateTo('ai-tool-selector');
      } catch (error) {
        // On error, go back to branch list
        console.error('Failed to create branch:', error);
        goBack();
        refresh();
      }
    },
    [navigateTo, goBack, refresh, resolveBaseBranch, selectedBranch]
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
    setCleanupFooterMessage({ text: `Processing... ${initialFrame}`, color: 'cyan' });
    setCleanupProcessingBranch(null);
    spinnerFrameIndexRef.current = 0;
    setSpinnerFrameIndex(0);

    let targets;
    try {
      targets = await getMergedPRWorktrees();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setCleanupIndicators({});
      setCleanupFooterMessage({ text: `❌ ${message}`, color: 'red' });
      setCleanupInputLocked(false);
      completionTimerRef.current = setTimeout(() => {
        setCleanupFooterMessage(null);
        completionTimerRef.current = null;
      }, COMPLETION_HOLD_DURATION_MS);
      return;
    }

    if (targets.length === 0) {
      setCleanupIndicators({});
      setCleanupFooterMessage({ text: '✅ クリーンアップ対象のマージ済みPRはありません。', color: 'green' });
      setCleanupInputLocked(false);
      completionTimerRef.current = setTimeout(() => {
        setCleanupFooterMessage(null);
        completionTimerRef.current = null;
      }, COMPLETION_HOLD_DURATION_MS);
      return;
    }

    // Reset hidden branches that may already be gone
    setHiddenBranches((prev) => prev.filter((name) => targets.find((t) => t.branch === name) === undefined));

    const initialIndicators = targets.reduce<Record<string, { icon: string; color?: 'cyan' | 'green' | 'yellow' | 'red' }>>((acc, target, index) => {
      const icon = index === 0 ? getSpinnerFrame(0) : '⏳';
      const color: 'cyan' | 'green' | 'yellow' | 'red' = index === 0 ? 'cyan' : 'yellow';
      acc[target.branch] = { icon, color };
      return acc;
    }, {});

    setCleanupIndicators(initialIndicators);
    const firstTarget = targets.length > 0 ? targets[0] : undefined;
    setCleanupProcessingBranch(firstTarget ? firstTarget.branch : null);
    spinnerFrameIndexRef.current = 0;
    setSpinnerFrameIndex(0);
    setCleanupFooterMessage({ text: `Processing... ${getSpinnerFrame(0)}`, color: 'cyan' });

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
        updated[target.branch] = { icon: getSpinnerFrame(0), color: 'cyan' };
        for (const pending of targets.slice(index + 1)) {
          const current = updated[pending.branch];
          if (!current || current.icon !== '⏳') {
            updated[pending.branch] = { icon: '⏳', color: 'yellow' };
          }
        }
        return updated;
      });

      const shouldSkip =
        target.hasUncommittedChanges ||
        target.hasUnpushedCommits ||
        (target.cleanupType === 'worktree-and-branch' && (!target.worktreePath || target.isAccessible === false));

      if (shouldSkip) {
        setCleanupIndicators((prev) => ({
          ...prev,
          [target.branch]: { icon: '⏭️', color: 'yellow' },
        }));
        setCleanupProcessingBranch(null);
        continue;
      }

      try {
        if (target.cleanupType === 'worktree-and-branch' && target.worktreePath) {
          await removeWorktree(target.worktreePath, true);
        }

        await deleteBranch(target.branch, true);
        succeededBranches.push(target.branch);
        setCleanupIndicators((prev) => ({
          ...prev,
          [target.branch]: { icon: '✅', color: 'green' },
        }));
      } catch (error) {
        const icon = '❌';
        setCleanupIndicators((prev) => ({
          ...prev,
          [target.branch]: { icon, color: 'red' },
        }));
      }

      setCleanupProcessingBranch(null);
    }

    setCleanupProcessingBranch(null);
    setCleanupInputLocked(false);
    setCleanupFooterMessage({ text: 'Cleanup completed. Finalizing...', color: 'green' });

    completionTimerRef.current = setTimeout(resetAfterWait, COMPLETION_HOLD_DURATION_MS);
  }, [cleanupInputLocked, deleteBranch, getMergedPRWorktrees, refresh, removeWorktree]);

  // Handle AI tool selection
  const handleToolSelect = useCallback(
    (tool: AITool) => {
      setSelectedTool(tool);
      navigateTo('execution-mode-selector');
    },
    [navigateTo]
  );

  // Handle session selection
  const handleSessionSelect = useCallback(
    (session: string) => {
      // TODO: Load selected session and navigate to next screen
      // For now, just go back to branch list
      goBack();
    },
    [goBack]
  );

  // Handle execution mode and skipPermissions selection
  const handleModeSelect = useCallback(
    (result: { mode: ExecutionMode; skipPermissions: boolean }) => {
      // All selections complete - exit with result
      if (selectedBranch && selectedTool) {
        const payload: SelectionResult = {
          branch: selectedBranch.name,
          displayName: selectedBranch.displayName,
          branchType: selectedBranch.branchType,
          tool: selectedTool,
          mode: result.mode,
          skipPermissions: result.skipPermissions,
          ...(selectedBranch.remoteBranch
            ? { remoteBranch: selectedBranch.remoteBranch }
            : {}),
        };

        onExit(payload);
        exit();
      }
    },
    [selectedBranch, selectedTool, onExit, exit]
  );

  // Render screen based on currentScreen
  const renderScreen = () => {
    switch (currentScreen) {
      case 'branch-list':
        return (
          <BranchListScreen
            branches={branchItems}
            stats={stats}
            onSelect={handleSelect}
            onNavigate={handleNavigate}
            onQuit={handleQuit}
            onCleanupCommand={handleCleanupCommand}
            loading={loading}
            error={error}
            lastUpdated={lastUpdated}
            loadingIndicatorDelay={loadingIndicatorDelay}
            cleanupUI={{
              indicators: cleanupIndicators,
              footerMessage: cleanupFooterMessage,
              inputLocked: cleanupInputLocked,
            }}
          />
        );

      case 'worktree-manager':
        return (
          <WorktreeManagerScreen
            worktrees={worktreeItems}
            onBack={goBack}
            onSelect={handleWorktreeSelect}
          />
        );

      case 'branch-creator':
        return (
          <BranchCreatorScreen
            onBack={goBack}
            onCreate={handleCreate}
            baseBranch={selectedBranch?.displayName}
          />
        );

      case 'branch-action-selector':
        return (
          <BranchActionSelectorScreen
            selectedBranch={selectedBranch?.displayName ?? ''}
            onUseExisting={handleUseExistingBranch}
            onCreateNew={handleCreateNewBranch}
          />
        );

      case 'ai-tool-selector':
        return <AIToolSelectorScreen onBack={goBack} onSelect={handleToolSelect} />;

      case 'session-selector':
        // TODO: Implement session data fetching
        return (
          <SessionSelectorScreen
            sessions={[]}
            onBack={goBack}
            onSelect={handleSessionSelect}
          />
        );

      case 'execution-mode-selector':
        return (
          <ExecutionModeSelectorScreen onBack={goBack} onSelect={handleModeSelect} />
        );

      default:
        return (
          <BranchListScreen
            branches={branchItems}
            stats={stats}
            onSelect={handleSelect}
            onNavigate={handleNavigate}
            onQuit={handleQuit}
            loading={loading}
            error={error}
            lastUpdated={lastUpdated}
            loadingIndicatorDelay={loadingIndicatorDelay}
          />
        );
    }
  };

  return <ErrorBoundary>{renderScreen()}</ErrorBoundary>;
}
