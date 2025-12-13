import React, { useEffect, useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { useBranch, useSyncBranch } from "../hooks/useBranches";
import { useCreateWorktree } from "../hooks/useWorktrees";
import {
  useStartSession,
  useSessions,
  useDeleteSession,
} from "../hooks/useSessions";
import { useConfig } from "../hooks/useConfig";
import { ApiError } from "../lib/api";
import { PageHeader } from "@/components/common/PageHeader";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
  SessionHistoryTable,
  ToolLauncher,
  BranchInfoCards,
  TerminalPanel,
  type SelectableTool,
} from "@/components/branch-detail";
import type {
  Branch,
  CustomAITool,
  LastToolUsage,
} from "../../../../types/api.js";

type ToolType = "claude-code" | "codex-cli" | "custom";
type ToolMode = "normal" | "continue" | "resume";

interface BannerState {
  type: "success" | "error" | "info";
  message: string;
}

const BRANCH_TYPE_LABEL: Record<Branch["type"], string> = {
  local: "ローカル",
  remote: "リモート",
};

const MERGE_STATUS_LABEL: Record<Branch["mergeStatus"], string> = {
  merged: "マージ済み",
  unmerged: "未マージ",
  unknown: "状態不明",
};

export function BranchDetailPage() {
  const { branchName } = useParams<{ branchName: string }>();
  const decodedBranchName = branchName ? decodeURIComponent(branchName) : "";

  const { data: branch, isLoading, error } = useBranch(decodedBranchName);
  const syncBranch = useSyncBranch(decodedBranchName);
  const createWorktree = useCreateWorktree();
  const startSession = useStartSession();
  const { data: sessionsData, isLoading: isSessionsLoading } = useSessions();
  const deleteSession = useDeleteSession();
  const {
    data: config,
    isLoading: isConfigLoading,
    error: configError,
  } = useConfig();

  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [isStartingSession, setIsStartingSession] = useState(false);
  const [banner, setBanner] = useState<BannerState | null>(null);
  const [isTerminalFullscreen, setIsTerminalFullscreen] = useState(false);
  const [selectedToolId, setSelectedToolId] = useState<string>("claude-code");
  const [selectedMode, setSelectedMode] = useState<ToolMode>("normal");
  const [skipPermissions, setSkipPermissions] = useState(false);
  const [extraArgsText, setExtraArgsText] = useState("");
  const [terminatingSessionId, setTerminatingSessionId] = useState<
    string | null
  >(null);

  const formattedCommitDate = useMemo(
    () => formatDate(branch?.commitDate),
    [branch?.commitDate],
  );

  // Handle fullscreen body overflow
  useEffect(() => {
    if (!isTerminalFullscreen) return undefined;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, [isTerminalFullscreen]);

  // Loading state
  if (isLoading) {
    return (
      <div className="min-h-screen bg-background">
        <PageHeader
          eyebrow="BRANCH DETAIL"
          title="読み込み中..."
          subtitle="ブランチ情報を取得しています"
        />
        <main className="mx-auto max-w-7xl px-6 py-8">
          <div className="flex items-center justify-center py-20">
            <div className="text-center">
              <div className="mb-4 text-4xl">⏳</div>
              <p className="text-muted-foreground">Loading branch...</p>
            </div>
          </div>
        </main>
      </div>
    );
  }

  // Error state
  if (error) {
    return (
      <div className="min-h-screen bg-background">
        <PageHeader eyebrow="BRANCH DETAIL" title="エラー" />
        <main className="mx-auto max-w-7xl px-6 py-8">
          <Alert variant="destructive">
            <AlertDescription>
              {error instanceof Error ? error.message : "未知のエラーです"}
            </AlertDescription>
          </Alert>
          <div className="mt-4">
            <Button variant="ghost" asChild>
              <Link to="/">← ブランチ一覧に戻る</Link>
            </Button>
          </div>
        </main>
      </div>
    );
  }

  // Not found state
  if (!branch) {
    return (
      <div className="min-h-screen bg-background">
        <PageHeader eyebrow="BRANCH DETAIL" title="Branch not found" />
        <main className="mx-auto max-w-7xl px-6 py-8">
          <p className="mb-4 text-muted-foreground">
            指定されたブランチは存在しません。
          </p>
          <Button variant="ghost" asChild>
            <Link to="/">← ブランチ一覧に戻る</Link>
          </Button>
        </main>
      </div>
    );
  }

  // Computed values
  const canStartSession = Boolean(branch.worktreePath);
  const divergenceInfo = branch.divergence ?? null;
  const hasBlockingDivergence = Boolean(
    divergenceInfo && divergenceInfo.ahead > 0 && divergenceInfo.behind > 0,
  );
  const needsRemoteSync = Boolean(
    branch.worktreePath &&
    divergenceInfo &&
    divergenceInfo.behind > 0 &&
    divergenceInfo.ahead === 0 &&
    !hasBlockingDivergence,
  );
  const isSyncingBranch = syncBranch.isPending;

  // Available tools
  const customTools: CustomAITool[] = config?.tools ?? [];
  const availableTools: SelectableTool[] = useMemo(
    () => [
      { id: "claude-code", label: "Claude Code", target: "claude" },
      { id: "codex-cli", label: "Codex CLI", target: "codex" },
      ...customTools.map(
        (tool): SelectableTool => ({
          id: tool.id,
          label: tool.displayName,
          target: "custom" as const,
          definition: tool,
        }),
      ),
    ],
    [customTools],
  );

  // Ensure selected tool is valid
  useEffect(() => {
    if (!availableTools.length) {
      setSelectedToolId("claude-code");
      return;
    }
    if (!availableTools.find((tool) => tool.id === selectedToolId)) {
      const first = availableTools[0];
      if (first) setSelectedToolId(first.id);
    }
  }, [availableTools, selectedToolId]);

  const selectedTool = availableTools.find(
    (tool) => tool.id === selectedToolId,
  );

  // Branch sessions
  const branchSessions = useMemo(() => {
    return (sessionsData ?? [])
      .filter((session) => session.worktreePath === branch?.worktreePath)
      .sort((a, b) => (b.startedAt ?? "").localeCompare(a.startedAt ?? ""));
  }, [sessionsData, branch?.worktreePath]);

  // Latest tool usage
  const latestToolUsage: LastToolUsage | null = useMemo(() => {
    if (branch?.lastToolUsage) return branch.lastToolUsage;
    const first = branchSessions[0];
    if (!first) return null;
    return {
      branch: branch.name,
      worktreePath: branch.worktreePath ?? null,
      toolId:
        first.toolType === "custom"
          ? (first.toolName ?? "custom")
          : (first.toolType as LastToolUsage["toolId"]),
      toolLabel:
        first.toolType === "custom"
          ? (first.toolName ?? "Custom")
          : toolLabel(first.toolType),
      mode: first.mode ?? "normal",
      model: null,
      timestamp: first.startedAt ? Date.parse(first.startedAt) : Date.now(),
    };
  }, [
    branch?.lastToolUsage,
    branch?.name,
    branch?.worktreePath,
    branchSessions,
  ]);

  // Handlers
  const handleCreateWorktree = async () => {
    try {
      await createWorktree.mutateAsync({
        branchName: branch.name,
        createBranch: false,
      });
      setBanner({
        type: "success",
        message: `${branch.name} のWorktreeを作成しました。`,
      });
    } catch (err) {
      setBanner({
        type: "error",
        message: formatError(err, "Worktreeの作成に失敗しました"),
      });
    }
  };

  const handleStartSession = async () => {
    if (!branch.worktreePath) {
      setBanner({
        type: "error",
        message: "Worktreeが存在しないため、先に作成してください。",
      });
      return;
    }

    if (!selectedTool) {
      setBanner({
        type: "error",
        message: "起動するAIツールを選択してください",
      });
      return;
    }

    if (needsRemoteSync) {
      setBanner({
        type: "error",
        message: "リモートの更新を取り込むまでAIツールは起動できません。",
      });
      return;
    }

    if (hasBlockingDivergence) {
      setBanner({
        type: "error",
        message: "差分を解消してから起動してください。",
      });
      return;
    }

    if (
      skipPermissions &&
      !window.confirm("権限チェックをスキップして起動します。続行しますか？")
    ) {
      return;
    }

    setIsStartingSession(true);
    try {
      const toolType: ToolType =
        selectedTool.target === "codex"
          ? "codex-cli"
          : selectedTool.target === "custom"
            ? "custom"
            : "claude-code";
      const extraArgs = extraArgsText
        .split(/\s+/)
        .map((c) => c.trim())
        .filter(Boolean);
      const sessionRequest = {
        toolType,
        toolName: selectedTool.target === "custom" ? selectedTool.id : null,
        ...(selectedTool.target === "custom"
          ? { customToolId: selectedTool.id }
          : {}),
        mode: selectedMode,
        worktreePath: branch.worktreePath,
        skipPermissions,
        ...(selectedTool.target === "codex"
          ? { bypassApprovals: skipPermissions }
          : {}),
        ...(extraArgs.length ? { extraArgs } : {}),
      } as const;

      const session = await startSession.mutateAsync(sessionRequest);
      setActiveSessionId(session.sessionId);
      setIsTerminalFullscreen(false);
      setBanner({
        type: "info",
        message: `${toolLabel(toolType, selectedTool)} を起動しました。`,
      });
    } catch (err) {
      setBanner({
        type: "error",
        message: formatError(err, "セッションの起動に失敗しました"),
      });
    } finally {
      setIsStartingSession(false);
    }
  };

  const handleTerminateSession = async (sessionId: string) => {
    setTerminatingSessionId(sessionId);
    try {
      await deleteSession.mutateAsync(sessionId);
      setBanner({ type: "success", message: "セッションを終了しました" });
      if (activeSessionId === sessionId) setActiveSessionId(null);
    } catch (err) {
      setBanner({
        type: "error",
        message: formatError(err, "セッションの終了に失敗しました"),
      });
    } finally {
      setTerminatingSessionId(null);
    }
  };

  const handleSyncBranch = async () => {
    if (!branch.worktreePath) {
      setBanner({
        type: "error",
        message: "Worktreeが存在しないため同期できません。",
      });
      return;
    }

    try {
      const result = await syncBranch.mutateAsync({
        worktreePath: branch.worktreePath,
      });
      if (result.pullStatus === "success") {
        setBanner({
          type: "success",
          message: "リモートの最新変更を取り込みました。",
        });
      } else {
        const warning =
          result.warnings?.join("\n") ??
          "fast-forward pull が完了しませんでした";
        setBanner({
          type: "error",
          message: `git pull --ff-only が失敗しました。\n${warning}`,
        });
      }
    } catch (err) {
      setBanner({
        type: "error",
        message: formatError(err, "Git同期に失敗しました"),
      });
    }
  };

  const handleSessionExit = (code: number) => {
    setActiveSessionId(null);
    setIsTerminalFullscreen(false);
    setBanner({
      type: code === 0 ? "success" : "error",
      message: `セッションがコード ${code} で終了しました。`,
    });
  };

  return (
    <div className="min-h-screen bg-background">
      {/* Fullscreen backdrop */}
      {isTerminalFullscreen && (
        <div
          className="fixed inset-0 z-40 bg-black/80"
          aria-hidden="true"
          onClick={() => setIsTerminalFullscreen(false)}
        />
      )}

      {/* Header */}
      <PageHeader
        eyebrow="BRANCH DETAIL"
        title={branch.name}
        subtitle={`最新コミット ${branch.commitHash.slice(0, 7)} · ${formattedCommitDate}`}
      >
        <div className="mt-4 flex flex-wrap gap-2">
          <Badge variant={branch.type === "local" ? "local" : "remote"}>
            {BRANCH_TYPE_LABEL[branch.type]}
          </Badge>
          <Badge
            variant={
              branch.mergeStatus === "merged"
                ? "success"
                : branch.mergeStatus === "unmerged"
                  ? "warning"
                  : "outline"
            }
          >
            {MERGE_STATUS_LABEL[branch.mergeStatus]}
          </Badge>
          <Badge variant={branch.worktreePath ? "success" : "outline"}>
            {branch.worktreePath ? "Worktreeあり" : "Worktree未作成"}
          </Badge>
        </div>
        <div className="mt-4 flex flex-wrap gap-2">
          <Button variant="ghost" size="sm" asChild>
            <Link to="/">← ブランチ一覧</Link>
          </Button>
          {!canStartSession ? (
            <Button
              onClick={handleCreateWorktree}
              disabled={createWorktree.isPending}
            >
              {createWorktree.isPending ? "作成中..." : "Worktreeを作成"}
            </Button>
          ) : (
            <Button variant="secondary" asChild>
              <Link to="/config">カスタムツール設定</Link>
            </Button>
          )}
        </div>
      </PageHeader>

      {/* Banner */}
      {banner && (
        <div className="mx-auto max-w-7xl px-6 pt-4">
          <Alert
            variant={
              banner.type === "error"
                ? "destructive"
                : banner.type === "success"
                  ? "success"
                  : "info"
            }
          >
            <AlertDescription>{banner.message}</AlertDescription>
          </Alert>
        </div>
      )}

      {/* Main Content */}
      <main className="mx-auto max-w-7xl space-y-6 px-6 py-8">
        <div className="grid gap-6 lg:grid-cols-[1fr_400px]">
          {/* Left Column - Tool Launcher & Session History */}
          <div className="space-y-6">
            <ToolLauncher
              branch={branch}
              availableTools={availableTools}
              selectedToolId={selectedToolId}
              selectedMode={selectedMode}
              skipPermissions={skipPermissions}
              extraArgsText={extraArgsText}
              isConfigLoading={isConfigLoading}
              configError={configError ?? null}
              isStartingSession={isStartingSession}
              isSyncingBranch={isSyncingBranch}
              needsRemoteSync={needsRemoteSync}
              hasBlockingDivergence={hasBlockingDivergence}
              onToolChange={setSelectedToolId}
              onModeChange={setSelectedMode}
              onSkipPermissionsChange={setSkipPermissions}
              onExtraArgsChange={setExtraArgsText}
              onStartSession={handleStartSession}
              onSyncBranch={handleSyncBranch}
            />

            <SessionHistoryTable
              sessions={branchSessions}
              isLoading={isSessionsLoading}
              terminatingSessionId={terminatingSessionId}
              isDeleting={deleteSession.isPending}
              onTerminate={handleTerminateSession}
              onSelectSession={setActiveSessionId}
            />

            <BranchInfoCards
              branch={branch}
              formattedCommitDate={formattedCommitDate}
              latestToolUsage={latestToolUsage}
            />
          </div>

          {/* Right Column - Terminal */}
          <div className="lg:sticky lg:top-6 lg:self-start">
            <TerminalPanel
              sessionId={activeSessionId}
              isFullscreen={isTerminalFullscreen}
              onToggleFullscreen={() =>
                setIsTerminalFullscreen((prev) => !prev)
              }
              onExit={handleSessionExit}
              onError={(message) =>
                setBanner({ type: "error", message: message ?? "不明なエラー" })
              }
            />
          </div>
        </div>
      </main>
    </div>
  );
}

// Helper functions
function formatDate(value?: string | null): string {
  if (!value) return "日時不明";
  try {
    return new Intl.DateTimeFormat("ja-JP", {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }).format(new Date(value));
  } catch {
    return value;
  }
}

function formatError(error: unknown, fallback: string): string {
  if (error instanceof ApiError) {
    return `${error.message}${error.details ? `\n${error.details}` : ""}`;
  }
  if (error instanceof Error) return error.message;
  return fallback;
}

function toolLabel(tool: string, selectedTool?: SelectableTool): string {
  if (tool === "custom" && selectedTool?.target === "custom")
    return selectedTool.label;
  if (tool === "codex-cli") return "Codex CLI";
  return "Claude Code";
}
