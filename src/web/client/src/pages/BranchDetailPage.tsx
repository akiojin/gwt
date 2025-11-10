/**
 * ブランチ詳細ページ (/:branchName)
 *
 * 特定のブランチの詳細情報、Worktree管理、AI Toolセッション起動。
 */

import React from "react";
import { useParams, Link } from "react-router-dom";
import { useBranch } from "../hooks/useBranches";

export function BranchDetailPage() {
  const { branchName } = useParams<{ branchName: string }>();
  const decodedBranchName = branchName ? decodeURIComponent(branchName) : "";

  const { data: branch, isLoading, error } = useBranch(decodedBranchName);

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

  return (
    <div style={{ padding: "2rem" }}>
      <Link to="/">← Back to branches</Link>
      <h1>Branch: {branch.name}</h1>

      <div style={{ marginTop: "2rem" }}>
        <h2>Details</h2>
        <p>Type: {branch.type}</p>
        <p>Commit: {branch.commitHash}</p>
        <p>Message: {branch.commitMessage || "N/A"}</p>
        <p>Author: {branch.author || "N/A"}</p>
        <p>Date: {branch.commitDate || "N/A"}</p>
        <p>Merge Status: {branch.mergeStatus}</p>
        {branch.worktreePath && (
          <p>Worktree Path: {branch.worktreePath}</p>
        )}
      </div>

      {branch.divergence && (
        <div style={{ marginTop: "2rem" }}>
          <h2>Divergence</h2>
          <p>Ahead: {branch.divergence.ahead}</p>
          <p>Behind: {branch.divergence.behind}</p>
          <p>Up to date: {branch.divergence.upToDate ? "Yes" : "No"}</p>
        </div>
      )}

      <div style={{ marginTop: "2rem" }}>
        <h2>Actions</h2>
        <button>Create Worktree</button>
        <button style={{ marginLeft: "1rem" }}>Start AI Tool Session</button>
      </div>
    </div>
  );
}
