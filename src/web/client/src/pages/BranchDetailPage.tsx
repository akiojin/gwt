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
import { Terminal } from "../components/Terminal";
import type { Branch, CustomAITool } from "../../../../types/api.js";

type ToolType = "claude-code" | "codex-cli" | "custom";
type ToolMode = "normal" | "continue" | "resume";

type SelectableTool =
  | { id: "claude-code"; label: string; target: "claude" }
  | { id: "codex-cli"; label: string; target: "codex" }
  | { id: string; label: string; target: "custom"; definition: CustomAITool };

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

const BUILTIN_TOOL_SUMMARIES: Record<string, ToolSummary> = {
  "claude-code": {
    command: "claude",
    defaultArgs: [],
    modeArgs: {
      normal: [],
      continue: ["-c"],
      resume: ["-r"],
    },
    permissionSkipArgs: ["--dangerously-skip-permissions"],
  },
  "codex-cli": {
    command: "codex",
    defaultArgs: ["--auto-approve", "--verbose"],
    modeArgs: {
      normal: [],
      continue: ["resume", "--last"],
      resume: ["resume"],
    },
  },
};

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

const MERGE_STATUS_TONE: Record<Branch["mergeStatus"], "success" | "warning" | "muted"> = {
  merged: "success",
  unmerged: "warning",
  unknown: "muted",
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
  const [terminatingSessionId, setTerminatingSessionId] = useState<string | null>(null);

  const formattedCommitDate = useMemo(
    () => formatDate(branch?.commitDate),
    [branch?.commitDate],
  );

  useEffect(() => {
    if (!isTerminalFullscreen) {
      return undefined;
    }

    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";

    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, [isTerminalFullscreen]);

  // Hook values that don't depend on branch existence must be declared before the early returns
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

  const branchSessions = useMemo(() => {
    return (sessionsData ?? [])
      .filter((session) => session.worktreePath === branch?.worktreePath)
      .sort((a, b) => (b.startedAt ?? "").localeCompare(a.startedAt ?? ""));
  }, [sessionsData, branch?.worktreePath]);

  if (isLoading) {
    return (
      <div className="app-shell">
        <div className="page-state page-state--centered">
          <h1>読み込み中</h1>
          <p>ブランチ情報を取得しています...</p>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="app-shell">
        <div className="page-state page-state--centered">
          <h1>ブランチの取得に失敗しました</h1>
          <p>{error instanceof Error ? error.message : "未知のエラーです"}</p>
          <Link to="/" className="button button--ghost">
            ブランチ一覧に戻る
          </Link>
        </div>
      </div>
    );
  }

  if (!branch) {
    return (
      <div className="app-shell">
        <div className="page-state page-state--centered">
          <h1>Branch not found</h1>
          <p>指定されたブランチは存在しません。</p>
          <Link to="/" className="button button--ghost">
            ブランチ一覧に戻る
          </Link>
        </div>
      </div>
    );
  }

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
      setBanner({ type: "error", message: "起動するAIツールを選択してください" });
      return;
    }

    if (needsRemoteSync) {
      setBanner({
        type: "error",
        message: "リモートの更新を取り込むまでAIツールは起動できません。『最新の変更を同期』を実行してください。",
      });
      return;
    }

    if (hasBlockingDivergence) {
      setBanner({
        type: "error",
        message:
          "リモートとローカルの双方で進捗が発生しているため、CLIと同様にAIツールの起動をブロックしました。先に rebase/merge 等で差分を解消してください。",
      });
      return;
    }

    if (skipPermissions && !window.confirm("権限チェックをスキップして起動します。自己責任で実行してください。続行しますか？")) {
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
      const extraArgs = parseExtraArgs(extraArgsText);
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
      if (activeSessionId === sessionId) {
        setActiveSessionId(null);
      }
    } catch (err) {
      setBanner({ type: "error", message: formatError(err, "セッションの終了に失敗しました") });
    } finally {
      setTerminatingSessionId(null);
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
        setBanner({
          type: "error",
          message: `git pull --ff-only が失敗しました。\n${warning}`,
        });
      }
    } catch (err) {
      setBanner({ type: "error", message: formatError(err, "Git同期に失敗しました") });
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
    <div className="app-shell">
      <header className="page-hero page-hero--compact">
        <Link to="/" className="link-back">
          ← ブランチ一覧に戻る
        </Link>
        <p className="page-hero__eyebrow">BRANCH DETAIL</p>
        <h1>{branch.name}</h1>
        <p className="page-hero__subtitle">
          最新コミット {branch.commitHash.slice(0, 7)} ・ {formattedCommitDate}
        </p>
        <div className="badge-group">
          <span className={`status-badge status-badge--${branch.type}`}>
            {BRANCH_TYPE_LABEL[branch.type]}
          </span>
          <span className={`status-badge status-badge--${MERGE_STATUS_TONE[branch.mergeStatus]}`}>
            {MERGE_STATUS_LABEL[branch.mergeStatus]}
          </span>
          <span
            className={`status-badge ${
              branch.worktreePath ? "status-badge--success" : "status-badge--muted"
            }`}
          >
            {branch.worktreePath ? "Worktreeあり" : "Worktree未作成"}
          </span>
        </div>
        <div className="page-hero__actions">
          {!canStartSession ? (
            <button
              type="button"
              className="button button--primary"
              onClick={handleCreateWorktree}
              disabled={createWorktree.isPending}
            >
              {createWorktree.isPending ? "作成中..." : "Worktreeを作成"}
            </button>
          ) : (
            <Link to="/config" className="button button--secondary">
              カスタムツール設定を開く
            </Link>
          )}
        </div>

        {banner && (
          <div className={`inline-banner inline-banner--${banner.type}`}>
            {banner.message}
          </div>
        )}
      </header>

      {isTerminalFullscreen && (
        <div
          className="terminal-overlay-backdrop"
          aria-hidden="true"
          onClick={() => setIsTerminalFullscreen(false)}
        />
      )}
      <main className="page-content page-content--wide">
        <div className="page-layout page-layout--split">
          <div className="info-stack">
            <section className="section-card">
              <header className="terminal-section__header">
                <div>
                  <h2>AIツール起動</h2>
                  <p className="section-card__body">
                    Web UI から直接AIツールを起動できます。設定したカスタムツールも一覧に表示されます。
                  </p>
                </div>
                {configError && (
                  <span className="pill pill--warning">設定の取得に失敗しました</span>
                )}
              </header>

              {!canStartSession ? (
                <p className="section-card__body">
                  Worktreeが未作成のため、先にWorktreeを作成してください。
                </p>
              ) : (
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
                  {skipPermissions && (
                    <div className="inline-banner inline-banner--warning">
                      <p>
                        権限チェックをスキップすることで、CLI での `--dangerously-skip-permissions` 指定と同様のリスクを負います。
                      </p>
                    </div>
                  )}
                  {needsRemoteSync && (
                    <div className="inline-banner inline-banner--info" data-testid="sync-required">
                      <p>
                        リモートに未取得の更新 ({branch.divergence?.behind ?? 0} commits) があるため、AIツールを起動する前に同期してください。
                      </p>
                      <p className="section-card__body">
                        CLI の `git fetch --all` と `git pull --ff-only` と同じ処理を Web UI から実行できます。
                      </p>
                    </div>
                  )}
                  {hasBlockingDivergence && (
                    <div className="inline-banner inline-banner--warning" data-testid="divergence-warning">
                      <p>
                        リモートとローカルの両方に未解決の差分があるため、Web UI でも CLI と同様に起動をブロックしています。
                      </p>
                      <ul className="list-muted">
                        <li>git fetch && git pull --ff-only origin {branch.name}</li>
                        <li>必要に応じて git push origin {branch.name} でローカル進捗を共有</li>
                      </ul>
                      <p className="section-card__body">
                        rebase / merge などで差分を解消した後にページを更新してください。
                      </p>
                    </div>
                  )}

                  <div className="tool-card__actions">
                    <button
                      type="button"
                      className="button button--primary"
                      onClick={handleStartSession}
                      disabled={
                        isStartingSession ||
                        !selectedTool ||
                        hasBlockingDivergence ||
                        needsRemoteSync ||
                        isSyncingBranch
                      }
                    >
                      {isStartingSession ? "起動中..." : "セッションを起動"}
                    </button>
                    <button
                      type="button"
                      className="button button--secondary"
                      onClick={handleSyncBranch}
                      disabled={!branch.worktreePath || isSyncingBranch}
                    >
                      {isSyncingBranch ? "同期中..." : "最新の変更を同期"}
                    </button>
                    <Link to="/config" className="button button--ghost">
                      設定を編集
                    </Link>
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
              )}
            </section>
            <section className="section-card">
              <header className="terminal-section__header">
                <div>
                  <h2>セッション履歴</h2>
                  <p className="section-card__body">
                    この Worktree に紐づいた最新の AI セッションが表示されます。CLI からの起動分も共有されます。
                  </p>
                </div>
                {isSessionsLoading && <span className="pill">読み込み中...</span>}
              </header>
              {branchSessions.length === 0 ? (
                <p className="section-card__body">セッション履歴はまだありません。</p>
              ) : (
                <div className="session-table-wrapper">
                  <table className="session-table">
                    <thead>
                      <tr>
                        <th>状態</th>
                        <th>ツール</th>
                        <th>モード</th>
                        <th>開始時刻</th>
                        <th>終了時刻</th>
                        <th>操作</th>
                      </tr>
                    </thead>
                    <tbody>
                      {branchSessions.slice(0, 5).map((session) => (
                        <tr key={session.sessionId}>
                          <td>
                            <span className={`status-pill status-pill--${session.status}`}>
                              {SESSION_STATUS_LABEL[session.status]}
                            </span>
                          </td>
                          <td>{session.toolType === "custom" ? session.toolName ?? "custom" : toolLabel(session.toolType)}</td>
                          <td>{session.mode}</td>
                          <td>{formatDate(session.startedAt)}</td>
                          <td>{session.endedAt ? formatDate(session.endedAt) : "--"}</td>
                          <td>
                            {session.status === "running" ? (
                              <button
                                type="button"
                                className="button button--ghost"
                                onClick={() => handleTerminateSession(session.sessionId)}
                                disabled={terminatingSessionId === session.sessionId || deleteSession.isPending}
                              >
                                {terminatingSessionId === session.sessionId ? "終了中..." : "終了"}
                              </button>
                            ) : (
                              <span className="session-table__muted">--</span>
                            )}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
            <section className="section-card">
              <header>
                <h2>ブランチインサイト</h2>
              </header>
              <dl className="metadata-grid">
                <div>
                  <dt>コミット</dt>
                  <dd>{branch.commitHash}</dd>
                </div>
                <div>
                  <dt>Author</dt>
                  <dd>{branch.author ?? "N/A"}</dd>
                </div>
                <div>
                  <dt>更新日</dt>
                  <dd>{formattedCommitDate}</dd>
                </div>
                <div>
                  <dt>Worktree</dt>
                  <dd>{branch.worktreePath ?? "未作成"}</dd>
                </div>
              </dl>
            </section>

            <section className="section-card">
              <header>
                <h2>コミット情報</h2>
              </header>
              <p className="section-card__body">
                {branch.commitMessage ?? "コミットメッセージがありません。"}
              </p>
            </section>

            {branch.divergence && (
              <section className="section-card">
                <header>
                  <h2>差分状況</h2>
                </header>
                <div className="pill-group">
                  <span className="pill">Ahead {branch.divergence.ahead}</span>
                  <span className="pill">Behind {branch.divergence.behind}</span>
                  <span
                    className={`pill ${
                      branch.divergence.upToDate ? "pill--success" : "pill--warning"
                    }`}
                  >
                    {branch.divergence.upToDate ? "最新" : "更新あり"}
                  </span>
                </div>
              </section>
            )}

            <section className="section-card">
              <header>
                <h2>Worktree情報</h2>
              </header>
              <ul className="list-muted">
                <li>
                  パス: <strong>{branch.worktreePath ?? "未作成"}</strong>
                </li>
                <li>AIツールの起動にはクリーンなワークツリーであることを推奨します。</li>
                <li>Worktreeを再作成すると既存のローカル変更が失われる可能性があります。</li>
              </ul>
            </section>
          </div>

          <div className="terminal-column">
            {activeSessionId ? (
              <section
                className={`section-card terminal-section ${
                  isTerminalFullscreen ? "terminal-section--fullscreen" : ""
                }`}
                data-testid="active-terminal"
              >
                <div className="terminal-section__header">
                  <div>
                    <h2>ターミナルセッション</h2>
                    <p className="section-card__body">
                      出力はリアルタイムにストリームされます。終了するとこのパネルは自動で閉じます。
                    </p>
                  </div>
                  <div className="terminal-section__controls">
                    <button
                      type="button"
                      className="button button--ghost"
                      onClick={() => setIsTerminalFullscreen((prev) => !prev)}
                    >
                      {isTerminalFullscreen ? "通常表示に戻す" : "ターミナルを最大化"}
                    </button>
                  </div>
                </div>
                <div className="terminal-surface">
                  <Terminal
                    sessionId={activeSessionId}
                    onExit={handleSessionExit}
                    onError={(message) =>
                      setBanner({ type: "error", message: message ?? "不明なエラー" })
                    }
                  />
                </div>
                {isTerminalFullscreen && (
                  <button
                    type="button"
                    className="terminal-section__close"
                    aria-label="ターミナルを閉じる"
                    onClick={() => setIsTerminalFullscreen(false)}
                  >
                    ×
                  </button>
                )}
              </section>
            ) : (
              <section className="section-card session-hint">
                <header>
                  <h2>セッションは未起動</h2>
                </header>
                <p className="section-card__body">
                  上部のアクションからAIツールを起動すると、このエリアにターミナルが表示されます。
                </p>
              </section>
            )}
          </div>
        </div>
      </main>
    </div>
  );
}

function formatDate(value?: string | null) {
  if (!value) {
    return "日時不明";
  }

  try {
    const date = new Date(value);
    return new Intl.DateTimeFormat("ja-JP", {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }).format(date);
  } catch (_err) {
    return value;
  }
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

function toolLabel(tool: ToolType, selectedTool?: SelectableTool) {
  if (tool === "custom" && selectedTool?.target === "custom") {
    return selectedTool.label;
  }
  if (tool === "codex-cli") {
    return "Codex CLI";
  }
  return "Claude Code";
}

function renderArgs(args?: string[] | null) {
  if (!args || args.length === 0) {
    return <span className="tool-card__muted">未設定</span>;
  }
  return args.join(" ");
}

const SESSION_STATUS_LABEL: Record<"pending" | "running" | "completed" | "failed", string> = {
  pending: "pending",
  running: "running",
  completed: "completed",
  failed: "failed",
};

function parseExtraArgs(value: string): string[] {
  return value
    .split(/\s+/)
    .map((chunk) => chunk.trim())
    .filter(Boolean);
}
