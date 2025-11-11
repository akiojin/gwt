import React from "react";
import type { Mock } from "vitest";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { Branch } from "../../../../../src/types/api.js";
import { BranchListPage } from "../../../../../src/web/client/src/pages/BranchListPage.js";
import { useBranches } from "../../../../../src/web/client/src/hooks/useBranches.js";

vi.mock("../../../../../src/web/client/src/hooks/useBranches.js", () => ({
  useBranches: vi.fn(),
}));

const mockedUseBranches = useBranches as unknown as Mock;

const sampleBranches: Branch[] = [
  {
    name: "feature/design-refresh",
    type: "local",
    mergeStatus: "unmerged",
    worktreePath: "/tmp/feature-design",
    commitHash: "abc123",
    commitMessage: "Refine UI layout",
    author: "Akira",
    commitDate: "2025-11-10T09:00:00.000Z",
    divergence: { ahead: 2, behind: 0, upToDate: false },
  },
  {
    name: "release/v1.0.0",
    type: "remote",
    mergeStatus: "merged",
    worktreePath: null,
    commitHash: "def789",
    commitMessage: "Tagged release",
    author: "Sana",
    commitDate: "2025-11-05T04:00:00.000Z",
    divergence: { ahead: 0, behind: 0, upToDate: true },
  },
  {
    name: "hotfix/security",
    type: "local",
    mergeStatus: "unknown",
    worktreePath: null,
    commitHash: "ghi456",
    commitMessage: "Patch CVE",
    author: "Noa",
    commitDate: "2025-11-03T01:00:00.000Z",
    divergence: { ahead: 0, behind: 3, upToDate: false },
  },
];

describe("BranchListPage", () => {
  beforeEach(() => {
    mockedUseBranches.mockReturnValue({
      data: sampleBranches,
      isLoading: false,
      error: null,
    });
  });

  const renderPage = () =>
    render(
      <MemoryRouter>
        <BranchListPage />
      </MemoryRouter>,
    );

  it("renders summary metrics and branch cards", () => {
    renderPage();

    expect(screen.getByText("総ブランチ数")).toBeInTheDocument();
    expect(screen.getByTestId("metric-total")).toHaveTextContent("3");
    expect(screen.getByTestId("metric-worktrees")).toHaveTextContent("1");
    expect(screen.getByText("feature/design-refresh")).toBeInTheDocument();
    expect(screen.getByText("未マージ")).toBeInTheDocument();
    expect(screen.getByText("リモート追跡ブランチ")).toBeInTheDocument();
  });

  it("filters branches by the search query and shows empty state when unmatched", () => {
    renderPage();

    const input = screen.getByPlaceholderText("ブランチ名やタイプで検索...");
    fireEvent.change(input, { target: { value: "release" } });

    expect(screen.getByText("release/v1.0.0")).toBeInTheDocument();
    expect(screen.queryByText("feature/design-refresh")).not.toBeInTheDocument();

    fireEvent.change(input, { target: { value: "zzz" } });
    expect(screen.getByText("一致するブランチがありません")).toBeInTheDocument();
  });
});
