/**
 * ブランチ一覧ページ (/)
 *
 * すべてのブランチを一覧表示し、Worktree作成やセッション起動が可能。
 */

import React from "react";
import { Link } from "react-router-dom";
import { useBranches } from "../hooks/useBranches";

export function BranchListPage() {
  const { data: branches, isLoading, error } = useBranches();

  if (isLoading) {
    return (
      <div style={{ padding: "2rem" }}>
        <h1>Loading branches...</h1>
      </div>
    );
  }

  if (error) {
    return (
      <div style={{ padding: "2rem" }}>
        <h1>Error loading branches</h1>
        <p>{error instanceof Error ? error.message : String(error)}</p>
      </div>
    );
  }

  return (
    <div style={{ padding: "2rem" }}>
      <h1>Claude Worktree - Branches</h1>
      <p>Total branches: {branches?.length || 0}</p>

      <div style={{ marginTop: "2rem" }}>
        {branches?.map((branch) => (
          <div
            key={branch.name}
            style={{
              padding: "1rem",
              margin: "1rem 0",
              border: "1px solid #ccc",
              borderRadius: "4px",
            }}
          >
            <h2>
              <Link to={`/${encodeURIComponent(branch.name)}`}>
                {branch.name}
              </Link>
            </h2>
            <p>Type: {branch.type}</p>
            <p>Merge Status: {branch.mergeStatus}</p>
            {branch.worktreePath && (
              <p>Worktree: {branch.worktreePath}</p>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
