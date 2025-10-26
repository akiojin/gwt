import React, {
  useCallback,
  useEffect,
  useMemo,
  useState,
} from 'react';
import { useApp } from 'ink';
import { ErrorBoundary } from './common/ErrorBoundary.js';
import { BranchListScreen } from './screens/BranchListScreen.js';
import { WorktreeManagerScreen } from './screens/WorktreeManagerScreen.js';
import { BranchCreatorScreen } from './screens/BranchCreatorScreen.js';
import { PRCleanupScreen } from './screens/PRCleanupScreen.js';
import {
  AIToolSelectorScreen,
  type AITool,
  type AIToolItem,
} from './screens/AIToolSelectorScreen.js';
import {
  SessionSelectorScreen,
  type SessionListEntry,
} from './screens/SessionSelectorScreen.js';
import {
  ExecutionModeSelectorScreen,
  type ExecutionMode,
} from './screens/ExecutionModeSelectorScreen.js';
import type { WorktreeItem } from './screens/WorktreeManagerScreen.js';
import { useGitData } from '../hooks/useGitData.js';
import { useScreenState } from '../hooks/useScreenState.js';
import { formatBranchItems } from '../utils/branchFormatter.js';
import { calculateStatistics } from '../utils/statisticsCalculator.js';
import type { BranchItem, MergedPullRequest } from '../types.js';
import {
  loadSession,
  getAllSessions,
  type SessionData,
} from '../../config/index.js';
import {
  worktreeExists,
  generateWorktreePath,
} from '../../worktree.js';
import { isClaudeCodeAvailable } from '../../claude.js';
import { isCodexAvailable } from '../../codex.js';

export interface LaunchRequest {
  repoRoot: string;
  branchName: string;
  worktreePath: string;
  mode: ExecutionMode;
  tool?: AITool;
  skipPermissions: boolean;
  createWorktree: boolean;
  isNewBranch: boolean;
  baseBranch?: string;
  session?: SessionData;
}

export type AppResult =
  | { type: 'quit' }
  | { type: 'launch'; launch: LaunchRequest };

export interface AppProps {
  repoRoot: string;
  onExit: (result: AppResult) => void;
}

interface ToolAvailabilityState {
  claude: boolean;
  codex: boolean;
  loading: boolean;
  error?: string | null;
}

/**
 * App - Ink UI orchestration
 */
