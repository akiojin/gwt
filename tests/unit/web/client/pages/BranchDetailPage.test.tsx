import React from "react";
import type { Mock } from "vitest";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import type { Branch } from "../../../../../src/types/api.js";
import { BranchDetailPage } from "../../../../../src/web/client/src/pages/BranchDetailPage.js";
import { useBranch } from "../../../../../src/web/client/src/hooks/useBranches.js";
import { useCreateWorktree } from "../../../../../src/web/client/src/hooks/useWorktrees.js";
import { useStartSession } from "../../../../../src/web/client/src/hooks/useSessions.js";

vi.mock("../../../../../src/web/client/src/hooks/useBranches.js", () => ({
  useBranch: vi.fn(),
}));

vi.mock("../../../../../src/web/client/src/hooks/useWorktrees.js", () => ({
  useCreateWorktree: vi.fn(),
}));

vi.mock("../../../../../src/web/client/src/hooks/useSessions.js", () => ({
  useStartSession: vi.fn(),
}));

const mockedUseBranch = useBranch as unknown as Mock;
const mockedUseCreateWorktree = useCreateWorktree as unknown as Mock;
const mockedUseStartSession = useStartSession as unknown as Mock;

const baseBranch: Branch = {
  name: "feature/design-refresh",
  type: "local",
  mergeStatus: "unmerged",
  commitHash: "abc123",
  commitMessage: "Refine UI layout",
  author: "Akira",
  commitDate: "2025-11-10T09:00:00.000Z",
  worktreePath: "/tmp/feature-design",
  divergence: { ahead: 2, behind: 0, upToDate: false },
};

const renderPage = () =>
  render(
    <MemoryRouter initialEntries={["/feature%2Fdesign-refresh"]}>
      <Routes>
        <Route path="/:branchName" element={<BranchDetailPage />} />
      </Routes>
    </MemoryRouter>,
  );

describe("BranchDetailPage", () => {
  beforeEach(() => {
    mockedUseBranch.mockReturnValue({
      data: baseBranch,
      isLoading: false,
      error: null,
    });

    mockedUseCreateWorktree.mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    });

    mockedUseStartSession.mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    });
  });

  it("renders branch metadata and session actions when a worktree exists", () => {
    renderPage();

    expect(screen.getByText("feature/design-refresh")).toBeInTheDocument();
    expect(screen.getByText("コミット情報")).toBeInTheDocument();
    expect(screen.getByText("差分状況")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Claude Codeを起動" })).toBeEnabled();
    expect(screen.getByText("Worktree情報")).toBeInTheDocument();
  });

  it("shows worktree creation CTA when no worktree is present", () => {
    mockedUseBranch.mockReturnValueOnce({
      data: { ...baseBranch, worktreePath: null },
      isLoading: false,
      error: null,
    });

    renderPage();

    expect(
      screen.getByRole("button", { name: "Worktreeを作成" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Claude Codeを起動" }),
    ).not.toBeInTheDocument();
  });
});
