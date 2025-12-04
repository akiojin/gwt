/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import type { Mock } from "vitest";
import { render, act } from "@testing-library/react";
import React from "react";
import type { BranchItem, CleanupTarget } from "../../types.js";
import { Window } from "happy-dom";
let App: typeof import("../../components/App.js").App;

const navigateToMock = vi.fn();
const goBackMock = vi.fn();
const resetMock = vi.fn();

const branchCreatorProps: any[] = [];
const branchListProps: any[] = [];

const useGitDataMock = vi.fn();
const useScreenStateMock = vi.fn();
const getMergedPRWorktreesMock = vi.fn();
const generateWorktreePathMock = vi.fn();
const createWorktreeMock = vi.fn();
const removeWorktreeMock = vi.fn();
const getRepositoryRootMock = vi.fn();
const deleteBranchMock = vi.fn();

vi.mock("../../hooks/useGitData.js", () => ({
  useGitData: (...args: any[]) => useGitDataMock(...args),
}));

vi.mock("../../hooks/useScreenState.js", () => ({
  useScreenState: (...args: any[]) => useScreenStateMock(...args),
}));

vi.mock("../../../../worktree.js", async () => {
  const actual = await vi.importActual<
    typeof import("../../../../worktree.js")
  >("../../../../worktree.js");
  return {
    ...actual,
    getMergedPRWorktrees: getMergedPRWorktreesMock,
    generateWorktreePath: generateWorktreePathMock,
    createWorktree: createWorktreeMock,
    removeWorktree: removeWorktreeMock,
  };
});

vi.mock("../../../../git.js", async () => {
  const actual =
    await vi.importActual<typeof import("../../../../git.js")>(
      "../../../../git.js",
    );
  return {
    ...actual,
    getRepositoryRoot: getRepositoryRootMock,
    deleteBranch: deleteBranchMock,
  };
});

vi.mock("../../components/screens/BranchCreatorScreen.js", () => {
  return {
    BranchCreatorScreen: (props: any) => {
      branchCreatorProps.push(props);
      return React.createElement("div", null, "BranchCreatorScreenMock");
    },
  };
});

vi.mock("../../components/screens/BranchListScreen.js", () => {
  return {
    BranchListScreen: (props: any) => {
      branchListProps.push(props);
      return React.createElement("div", null, "BranchListScreenMock");
    },
  };
});