export function App({ repoRoot, onExit }: AppProps) {
  const { exit } = useApp();
  const {
    branches,
    worktrees,
    loading: gitLoading,
    error: gitError,
    refresh,
    lastUpdated,
  } = useGitData();
  const { currentScreen, navigateTo, goBack, reset } = useScreenState();

  const [pendingLaunch, setPendingLaunch] = useState<LaunchRequest | null>(null);
  const [sessions, setSessions] = useState<SessionData[]>([]);
  const [sessionEntries, setSessionEntries] = useState<SessionListEntry[]>([]);
  const [infoMessage, setInfoMessage] = useState<string | null>(null);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [toolAvailability, setToolAvailability] = useState<ToolAvailabilityState>({
    claude: false,
    codex: false,
    loading: false,
  });

  // Format branches to BranchItems (memoized for performance)
  const branchItems: BranchItem[] = useMemo(
    () => formatBranchItems(branches),
    [branches],
  );

  // Calculate statistics (memoized for performance)
  const stats = useMemo(
    () => calculateStatistics(branches),
    [branches],
  );

  // Format worktrees to WorktreeItems
  const worktreeItems: WorktreeItem[] = useMemo(
    () =>
      worktrees.map((wt): WorktreeItem => ({
        branch: wt.branch,
        path: wt.path,
        isAccessible: wt.isAccessible ?? true,
      })),
    [worktrees],
  );

  /**
   * When a launch request is fully specified (includes tool), exit Ink UI
   * and bubble the decision to the Node entry point.
   */
  useEffect(() => {
    if (pendingLaunch?.tool) {
      onExit({ type: 'launch', launch: pendingLaunch });
      exit();
    }
  }, [pendingLaunch, exit, onExit]);

  const finalizeLaunchWithTool = useCallback(
    (tool: AITool) => {
      setPendingLaunch((prev) =>
        prev
          ? {
              ...prev,
              tool,
            }
          : prev,
      );
    },
    [],
  );

  const toggleSkipPermissions = useCallback(() => {
    setPendingLaunch((prev) =>
      prev
        ? {
            ...prev,
            skipPermissions: !prev.skipPermissions,
          }
        : prev,
    );
  }, []);

  const ensureToolAvailability = useCallback(async () => {
    setToolAvailability({
      claude: false,
      codex: false,
      loading: true,
      error: null,
    });
    try {
      const [claude, codex] = await Promise.all([
        isClaudeCodeAvailable().catch(() => false),
        isCodexAvailable().catch(() => false),
      ]);
      setToolAvailability({
        claude,
        codex,
        loading: false,
        error: null,
      });
      if (!claude && !codex) {
        setInfoMessage(
          "No AI tools detected in PATH. Install Claude Code or Codex CLI first.",
        );
      } else {
        setInfoMessage(null);
      }
    } catch (error) {
      setToolAvailability({
        claude: false,
        codex: false,
        loading: false,
        error: error instanceof Error ? error.message : String(error),
      });
      setInfoMessage("Failed to detect AI tool availability.");
    }
  }, []);

  /**
   * Branch selection (normal mode)
   */
  const handleBranchSelect = useCallback(
    (item: BranchItem) => {
      (async () => {
        try {
          setStatusMessage(`Preparing worktree for ${item.name}...`);
          const existingWorktreePath =
            item.worktree?.path ||
            (await worktreeExists(item.name)) ||
            null;

          let worktreePath = existingWorktreePath;
          let createWorktree = false;

          if (!worktreePath) {
            worktreePath = await generateWorktreePath(repoRoot, item.name);
            createWorktree = true;
          }

          setPendingLaunch({
            repoRoot,
            branchName: item.name,
            worktreePath,
            mode: 'normal',
            skipPermissions: false,
            createWorktree,
            isNewBranch: false,
          });

          setStatusMessage(null);
          await ensureToolAvailability();
          navigateTo('ai-tool-selector');
        } catch (error) {
          setStatusMessage(null);
          setInfoMessage(
            error instanceof Error
              ? error.message
              : `Failed to prepare branch: ${String(error)}`,
          );
        }
      })().catch(() => {
        setStatusMessage(null);
      });
    },
    [ensureToolAvailability, navigateTo, repoRoot],
  );

  /**
   * Execution mode selection (normal / continue / resume)
   */
  const handleModeSelect = useCallback(
    (mode: ExecutionMode) => {
      switch (mode) {
        case 'normal': {
          setInfoMessage(null);
          setStatusMessage('Select a branch to start a new session.');
          navigateTo('branch-list');
          return;
        }
        case 'continue': {
          (async () => {
            try {
              setStatusMessage('Loading last session...');
              const session = await loadSession(repoRoot);
              if (!session || !session.lastWorktreePath || !session.lastBranch) {
                setInfoMessage(
                  'No recent session found. Start a new session from branch list.',
                );
                setStatusMessage(null);
                navigateTo('branch-list');
                return;
              }

              setPendingLaunch({
                repoRoot,
                branchName: session.lastBranch,
                worktreePath: session.lastWorktreePath,
                mode: 'continue',
                skipPermissions: false,
                createWorktree: false,
                isNewBranch: false,
                session,
              });

              setStatusMessage(null);
              setInfoMessage(null);
              await ensureToolAvailability();
              navigateTo('ai-tool-selector');
            } catch (error) {
              setStatusMessage(null);
              setInfoMessage(
                error instanceof Error
                  ? error.message
                  : `Failed to load session: ${String(error)}`,
              );
              navigateTo('branch-list');
            }
          })();
          return;
        }
        case 'resume': {
          (async () => {
            try {
              setStatusMessage('Loading available sessions...');
              const recentSessions = await getAllSessions();
              if (recentSessions.length === 0) {
                setInfoMessage('No sessions were found in the last 24 hours.');
                setStatusMessage(null);
                navigateTo('branch-list');
                return;
              }
              setSessions(recentSessions);
              const entries = recentSessions.map<SessionListEntry>((session) => ({
                id: `${session.repositoryRoot}:${session.lastWorktreePath ?? ''}:${session.timestamp}`,
                branchName: session.lastBranch ?? '(unknown)',
                worktreePath: session.lastWorktreePath ?? 'N/A',
                formattedTimestamp: new Date(session.timestamp).toLocaleString(),
              }));
              setSessionEntries(entries);
              setStatusMessage(null);
              setInfoMessage(null);
              navigateTo('session-selector');
            } catch (error) {
              setStatusMessage(null);
              setInfoMessage(
                error instanceof Error
                  ? error.message
                  : `Failed to load sessions: ${String(error)}`,
              );
              navigateTo('branch-list');
            }
          })();
          return;
        }
      }
    },
    [ensureToolAvailability, navigateTo, repoRoot],
  );

  /**
   * Session selection (resume mode)
   */
  const handleSessionSelect = useCallback(
    (entry: SessionListEntry) => {
      const session = sessions.find(
        (s) =>
          `${s.repositoryRoot}:${s.lastWorktreePath ?? ''}:${s.timestamp}` ===
          entry.id,
      );
      if (!session || !session.lastWorktreePath || !session.lastBranch) {
        setInfoMessage('Selected session is no longer valid.');
        goBack();
        return;
      }

      setPendingLaunch({
        repoRoot,
        branchName: session.lastBranch,
        worktreePath: session.lastWorktreePath,
        mode: 'resume',
        skipPermissions: false,
        createWorktree: false,
        isNewBranch: false,
        session,
      });

      (async () => {
        await ensureToolAvailability();
        navigateTo('ai-tool-selector');
      })();
    },
    [ensureToolAvailability, goBack, navigateTo, repoRoot, sessions],
  );

  /**
   * Worktree manager selection (placeholder)
   */
  const handleWorktreeSelect = useCallback(
    (item: WorktreeItem) => {
      setPendingLaunch({
        repoRoot,
        branchName: item.branch,
        worktreePath: item.path,
        mode: 'normal',
        skipPermissions: false,
        createWorktree: false,
        isNewBranch: false,
      });
      (async () => {
        await ensureToolAvailability();
        navigateTo('ai-tool-selector');
      })();
    },
    [ensureToolAvailability, navigateTo, repoRoot],
  );

  /**
   * AI tool selection
   */
  const handleToolSelect = useCallback(
    (tool: AITool) => {
      finalizeLaunchWithTool(tool);
    },
    [finalizeLaunchWithTool],
  );

  /**
   * Quit handler
   */
  const handleQuit = useCallback(() => {
    onExit({ type: 'quit' });
    exit();
  }, [exit, onExit]);

  // Helper: derive tool items based on availability
  const toolItems: AIToolItem[] = useMemo(() => {
    const items: AIToolItem[] = [];
    if (toolAvailability.claude) {
      items.push({
        label: 'Claude Code',
        value: 'claude-code',
        description: 'Official Claude CLI tool',
      });
    }
    if (toolAvailability.codex) {
      items.push({
        label: 'Codex CLI',
        value: 'codex-cli',
        description: 'Codex CLI (bunx @openai/codex)',
      });
    }
    return items;
  }, [toolAvailability.claude, toolAvailability.codex]);

  const pendingSkip = pendingLaunch?.skipPermissions ?? false;

  // Render screen based on currentScreen
  const renderScreen = () => {
    switch (currentScreen) {
      case 'branch-list':
        return (
          <BranchListScreen
            branches={branchItems}
            stats={stats}
            onSelect={handleBranchSelect}
            onNavigate={(screen) => navigateTo(screen as any)}
            onQuit={handleQuit}
            loading={gitLoading}
            error={gitError}
            lastUpdated={lastUpdated}
            infoMessage={infoMessage}
            statusMessage={statusMessage}
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
            onCreate={(branchName) => {
              // Placeholder for future integration: refresh data after creation.
              refresh();
              goBack();
              setInfoMessage(`Branch ${branchName} created. Select it from the list.`);
            }}
          />
        );

      case 'pr-cleanup':
        return (
          <PRCleanupScreen
            pullRequests={[]}
            onBack={goBack}
            onCleanup={(pr: MergedPullRequest) => {
              // Placeholder cleanup handler
              goBack();
              setInfoMessage(`Cleanup initiated for PR #${pr.number}.`);
            }}
          />
        );

      case 'ai-tool-selector': {
        const toggleProps = pendingLaunch
          ? { onToggleSkip: toggleSkipPermissions }
          : {};
        return (
          <AIToolSelectorScreen
            onBack={() => {
              // Reset pending launch state when backing out
              setPendingLaunch((prev) => {
                if (!prev) {
                  return prev;
                }
                const { tool: _tool, ...rest } = prev;
                return rest;
              });
              goBack();
            }}
            onSelect={handleToolSelect}
            items={toolItems}
            loading={toolAvailability.loading}
            skipPermissions={pendingSkip}
            infoMessage={toolAvailability.error ?? infoMessage}
            {...toggleProps}
          />
        );
      }

      case 'session-selector':
        return (
          <SessionSelectorScreen
            sessions={sessionEntries}
            onBack={goBack}
            onSelect={handleSessionSelect}
            infoMessage={infoMessage}
          />
        );

      case 'execution-mode-selector':
        return (
          <ExecutionModeSelectorScreen
            onBack={handleQuit}
            onSelect={handleModeSelect}
          />
        );

      default:
        return (
          <BranchListScreen
            branches={branchItems}
            stats={stats}
            onSelect={handleBranchSelect}
            loading={gitLoading}
            error={gitError}
            lastUpdated={lastUpdated}
            infoMessage={infoMessage}
            statusMessage={statusMessage}
          />
        );
    }
  };

  return <ErrorBoundary>{renderScreen()}</ErrorBoundary>;
}
