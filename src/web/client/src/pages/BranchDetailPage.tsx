import React, { useEffect, useMemo, useState } from "react";
import { Link, useLocation, useNavigate, useParams } from "react-router-dom";
import { useBranch } from "../hooks/useBranches";
import { useCreateWorktree } from "../hooks/useWorktrees";
import { useSessions, useDeleteSession } from "../hooks/useSessions";
import { useConfig } from "../hooks/useConfig";
import { ApiError } from "../lib/api";
import { Terminal } from "../components/Terminal";
import type { Branch } from "../../../../types/api.js";
import { useQueryClient } from "@tanstack/react-query";

interface BannerState {
  type: "success" | "error" | "info";
  message: string;
}

const BRANCH_TYPE_LABEL: Record<Branch["type"], string> = {
  local: "Local",
  remote: "Remote",
};

const MERGE_STATUS_LABEL: Record<Branch["mergeStatus"], string> = {
  merged: "Merged",
  unmerged: "Not merged",
  unknown: "Unknown",
};

const MERGE_STATUS_TONE: Record<Branch["mergeStatus"], "success" | "warning" | "muted"> = {
  merged: "success",
  unmerged: "warning",
  unknown: "muted",
};

export function BranchDetailPage() {
  const { branchName } = useParams<{ branchName: string }>();
  const decodedBranchName = branchName ? decodeURIComponent(branchName) : "";
  const navigate = useNavigate();
  const location = useLocation();
  const queryClient = useQueryClient();

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

  const focusSessionId = (location.state as { focusSessionId?: string } | null)?.focusSessionId;

  useEffect(() => {
    if (!focusSessionId) {
      return;
    }
    setActiveSessionId(focusSessionId);
    setIsTerminalFullscreen(false);
    navigate(location.pathname + location.search, { replace: true, state: {} });
  }, [focusSessionId, navigate, location.pathname, location.search]);

  useEffect(() => {
    if (!branchSessions.length) {
      if (activeSessionId) {
        setActiveSessionId(null);
      }
      return;
    }

    const currentSession = branchSessions.find((session) => session.sessionId === activeSessionId);
    if (currentSession && (currentSession.status === "running" || currentSession.status === "pending")) {
      return;
    }

    const nextSession =
      branchSessions.find((session) => session.status === "running") ??
      branchSessions.find((session) => session.status === "pending") ??
      null;

    if (nextSession) {
      setActiveSessionId(nextSession.sessionId);
      return;
    }

    if (activeSessionId) {
      setActiveSessionId(null);
    }
  }, [branchSessions, activeSessionId]);

  if (isLoading) {
      return (
        <div className="app-shell">
          <div className="page-state page-state--centered">
            <h1>Loading</h1>
            <p>Fetching branch details...</p>
          </div>
        </div>
      );
  }

  if (error) {
      return (
        <div className="app-shell">
          <div className="page-state page-state--centered">
            <h1>Failed to load branch</h1>
            <p>{error instanceof Error ? error.message : "Unknown error"}</p>
            <Link to="/" className="button button--ghost">
              Back to branches
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
            <p>The requested branch does not exist.</p>
            <Link to="/" className="button button--ghost">
              Back to branches
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
        message: `Worktree created for ${branch.name}.`,
      });
    } catch (err) {
      setBanner({
        type: "error",
        message: err instanceof Error ? err.message : "Failed to create worktree",
      });
    }
  };

  const handleTerminateSession = async (sessionId: string) => {
    setTerminatingSessionId(sessionId);
    try {
      await deleteSession.mutateAsync(sessionId);
      setBanner({ type: "success", message: "Session terminated" });
      if (activeSessionId === sessionId) {
        setActiveSessionId(null);
      }
      queryClient.invalidateQueries({ queryKey: ["sessions"] });
    } catch (err) {
      setBanner({ type: "error", message: formatError(err, "Failed to terminate session") });
    } finally {
      setTerminatingSessionId(null);
    }
  };

  

  const handleSessionExit = (code: number) => {
    setActiveSessionId(null);
    setIsTerminalFullscreen(false);
    queryClient.invalidateQueries({ queryKey: ["sessions"] });
    setBanner({
      type: code === 0 ? "success" : "error",
      message: `Session exited with code ${code}.`,
    });
    navigate("/", { replace: false });
  };

  const handleFocusSession = (sessionId: string) => {
    setActiveSessionId(sessionId);
    setIsTerminalFullscreen(false);
  };

  return (
    <div className="app-shell">
      <header className="page-hero page-hero--compact">
        <Link to="/" className="link-back">
          ← Back to branches
        </Link>
        <p className="page-hero__eyebrow">BRANCH DETAIL</p>
        <h1>{branch.name}</h1>
        <p className="page-hero__subtitle">
          Latest commit {branch.commitHash.slice(0, 7)} · {formattedCommitDate}
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
            {branch.worktreePath ? "Worktree ready" : "No worktree"}
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
              {createWorktree.isPending ? "Creating..." : "Create worktree"}
            </button>
          ) : (
            <Link to="/config" className="button button--secondary">
              Open custom tools
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
                  <h2>AI tool settings</h2>
                  <p className="section-card__body">
                    Launch AI tools from the branch list modal. This page shows current branch info and session status.
                  </p>
                </div>
                {configError && (
                  <span className="pill pill--warning">Failed to load config</span>
                )}
              </header>

              {!branch.worktreePath && (
                <div className="inline-banner inline-banner--warning">
                  Worktree missing. Create one on the branch list before launching AI tools.
                </div>
              )}

              <div className="tool-summary-grid">
                <div>
                  <p className="tool-card__eyebrow">Built-in tools</p>
                  <ul className="tool-summary-list">
                    {builtinTools.map((tool) => (
                      <li key={tool.id}>{tool.label}</li>
                    ))}
                  </ul>
                </div>
                <div>
                  <p className="tool-card__eyebrow">Custom tools</p>
                  {customTools.length === 0 ? (
                    <p className="tool-card__muted">No custom tools registered.</p>
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
                  Open branch list
                </Link>
                <Link to="/config" className="button button--ghost">
                  Custom tool settings
                </Link>
              </div>
            </section>
            <section className="section-card">
              <header className="terminal-section__header">
                <div>
                  <h2>Session history</h2>
                  <p className="section-card__body">
                    Shows the latest AI sessions tied to this worktree (including CLI launches).
                  </p>
                </div>
                {isSessionsLoading && <span className="pill">Loading...</span>}
              </header>
              {branchSessions.length === 0 ? (
                <p className="section-card__body">No session history yet.</p>
              ) : (
                <div className="session-table-wrapper">
                  <table className="session-table">
                    <thead>
                      <tr>
                        <th>Status</th>
                        <th>Tool</th>
                        <th>Mode</th>
                        <th>Started</th>
                        <th>Ended</th>
                        <th>Actions</th>
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
                                  View
                                </button>
                                <button
                                  type="button"
                                  className="button button--ghost"
                                  onClick={() => handleTerminateSession(session.sessionId)}
                                  disabled={terminatingSessionId === session.sessionId || deleteSession.isPending}
                                  data-testid="session-stop-button"
                                >
                                  {terminatingSessionId === session.sessionId ? "Stopping..." : "Stop"}
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
                  <h2>Branch insights</h2>
              </header>
              <dl className="metadata-grid">
                <div>
                    <dt>Commit</dt>
                  <dd>{branch.commitHash}</dd>
                </div>
                <div>
                  <dt>Author</dt>
                  <dd>{branch.author ?? "N/A"}</dd>
                </div>
                <div>
                    <dt>Updated</dt>
                  <dd>{formattedCommitDate}</dd>
                </div>
                <div>
                    <dt>Worktree</dt>
                  <dd>{branch.worktreePath ?? "Not created"}</dd>
                </div>
              </dl>
            </section>

            <section className="section-card">
                <header>
                  <h2>Commit message</h2>
              </header>
              <p className="section-card__body">
                  {branch.commitMessage ?? "No commit message."}
              </p>
            </section>

            {branch.divergence && (
              <section className="section-card">
                <header>
                  <h2>Divergence</h2>
                </header>
                <div className="pill-group">
                  <span className="pill">Ahead {branch.divergence.ahead}</span>
                  <span className="pill">Behind {branch.divergence.behind}</span>
                  <span
                    className={`pill ${
                      branch.divergence.upToDate ? "pill--success" : "pill--warning"
                    }`}
                  >
                    {branch.divergence.upToDate ? "Up-to-date" : "Needs sync"}
                  </span>
                </div>
              </section>
            )}

            <section className="section-card">
                <header>
                  <h2>Worktree info</h2>
              </header>
              <ul className="list-muted">
                <li>
                  Path: <strong>{branch.worktreePath ?? "Not created"}</strong>
                </li>
                <li>Ensure the worktree is clean before launching AI tools.</li>
                <li>Recreating worktrees can discard local changes.</li>
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
                    <h2>Terminal session</h2>
                    <p className="section-card__body">
                      Output streams in real time. This panel closes when the session exits.
                    </p>
                  </div>
                  <div className="terminal-section__controls">
                    <button
                      type="button"
                      className="button button--ghost"
                      onClick={() => setIsTerminalFullscreen((prev) => !prev)}
                    >
                      {isTerminalFullscreen ? "Exit fullscreen" : "Maximize terminal"}
                    </button>
                  </div>
                </div>
                <div className="terminal-surface">
                  <Terminal
                    sessionId={activeSessionId}
                    onExit={handleSessionExit}
                    onError={(message) =>
                      setBanner({ type: "error", message: message ?? "Unknown error" })
                    }
                  />
                </div>
                {isTerminalFullscreen && (
                  <button
                    type="button"
                    className="terminal-section__close"
                    aria-label="Close terminal"
                    onClick={() => setIsTerminalFullscreen(false)}
                  >
                    ×
                  </button>
                )}
              </section>
            ) : (
              <section className="section-card session-hint">
                <header>
                  <h2>No active session</h2>
                </header>
                <p className="section-card__body">
                  Launch an AI tool from the branch list and select the running session to see live output here.
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
    return "Unknown";
  }

  try {
    const date = new Date(value);
    return new Intl.DateTimeFormat("en-US", {
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
