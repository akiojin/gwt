/**
 * @vitest-environment happy-dom
 */
import {
  describe,
  it,
  expect,
  beforeEach,
  afterEach,
  afterAll,
  vi,
} from "vitest";
import { act, render } from "@testing-library/react";
import React from "react";
import { Window } from "happy-dom";
import type { BranchInfo, BranchItem } from "../../types.js";
import type { ScreenType } from "../../types.js";

const navigateToMock = vi.fn();
const goBackMock = vi.fn();
const resetMock = vi.fn();

const branchListProps: any[] = [];
const branchActionProps: any[] = [];
const aiToolProps: any[] = [];
let currentScreenState: ScreenType;
let App: typeof import("../../components/App.js").App;
const useGitDataMock = vi.fn();
const useScreenStateMock = vi.fn();
const switchToProtectedBranchMock = vi.fn();
const getRepositoryRootMock = vi.fn();

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
    switchToProtectedBranch: switchToProtectedBranchMock,
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

vi.mock("../../screens/BranchActionSelectorScreen.js", () => {
  return {
    BranchActionSelectorScreen: (props: any) => {
      branchActionProps.push(props);
      return React.createElement("div", null, "BranchActionSelectorMock");
    },
  };
});

vi.mock("../../components/screens/AIToolSelectorScreen.js", () => {
  return {
    AIToolSelectorScreen: (props: unknown) => {
      aiToolProps.push(props);
      return React.createElement("div");
    },
  };
});

describe("App protected branch handling", () => {
  beforeEach(async () => {
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    currentScreenState = "branch-list";
    navigateToMock.mockReset();
    goBackMock.mockReset();
    resetMock.mockReset();
    branchListProps.length = 0;
    branchActionProps.length = 0;
    aiToolProps.length = 0;

    useGitDataMock.mockReset();
    switchToProtectedBranchMock.mockReset();
    getRepositoryRootMock.mockReset();
    App = (await import("../../components/App.js")).App;

    useScreenStateMock.mockImplementation(() => ({
      currentScreen: currentScreenState,
      navigateTo: (screen: ScreenType) => {
        navigateToMock(screen);
        currentScreenState = screen;
      },
      goBack: goBackMock,
      reset: () => {
        resetMock();
        currentScreenState = "branch-list";
      },
    }));

    switchToProtectedBranchMock.mockResolvedValue("local");
    getRepositoryRootMock.mockResolvedValue("/repo");
  });

  afterEach(() => {
    useGitDataMock.mockReset();
    useScreenStateMock.mockReset();
    switchToProtectedBranchMock.mockReset();
    getRepositoryRootMock.mockReset();
    branchActionProps.length = 0;
  });

  it("shows protected branch warning and switches root without launching AI tool", async () => {
    const branches: BranchInfo[] = [
      {
        name: "main",
        type: "local",
        branchType: "main",
        isCurrent: false,
      },
      {
        name: "feature/example",
        type: "local",
        branchType: "feature",
        isCurrent: true,
      },
    ];

    useGitDataMock.mockReturnValue({
      branches,
      worktrees: [],
      loading: false,
      error: null,
      refresh: vi.fn(),
      lastUpdated: null,
    });

    render(<App onExit={vi.fn()} />);

    expect(branchListProps).not.toHaveLength(0);
    const latestProps = branchListProps.at(-1);
    expect(latestProps).toBeDefined();
    if (!latestProps) {
      throw new Error("BranchListScreen props missing");
    }

    const protectedBranch = (latestProps.branches as BranchItem[]).find(
      (item) => item.name === "main",
    );
    expect(protectedBranch).toBeDefined();
    if (!protectedBranch) {
      throw new Error("Protected branch item not found");
    }

    await act(async () => {
      latestProps.onSelect(protectedBranch);
      await Promise.resolve();
    });

    expect(navigateToMock).toHaveBeenCalledWith("branch-action-selector");
    expect(branchActionProps).not.toHaveLength(0);
    const actionProps = branchActionProps.at(-1);
    expect(actionProps?.mode).toBe("protected");
    expect(actionProps?.infoMessage).toContain("is a root branch");
    expect(actionProps?.primaryLabel).toBe("Use root branch (no worktree)");
    expect(actionProps?.secondaryLabel).toBe(
      "Create new branch from this branch",
    );

    await act(async () => {
      actionProps?.onUseExisting();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(switchToProtectedBranchMock).toHaveBeenCalledWith({
      branchName: "main",
      repoRoot: expect.any(String),
      remoteRef: null,
    });

    expect(navigateToMock).toHaveBeenCalledWith("ai-tool-selector");
    expect(aiToolProps).not.toHaveLength(0);
  });
});
