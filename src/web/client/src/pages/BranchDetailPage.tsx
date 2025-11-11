import React, { useEffect, useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { useBranch } from "../hooks/useBranches";
import { useCreateWorktree } from "../hooks/useWorktrees";
import { useSessions, useDeleteSession } from "../hooks/useSessions";
import { useConfig } from "../hooks/useConfig";
import { ApiError } from "../lib/api";
import { Terminal } from "../components/Terminal";
import type { Branch } from "../../../../types/api.js";

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
  const createWorktree = useCreateWorktree();
  const { data: sessionsData, isLoading: isSessionsLoading } = useSessions();
  const deleteSession = useDeleteSession();
  const { data: config, error: configError } = useConfig();

  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [banner, setBanner] = useState<BannerState | null>(null);
  const [isTerminalFullscreen, setIsTerminalFullscreen] = useState(false);
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

  const builtinTools = useMemo(
    () => [
      { id: "claude-code", label: "Claude Code" },
      { id: "codex-cli", label: "Codex CLI" },
    ],
    [],
  );
  const customTools = config?.tools ?? [];
  const branchSessions = useMemo(() => {
    return (sessionsData ?? [])
      .filter((session) => session.worktreePath === branch?.worktreePath)
      .sort((a, b) => (b.startedAt ?? "").localeCompare(a.startedAt ?? ""));
  }, [sessionsData, branch?.worktreePath]);

  useEffect(() => {
    if (!activeSessionId) {
      const running = branchSessions.find((session) => session.status === "running");
      if (running) {
        setActiveSessionId(running.sessionId);
      }
    }
  }, [branchSessions, activeSessionId]);

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
        message: err instanceof Error ? err.message : "Worktreeの作成に失敗しました",
      });
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

  

  const handleSessionExit = (code: number) => {
    setActiveSessionId(null);
    setIsTerminalFullscreen(false);
    setBanner({
      type: code === 0 ? "success" : "error",
      message: `セッションがコード ${code} で終了しました。`,
    });
  };

  const handleFocusSession = (sessionId: string) => {
    setActiveSessionId(sessionId);
    setIsTerminalFullscreen(false);
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
                  <h2>AIツール設定</h2>
                  <p className="section-card__body">
                    AIツールの起動はブランチ一覧画面でモーダルを開き、ツールとモードを選択して実行してください。
                    この画面では現在のブランチ情報とセッションの状態のみを参照できます。
                  </p>
                </div>
                {configError && (
                  <span className="pill pill--warning">設定の取得に失敗しました</span>
                )}
              </header>

              {!branch.worktreePath && (
                <div className="inline-banner inline-banner--warning">
                  Worktreeが未作成のため、AIツールを起動する前にブランチ一覧から作成してください。
                </div>
              )}

              <div className="tool-summary-grid">
                <div>
                  <p className="tool-card__eyebrow">ビルトインツール</p>
                  <ul className="tool-summary-list">
                    {builtinTools.map((tool) => (
                      <li key={tool.id}>{tool.label}</li>
                    ))}
                  </ul>
                </div>
                <div>
                  <p className="tool-card__eyebrow">カスタムツール</p>
                  {customTools.length === 0 ? (
                    <p className="tool-card__muted">登録されたカスタムツールはありません。</p>
                  ) : (
                    <ul className="tool-summary-list">
                      {customTools.map((tool) => (
                        <li key={tool.id}>{tool.displayName}</li>
                      ))}
                    </ul>
                  )}
                </div>
              </div>

              <div className="tool-card__actions">
                <Link to="/" className="button button--secondary">
                  ブランチ一覧を開く
                </Link>
                <Link to="/config" className="button button--ghost">
                  カスタムツール設定
                </Link>
              </div>
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
                        <tr key={session.sessionId} data-testid="session-row">
                          <td>
                            <span className={`status-pill status-pill--${session.status}`}>
                              {SESSION_STATUS_LABEL[session.status]}
                            </span>
                          </td>
                          <td>{sessionToolLabel(session)}</td>
                          <td>{session.mode}</td>
                          <td>{formatDate(session.startedAt)}</td>
                          <td>{session.endedAt ? formatDate(session.endedAt) : "--"}</td>
                          <td>
                            {session.status === "running" ? (
                              <div className="session-table__actions">
                                <button
                                  type="button"
                                  className="button button--secondary"
                                  onClick={() => handleFocusSession(session.sessionId)}
                                  data-testid="session-focus-button"
                                >
                                  表示
                                </button>
                                <button
                                  type="button"
                                  className="button button--ghost"
                                  onClick={() => handleTerminateSession(session.sessionId)}
                                  disabled={terminatingSessionId === session.sessionId || deleteSession.isPending}
                                  data-testid="session-stop-button"
                                >
                                  {terminatingSessionId === session.sessionId ? "終了中..." : "終了"}
                                </button>
                              </div>
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
                  ブランチ一覧でAIツールを起動し、稼働中のセッションをここから選択するとライブ出力が表示されます。
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

const SESSION_STATUS_LABEL: Record<"pending" | "running" | "completed" | "failed", string> = {
  pending: "pending",
  running: "running",
  completed: "completed",
  failed: "failed",
};

function sessionToolLabel(session: {
  toolType: "claude-code" | "codex-cli" | "custom";
  toolName?: string | null;
}): string {
  if (session.toolType === "custom") {
    return session.toolName ?? "Custom Tool";
  }
  return session.toolType === "codex-cli" ? "Codex CLI" : "Claude Code";
}
