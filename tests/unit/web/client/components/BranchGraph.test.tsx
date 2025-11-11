import React from "react";
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { Branch } from "../../../../../src/types/api.js";
import { BranchGraph } from "../../../../../src/web/client/src/components/BranchGraph";

const graphBranches: Branch[] = [
  {
    name: "feature/design-refresh",
    type: "local",
    mergeStatus: "unmerged",
    worktreePath: "/tmp/feature",
    commitHash: "abc123",
    commitMessage: "Update hero section",
    author: "Akira",
    commitDate: "2025-11-09T12:00:00.000Z",
    divergence: { ahead: 1, behind: 0, upToDate: false },
    baseBranch: "main",
  },
  {
    name: "hotfix/security",
    type: "local",
    mergeStatus: "merged",
    worktreePath: null,
    commitHash: "def789",
    commitMessage: "Urgent patch",
    author: "Noa",
    commitDate: "2025-11-07T08:00:00.000Z",
    divergence: { ahead: 0, behind: 2, upToDate: false },
    baseBranch: "origin/main",
  },
];

describe("BranchGraph", () => {
  const renderGraph = (branches: Branch[]) =>
    render(
      <MemoryRouter>
        <BranchGraph branches={branches} />
      </MemoryRouter>,
    );

  it("groups branches by base branch and renders nodes", () => {
    renderGraph(graphBranches);

    expect(
      screen.getByText("ベースブランチの関係をグラフィカルに把握"),
    ).toBeInTheDocument();

    const laneLabels = screen.getAllByText((content, element) => {
      return (
        content === "main" &&
        element?.classList.contains("branch-graph__lane-label")
      );
    });
    expect(laneLabels.length).toBeGreaterThan(0);

    expect(
      screen.getAllByText("feature/design-refresh").length,
    ).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText(/Worktree/).length).toBeGreaterThanOrEqual(1);
  });

  it("renders empty state when no branches are provided", () => {
    renderGraph([]);

    expect(
      screen.getByText("グラフ表示できるブランチがありません。"),
    ).toBeInTheDocument();
  });
});
