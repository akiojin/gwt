import React, { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import type { Branch, CustomAITool } from "../../../../types/api.js";
import {
  CLAUDE_PERMISSION_SKIP_ARGS,
  CODEX_DEFAULT_ARGS,
} from "../../../../shared/aiToolConstants.js";
import { useConfig } from "../hooks/useConfig";
import { useStartSession } from "../hooks/useSessions";
import { useCreateWorktree } from "../hooks/useWorktrees";
import { useSyncBranch } from "../hooks/useBranches";
import { ApiError } from "../lib/api";

const BUILTIN_TOOL_SUMMARIES: Record<string, ToolSummary> = {
  "claude-code": {
    command: "claude",
    defaultArgs: [],
    modeArgs: {
      normal: [],
      continue: ["-c"],
      resume: ["-r"],
    },
    permissionSkipArgs: Array.from(CLAUDE_PERMISSION_SKIP_ARGS),
  },
  "codex-cli": {
    command: "codex",
    defaultArgs: Array.from(CODEX_DEFAULT_ARGS),
    modeArgs: {
      normal: [],
      continue: ["resume", "--last"],
      resume: ["resume"],
    },
  },
};

interface ToolSummary {
  command: string;
  defaultArgs?: string[] | null;
  modeArgs?: {
    normal?: string[];
    continue?: string[];
    resume?: string[];
  };
  permissionSkipArgs?: string[] | null;
}

interface AIToolLaunchModalProps {
  branch: Branch;
  onClose: () => void;
}

type ToolMode = "normal" | "continue" | "resume";

type SelectableTool =
  | { id: "claude-code"; label: string; target: "claude" }
  | { id: "codex-cli"; label: string; target: "codex" }
  | { id: string; label: string; target: "custom"; definition: CustomAITool };

export function AIToolLaunchModal({ branch, onClose }: AIToolLaunchModalProps) {
  const { data: config, isLoading: isConfigLoading, error: configError } = useConfig();
  const startSession = useStartSession();
  const createWorktree = useCreateWorktree();
  const syncBranch = useSyncBranch(branch.name);
  const navigate = useNavigate();

  const [selectedToolId, setSelectedToolId] = useState<string>("claude-code");
  const [selectedMode, setSelectedMode] = useState<ToolMode>("normal");
  const [skipPermissions, setSkipPermissions] = useState(false);
  const [extraArgsText, setExtraArgsText] = useState("");
  const [banner, setBanner] = useState<{ type: "success" | "error" | "info"; message: string } | null>(null);
  const [isStartingSession, setIsStartingSession] = useState(false);
  const [isCreatingWorktree, setIsCreatingWorktree] = useState(false);

  const customTools = config?.tools ?? [];
  const availableTools: SelectableTool[] = useMemo(
    () => [
      { id: "claude-code", label: "Claude Code", target: "claude" },
      { id: "codex-cli", label: "Codex CLI", target: "codex" },
      ...customTools.map((tool) => ({
        id: tool.id,
        label: tool.displayName,
        target: "custom" as const,
        definition: tool,
      })),
    ],
    [customTools],
  );

  useEffect(() => {
    if (!availableTools.length) {
      setSelectedToolId("claude-code");
      return;
    }
    if (!availableTools.find((tool) => tool.id === selectedToolId)) {
      const first = availableTools[0];
      if (first) {
        setSelectedToolId(first.id);
      }
    }
  }, [availableTools, selectedToolId]);

  const selectedTool = availableTools.find((tool) => tool.id === selectedToolId);

  const selectedToolSummary: ToolSummary | null = useMemo(() => {
    if (!selectedTool) {
      return null;
    }
    if (selectedTool.target === "custom") {
      return {
        command: selectedTool.definition.command,
        defaultArgs: selectedTool.definition.defaultArgs ?? null,
        modeArgs: selectedTool.definition.modeArgs,
        permissionSkipArgs: selectedTool.definition.permissionSkipArgs ?? null,
      };
    }
    return BUILTIN_TOOL_SUMMARIES[selectedTool.id] ?? null;
  }, [selectedTool]);

  const argsPreview = useMemo(() => {
    if (!selectedToolSummary) {
      return null;
    }
    const args: string[] = [];
    if (selectedToolSummary.defaultArgs?.length) {
      args.push(...selectedToolSummary.defaultArgs);
    }
    const mode = selectedToolSummary.modeArgs?.[selectedMode];
    if (mode?.length) {
      args.push(...mode);
    }
    if (skipPermissions && selectedToolSummary.permissionSkipArgs?.length) {
      args.push(...selectedToolSummary.permissionSkipArgs);
    }
    const extraArgs = parseExtraArgs(extraArgsText);
    if (extraArgs.length) {
      args.push(...extraArgs);
    }
    return { command: selectedToolSummary.command, args };
  }, [selectedToolSummary, selectedMode, skipPermissions, extraArgsText]);

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

  useEffect(() => {
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, []);

  const handleClose = () => {
    setBanner(null);
    onClose();
  };

  const handleCreateWorktree = async () => {
    setIsCreatingWorktree(true);
    try {
      await createWorktree.mutateAsync({ branchName: branch.name, createBranch: false });
      setBanner({ type: "success", message: `${branch.name} のWorktreeを作成しました。再度同期を実行してください。` });
    } catch (error) {
      setBanner({ type: "error", message: formatError(error, "Worktreeの作成に失敗しました") });
    } finally {
      setIsCreatingWorktree(false);
    }
  };

  const handleSyncBranch = async () => {
    if (!branch.worktreePath) {
      setBanner({ type: "error", message: "Worktreeが存在しないため同期できません。" });
      return;
    }
    try {
      const result = await syncBranch.mutateAsync({ worktreePath: branch.worktreePath });
      if (result.pullStatus === "success") {
        setBanner({ type: "success", message: "リモートの最新変更を取り込みました。" });
      } else {
        const warning = result.warnings?.join("\n") ?? "fast-forward pull が完了しませんでした";
        setBanner({ type: "error", message: `git pull --ff-only が失敗しました。\n${warning}` });
      }
    } catch (error) {
      setBanner({ type: "error", message: formatError(error, "Git同期に失敗しました") });
    }
  };

  const handleStartSession = async () => {
    if (!branch.worktreePath) {
      setBanner({ type: "error", message: "Worktreeが存在しないため、先に作成してください。" });
      return;
    }
    if (!selectedTool) {
      setBanner({ type: "error", message: "起動するAIツールを選択してください" });
      return;
    }
    if (needsRemoteSync) {
      setBanner({ type: "error", message: "リモートの更新を取り込むまでAIツールは起動できません。同期を実行してください。" });
      return;
    }
    if (hasBlockingDivergence) {
      setBanner({
        type: "error",
        message: "リモートとローカルの双方で進捗が発生しているため起動できません。rebase/merge 等で差分を解消してください。",
      });
      return;
    }

    if (skipPermissions && !window.confirm("権限チェックをスキップして起動します。自己責任で実行してください。続行しますか？")) {
      return;
    }

    setIsStartingSession(true);
    try {
      const toolType =
        selectedTool.target === "codex"
          ? "codex-cli"
          : selectedTool.target === "custom"
            ? "custom"
            : "claude-code";
      const extraArgs = parseExtraArgs(extraArgsText);
      const sessionRequest = {
        toolType,
        toolName: selectedTool.target === "custom" ? selectedTool.id : null,
        ...(selectedTool.target === "custom" ? { customToolId: selectedTool.id } : {}),
        mode: selectedMode,
        worktreePath: branch.worktreePath,
        skipPermissions,
        ...(selectedTool.target === "codex" ? { bypassApprovals: skipPermissions } : {}),
        ...(extraArgs.length ? { extraArgs } : {}),
      } as const;

      await startSession.mutateAsync(sessionRequest);
      setBanner({ type: "success", message: "AIツールを起動しました。セッション画面で出力を確認してください。" });
      navigate(`/${encodeURIComponent(branch.name)}`);
    } catch (error) {
      setBanner({ type: "error", message: formatError(error, "セッションの起動に失敗しました") });
    } finally {
      setIsStartingSession(false);
    }
  };

  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal" role="document">
        <div className="modal__header">
          <div>
            <p className="tool-card__eyebrow">Launch AI Tool</p>
            <h2>{branch.name}</h2>
          </div>
          <button type="button" className="button button--ghost" onClick={handleClose}>
            ×
          </button>
        </div>

        {banner && (
          <div className={`inline-banner inline-banner--${banner.type}`}>
            {banner.message}
          </div>
        )}

        {configError && (
          <div className="inline-banner inline-banner--warning">
            設定の取得に失敗しました: {configError instanceof Error ? configError.message : "unknown"}
          </div>
        )}

        {!branch.worktreePath && (
          <div className="inline-banner inline-banner--warning">
            <p>Worktreeが未作成のため、AIツール起動の前に作成してください。</p>
            <button
              type="button"
              className="button button--secondary"
              onClick={handleCreateWorktree}
              disabled={isCreatingWorktree}
            >
              {isCreatingWorktree ? "作成中..." : "Worktreeを作成"}
            </button>
          </div>
        )}

        {needsRemoteSync && (
          <div className="inline-banner inline-banner--info">
            リモートに未取得の更新 ({branch.divergence?.behind ?? 0} commits) があるため、同期が必要です。
          </div>
        )}

        {hasBlockingDivergence && (
          <div className="inline-banner inline-banner--warning">
            リモートとローカルの両方に未解決の差分が存在します。rebase/merge 等で解消してください。
          </div>
        )}

        <div className="tool-form">
          <div className="form-grid">
            <label className="form-field">
              <span>AIツール</span>
              <select
                value={selectedToolId}
                onChange={(event) => setSelectedToolId(event.target.value)}
                disabled={isConfigLoading}
              >
                {availableTools.map((tool) => (
                  <option key={tool.id} value={tool.id}>
                    {tool.label}
                  </option>
                ))}
              </select>
            </label>

            <label className="form-field">
              <span>起動モード</span>
              <select
                value={selectedMode}
                onChange={(event) => setSelectedMode(event.target.value as ToolMode)}
              >
                <option value="normal">normal</option>
                <option value="continue">continue</option>
                <option value="resume">resume</option>
              </select>
            </label>

            <label className="form-field">
              <span>追加引数 (スペース区切り)</span>
              <input
                type="text"
                value={extraArgsText}
                onChange={(event) => setExtraArgsText(event.target.value)}
                placeholder="--flag value"
              />
            </label>
          </div>

          <label className="form-field">
            <span>
              <input
                type="checkbox"
                checked={skipPermissions}
                onChange={(event) => setSkipPermissions(event.target.checked)}
              />
              <span style={{ marginLeft: "0.5rem" }}>権限チェックをスキップ (自己責任)</span>
            </span>
          </label>

          <div className="tool-card__actions">
            <button
              type="button"
              className="button button--primary"
              onClick={handleStartSession}
              disabled={isStartingSession || !selectedTool || hasBlockingDivergence || needsRemoteSync}
            >
              {isStartingSession ? "起動中..." : "セッションを起動"}
            </button>
            <button
              type="button"
              className="button button--secondary"
              onClick={handleSyncBranch}
              disabled={!branch.worktreePath || syncBranch.isPending}
            >
              {syncBranch.isPending ? "同期中..." : "最新の変更を同期"}
            </button>
            <button type="button" className="button button--ghost" onClick={handleClose}>
              キャンセル
            </button>
          </div>

          {selectedToolSummary && (
            <dl className="metadata-grid metadata-grid--compact">
              <div>
                <dt>コマンド</dt>
                <dd className="tool-card__command">{selectedToolSummary.command}</dd>
              </div>
              <div>
                <dt>defaultArgs</dt>
                <dd>{renderArgs(selectedToolSummary.defaultArgs)}</dd>
              </div>
              <div>
                <dt>permissionSkipArgs</dt>
                <dd>{renderArgs(selectedToolSummary.permissionSkipArgs)}</dd>
              </div>
              {argsPreview && (
                <div className="metadata-grid__full">
                  <dt>最終的に実行されるコマンド</dt>
                  <dd className="tool-card__command">
                    {argsPreview.command} {argsPreview.args.join(" ")}
                  </dd>
                </div>
              )}
            </dl>
          )}
        </div>
      </div>
    </div>
  );
}

function parseExtraArgs(value: string): string[] {
  return value
    .split(/\s+/)
    .map((chunk) => chunk.trim())
    .filter(Boolean);
}

function renderArgs(args?: string[] | null) {
  if (!args || args.length === 0) {
    return <span className="tool-card__muted">未設定</span>;
  }
  return args.join(" ");
}

function formatError(error: unknown, fallback: string) {
  if (error instanceof ApiError) {
    return `${error.message}${error.details ? `\n${error.details}` : ""}`;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return fallback;
}
