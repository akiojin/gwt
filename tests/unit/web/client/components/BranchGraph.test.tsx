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
      screen.getByRole("group", { name: "Base branch filters" }),
    ).toBeInTheDocument();

    expect(
      screen.getByRole("button", { name: "Focus on main" }),
    ).toBeInTheDocument();

    expect(
      screen.getByRole("button", { name: "Select feature/design-refresh" }),
    ).toBeInTheDocument();
    expect(screen.getByText("divergence: +1 / -0")).toBeInTheDocument();
  });

  it("renders empty state when no branches are provided", () => {
    renderGraph([]);

    expect(screen.getByText("No branches to visualize yet.")).toBeInTheDocument();
  });
});
