import React from "react";
import type { Mock } from "vitest";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import type { Branch } from "../../../../../src/types/api.js";
import { BranchDetailPage } from "../../../../../src/web/client/src/pages/BranchDetailPage.js";
import { useBranch } from "../../../../../src/web/client/src/hooks/useBranches.js";
import { useCreateWorktree } from "../../../../../src/web/client/src/hooks/useWorktrees.js";
import {
  useStartSession,
  useSessions,
  useDeleteSession,
} from "../../../../../src/web/client/src/hooks/useSessions.js";
import { useConfig } from "../../../../../src/web/client/src/hooks/useConfig.js";

vi.mock("../../../../../src/web/client/src/hooks/useBranches.js", () => ({
  useBranch: vi.fn(),
}));

vi.mock("../../../../../src/web/client/src/hooks/useWorktrees.js", () => ({
  useCreateWorktree: vi.fn(),
}));

vi.mock("../../../../../src/web/client/src/hooks/useSessions.js", () => ({
  useStartSession: vi.fn(),
  useSessions: vi.fn(),
  useDeleteSession: vi.fn(),
}));

vi.mock("../../../../../src/web/client/src/hooks/useConfig.js", () => ({
  useConfig: vi.fn(),
}));

const mockedUseBranch = useBranch as unknown as Mock;
const mockedUseCreateWorktree = useCreateWorktree as unknown as Mock;
const mockedUseStartSession = useStartSession as unknown as Mock;
const mockedUseSessions = useSessions as unknown as Mock;
const mockedUseDeleteSession = useDeleteSession as unknown as Mock;
const mockedUseConfig = useConfig as unknown as Mock;

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
  hasUnpushedCommits: true,
  prInfo: null,
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

    mockedUseSessions.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    mockedUseDeleteSession.mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    });

    mockedUseConfig.mockReturnValue({
      data: { tools: [] },
      isLoading: false,
      error: null,
    });
  });

  it("renders branch metadata and session actions when a worktree exists", () => {
    renderPage();

    expect(screen.getByText("feature/design-refresh")).toBeInTheDocument();
    expect(screen.getByText("コミット情報")).toBeInTheDocument();
    expect(screen.getByText("差分状況")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "セッションを起動" })).toBeEnabled();
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
    expect(screen.queryByRole("button", { name: "セッションを起動" })).not.toBeInTheDocument();
  });

  it("renders session history rows when data exists", () => {
    mockedUseSessions.mockReturnValueOnce({
      data: [
        {
          sessionId: "abc",
          toolType: "claude-code",
          mode: "normal",
          status: "running",
          worktreePath: baseBranch.worktreePath,
          startedAt: "2025-11-10T00:00:00Z",
          endedAt: null,
          toolName: null,
        },
      ],
      isLoading: false,
      error: null,
    });

    renderPage();
    expect(screen.getByText("セッション履歴")).toBeInTheDocument();
    expect(screen.getByText("running")).toBeInTheDocument();
  });

  it("blocks session start when branch has conflicting divergence", () => {
    mockedUseBranch.mockReturnValueOnce({
      data: {
        ...baseBranch,
        divergence: { ahead: 2, behind: 4, upToDate: false },
      },
      isLoading: false,
      error: null,
    });

    renderPage();

    expect(screen.getByTestId("divergence-warning")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "セッションを起動" })).toBeDisabled();
  });
});