describe("App shortcuts integration", () => {
  beforeEach(async () => {
    if (typeof globalThis.document === "undefined") {
      const window = new Window();
      globalThis.window = window as any;
      globalThis.document = window.document as any;
    }
    branchCreatorProps.length = 0;
    branchListProps.length = 0;
    navigateToMock.mockClear();
    goBackMock.mockClear();
    resetMock.mockClear();
    useGitDataMock.mockReturnValue({
      branches: [],
      worktrees: [
        {
          branch: "feature/existing",
          path: "/worktrees/feature-existing",
          isAccessible: true,
        },
      ],
      loading: false,
      error: null,
      refresh: vi.fn(),
      lastUpdated: null,
    });
    useScreenStateMock.mockReturnValue({
      currentScreen: "branch-list",
      navigateTo: navigateToMock as Mock,
      goBack: goBackMock as Mock,
      reset: resetMock as Mock,
    });
    getMergedPRWorktreesMock.mockResolvedValue([
      {
        branch: "feature/add-new-feature",
        cleanupType: "worktree-and-branch",
        pullRequest: {
          number: 123,
          title: "Add new feature",
          branch: "feature/add-new-feature",
          mergedAt: "2025-01-20T10:00:00Z",
          author: "user1",
        },
        worktreePath: "/worktrees/feature-add-new-feature",
        hasUncommittedChanges: false,
        hasUnpushedCommits: false,
        hasRemoteBranch: true,
        isAccessible: true,
      },
      {
        branch: "hotfix/urgent-fix",
        cleanupType: "worktree-and-branch",
        pullRequest: {
          number: 456,
          title: "Urgent fix",
          branch: "hotfix/urgent-fix",
          mergedAt: "2025-01-21T09:00:00Z",
          author: "user2",
        },
        worktreePath: "/worktrees/hotfix-urgent-fix",
        hasUncommittedChanges: true,
        hasUnpushedCommits: false,
        hasRemoteBranch: true,
        isAccessible: true,
      },
    ] as CleanupTarget[]);
    generateWorktreePathMock.mockResolvedValue("/worktrees/new-branch");
    createWorktreeMock.mockResolvedValue(undefined);
    removeWorktreeMock.mockResolvedValue(undefined);
    getRepositoryRootMock.mockResolvedValue("/repo");
    deleteBranchMock.mockResolvedValue(undefined);
    App = (await import("../../components/App.js")).App;
  });

  afterEach(() => {
    useGitDataMock.mockReset();
    useScreenStateMock.mockReset();
    getMergedPRWorktreesMock.mockReset();
    generateWorktreePathMock.mockReset();
    createWorktreeMock.mockReset();
    removeWorktreeMock.mockReset();
    getRepositoryRootMock.mockReset();
    deleteBranchMock.mockReset();
    branchCreatorProps.length = 0;
    branchListProps.length = 0;
  });

  it("creates new worktree when branch creator submits", async () => {
    const onExit = vi.fn();

    // Update screen state mock to branch-creator for this test
    useScreenStateMock.mockReturnValue({
      currentScreen: "branch-creator",
      navigateTo: navigateToMock as Mock,
      goBack: goBackMock as Mock,
      reset: resetMock as Mock,
    });

    render(<App onExit={onExit} />);

    expect(branchCreatorProps).not.toHaveLength(0);
    const { onCreate } = branchCreatorProps[0];

    await act(async () => {
      await onCreate("feature/new-branch");
    });

    expect(createWorktreeMock).toHaveBeenCalledWith(
      expect.objectContaining({
        branchName: "feature/new-branch",
        isNewBranch: true,
      }),
    );
    expect(navigateToMock).toHaveBeenCalledWith("ai-tool-selector");
  });

  it("displays per-branch cleanup indicators and waits before clearing results", async () => {
    vi.useFakeTimers();

    try {
      const onExit = vi.fn();

      let resolveRemoveWorktree: (() => void) | undefined;
      let resolveDeleteBranch: (() => void) | undefined;

      removeWorktreeMock.mockImplementationOnce(
        () =>
          new Promise<void>((resolve) => {
            resolveRemoveWorktree = resolve;
          }),
      );

      deleteBranchMock.mockImplementationOnce(
        () =>
          new Promise<void>((resolve) => {
            resolveDeleteBranch = resolve;
          }),
      );

      useScreenStateMock.mockReturnValue({
        currentScreen: "branch-list",
        navigateTo: navigateToMock as Mock,
        goBack: goBackMock as Mock,
        reset: resetMock as Mock,
      });

      render(<App onExit={onExit} />);

      expect(branchListProps).not.toHaveLength(0);
      const initialProps = branchListProps.at(-1);
      expect(initialProps).toBeDefined();
      if (!initialProps) {
        throw new Error("BranchListScreen props missing");
      }

      act(() => {
        initialProps.onCleanupCommand?.();
      });

      await act(async () => {
        await Promise.resolve();
      });

      let latestProps = branchListProps.at(-1);
      expect(latestProps?.cleanupUI?.inputLocked).toBe(true);
      expect(latestProps?.cleanupUI?.footerMessage?.text).toBeTruthy();
      expect(latestProps?.cleanupUI?.indicators).toMatchObject({
        "feature/add-new-feature": expect.objectContaining({
          icon: expect.stringMatching(/⠋|⠙|⠹|⠸|⠼|⠴|⠦|⠧/),
        }),
        "hotfix/urgent-fix": expect.objectContaining({ icon: "⏳" }),
      });

      resolveRemoveWorktree?.();

      await act(async () => {
        await Promise.resolve();
      });

      resolveDeleteBranch?.();

      expect(removeWorktreeMock).toHaveBeenCalledWith(
        "/worktrees/feature-add-new-feature",
        true,
      );
      expect(deleteBranchMock).toHaveBeenCalledWith(
        "feature/add-new-feature",
        true,
      );

      // Flush state updates after processing first target
      await act(async () => {
        await Promise.resolve();
      });

      latestProps = branchListProps.at(-1);
      expect(latestProps?.cleanupUI?.indicators).toMatchObject({
        "feature/add-new-feature": { icon: "✅" },
        "hotfix/urgent-fix": { icon: "⏭️" },
      });
      expect(latestProps?.cleanupUI?.inputLocked).toBe(false);

      // Advance 3 seconds to allow UI to clear
      await act(async () => {
        vi.advanceTimersByTime(3000);
        await Promise.resolve();
      });

      latestProps = branchListProps.at(-1);
      expect(latestProps?.cleanupUI?.indicators).toEqual({});
      expect(latestProps?.cleanupUI?.inputLocked).toBe(false);
      expect(
        latestProps?.branches?.some(
          (branch: BranchItem) => branch.name === "feature/add-new-feature",
        ),
      ).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });
});
