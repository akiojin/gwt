/**
 * ブランチ詳細ページ (/:branchName)
 *
 * 特定のブランチの詳細情報、Worktree管理、AI Toolセッション起動。
 */

import React, { useState } from "react";
import { useParams, Link } from "react-router-dom";
import { useBranch } from "../hooks/useBranches";
import { useCreateWorktree } from "../hooks/useWorktrees";
import { useStartSession } from "../hooks/useSessions";
import { Terminal } from "../components/Terminal";

export function BranchDetailPage() {
  const { branchName } = useParams<{ branchName: string }>();
  const decodedBranchName = branchName ? decodeURIComponent(branchName) : "";

  const { data: branch, isLoading, error } = useBranch(decodedBranchName);
  const createWorktree = useCreateWorktree();
  const startSession = useStartSession();

  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [isStartingSession, setIsStartingSession] = useState(false);

  if (isLoading) {
    return (
      <div style={{ padding: "2rem" }}>
        <h1>Loading branch details...</h1>
      </div>
    );
  }

  if (error) {
    return (
      <div style={{ padding: "2rem" }}>
        <h1>Error loading branch</h1>
        <p>{error instanceof Error ? error.message : String(error)}</p>
        <Link to="/">Back to branches</Link>
      </div>
    );
  }

  if (!branch) {
    return (
      <div style={{ padding: "2rem" }}>
        <h1>Branch not found</h1>
        <Link to="/">Back to branches</Link>
      </div>
    );
  }

  const handleCreateWorktree = async () => {
    try {
      await createWorktree.mutateAsync({
        branchName: branch.name,
        createBranch: false,
      });
      alert(`Worktree created for ${branch.name}`);
    } catch (err) {
      alert(`Failed to create worktree: ${err}`);
    }
  };

  const handleStartSession = async (
    toolType: "claude-code" | "codex-cli",
  ) => {
    if (!branch.worktreePath) {
      alert("No worktree found. Please create a worktree first.");
      return;
    }

    setIsStartingSession(true);
    try {
      const session = await startSession.mutateAsync({
        toolType,
        toolName: null,
        mode: "normal",
        worktreePath: branch.worktreePath,
      });

      setActiveSessionId(session.sessionId);
    } catch (err) {
      alert(`Failed to start session: ${err}`);
    } finally {
      setIsStartingSession(false);
    }
  };

  const handleSessionExit = (code: number) => {
    console.log(`Session exited with code ${code}`);
    setActiveSessionId(null);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      {/* Header */}
      <div style={{ padding: "1rem", borderBottom: "1px solid #ccc" }}>
        <Link to="/">← Back to branches</Link>
        <h1>Branch: {branch.name}</h1>

        <div style={{ marginTop: "1rem" }}>
          <strong>Type:</strong> {branch.type} |{" "}
          <strong>Merge Status:</strong> {branch.mergeStatus}
          {branch.worktreePath && (
            <>
              {" "}
              | <strong>Worktree:</strong> {branch.worktreePath}
            </>
          )}
        </div>

        {/* Actions */}
        <div style={{ marginTop: "1rem" }}>
          {!branch.worktreePath && (
            <button
              onClick={handleCreateWorktree}
              disabled={createWorktree.isPending}
            >
              {createWorktree.isPending
                ? "Creating..."
                : "Create Worktree"}
            </button>
          )}

          {branch.worktreePath && !activeSessionId && (
            <>
              <button
                onClick={() => handleStartSession("claude-code")}
                disabled={isStartingSession}
                style={{ marginLeft: "0.5rem" }}
              >
                {isStartingSession
                  ? "Starting..."
                  : "Start Claude Code"}
              </button>
              <button
                onClick={() => handleStartSession("codex-cli")}
                disabled={isStartingSession}
                style={{ marginLeft: "0.5rem" }}
              >
                {isStartingSession ? "Starting..." : "Start Codex CLI"}
              </button>
            </>
          )}
        </div>
      </div>

      {/* Terminal */}
      {activeSessionId && (
        <div style={{ flex: 1, overflow: "hidden" }}>
          <Terminal
            sessionId={activeSessionId}
            onExit={handleSessionExit}
            onError={(msg) => console.error("Terminal error:", msg)}
          />
        </div>
      )}

      {/* Details (when no active session) */}
      {!activeSessionId && (
        <div style={{ flex: 1, padding: "2rem", overflow: "auto" }}>
          <h2>Details</h2>
          <p>
            <strong>Commit:</strong> {branch.commitHash}
          </p>
          <p>
            <strong>Message:</strong> {branch.commitMessage || "N/A"}
          </p>
          <p>
            <strong>Author:</strong> {branch.author || "N/A"}
          </p>
          <p>
            <strong>Date:</strong> {branch.commitDate || "N/A"}
          </p>

          {branch.divergence && (
            <div style={{ marginTop: "2rem" }}>
              <h2>Divergence</h2>
              <p>
                <strong>Ahead:</strong> {branch.divergence.ahead}
              </p>
              <p>
                <strong>Behind:</strong> {branch.divergence.behind}
              </p>
              <p>
                <strong>Up to date:</strong>{" "}
                {branch.divergence.upToDate ? "Yes" : "No"}
              </p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
