import React from "react";
import type { Mock } from "vitest";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import type { Branch } from "../../../../../src/types/api.js";
import { BranchDetailPage } from "../../../../../src/web/client/src/pages/BranchDetailPage.js";
import { useBranch } from "../../../../../src/web/client/src/hooks/useBranches.js";
import { useCreateWorktree } from "../../../../../src/web/client/src/hooks/useWorktrees.js";
import { useSessions, useDeleteSession } from "../../../../../src/web/client/src/hooks/useSessions.js";
import { useConfig } from "../../../../../src/web/client/src/hooks/useConfig.js";

class WebSocketMock {
  public onopen: ((event: Event) => void) | null = null;
  public onmessage: ((event: MessageEvent) => void) | null = null;
  public onerror: ((event: Event) => void) | null = null;
  public onclose: ((event: CloseEvent) => void) | null = null;
  public readyState = 1;

  constructor() {
    setTimeout(() => {
      this.onopen?.(new Event("open"));
    }, 0);
  }

  send() {}

  close() {
    this.readyState = 3;
    this.onclose?.(new Event("close"));
  }
}

vi.stubGlobal("WebSocket", WebSocketMock as unknown as typeof WebSocket);

vi.mock("../../../../../src/web/client/src/hooks/useBranches.js", () => ({
  useBranch: vi.fn(),
}));

vi.mock("../../../../../src/web/client/src/hooks/useWorktrees.js", () => ({
  useCreateWorktree: vi.fn(),
}));

vi.mock("../../../../../src/web/client/src/hooks/useSessions.js", () => ({
  useSessions: vi.fn(),
  useDeleteSession: vi.fn(),
}));

vi.mock("../../../../../src/web/client/src/hooks/useConfig.js", () => ({
  useConfig: vi.fn(),
}));

const mockedUseBranch = useBranch as unknown as Mock;
const mockedUseCreateWorktree = useCreateWorktree as unknown as Mock;
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

  it("renders branch metadata and AI tool summary when a worktree exists", () => {
    renderPage();

    expect(screen.getByText("feature/design-refresh")).toBeInTheDocument();
    expect(screen.getByText("AI tool settings")).toBeInTheDocument();
    expect(screen.getByText("Branch insights")).toBeInTheDocument();
    expect(screen.getByText("Session history")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open branch list" })).toBeInTheDocument();
  });

  it("shows worktree creation CTA when no worktree is present", () => {
    mockedUseBranch.mockReturnValueOnce({
      data: { ...baseBranch, worktreePath: null },
      isLoading: false,
      error: null,
    });

    renderPage();

    expect(
      screen.getByRole("button", { name: "Create worktree" }),
    ).toBeInTheDocument();
    expect(screen.getByText("AI tool settings")).toBeInTheDocument();
  });

  it("renders session history rows when data exists", () => {
    mockedUseSessions.mockReturnValue({
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
    expect(screen.getByText("Session history")).toBeInTheDocument();
    expect(screen.getAllByTestId("session-row").length).toBe(1);
  });

  it("shows session action buttons for running session", () => {
    mockedUseSessions.mockReturnValue({
      data: [
        {
          sessionId: "running-session",
          toolType: "codex-cli",
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

    expect(screen.getAllByTestId("session-focus-button").length).toBe(1);
    expect(screen.getAllByTestId("session-stop-button").length).toBe(1);
  });
});
