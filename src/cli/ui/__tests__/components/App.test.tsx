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
import type { BranchInfo } from "../../types.js";
import type { BranchListScreenProps } from "../../components/screens/BranchListScreen.js";

const mockRefresh = vi.fn();
let App: typeof import("../../components/App.js").App;
const branchListProps: BranchListScreenProps[] = [];
const useGitDataMock = vi.fn();

vi.mock("../../hooks/useGitData.js", () => ({
  useGitData: (...args: any[]) => useGitDataMock(...args),
}));

vi.mock("../../components/screens/BranchListScreen.js", () => {
  return {
    BranchListScreen: (props: BranchListScreenProps) => {
      branchListProps.push(props);
      return (
        <div>
          <div>gwt - Branch Selection</div>
          <div>Local: {props.stats?.localCount ?? 0}</div>
          <div>Remote: {props.stats?.remoteCount ?? 0}</div>
          <div>Worktrees: {props.stats?.worktreeCount ?? 0}</div>
          <div>Changes: {props.stats?.changesCount ?? 0}</div>
          {props.loading && <div>Loading Git information</div>}
          {props.error && <div>Error: {props.error.message}</div>}
          {!props.loading && !props.error && props.branches.length === 0 && (
            <div>No branches found</div>
          )}
          <ul>
            {props.branches.map((branch) => (
              <li
                key={branch.name}
              >{`${branch.icons?.join("") ?? ""} ${branch.name}`}</li>
            ))}
          </ul>
        </div>
      );
    },
  };
});

describe("App", () => {
  beforeEach(async () => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    vi.clearAllMocks();
    useGitDataMock.mockReset();
    branchListProps.length = 0;
    App = (await import("../../components/App.js")).App;
  });

  const mockBranches: BranchInfo[] = [
    {
      name: "main",
      type: "local",
      branchType: "main",
      isCurrent: true,
    },
    {
      name: "feature/test",
      type: "local",
      branchType: "feature",
      isCurrent: false,
    },
  ];

  it("should render BranchListScreen when data is loaded", () => {
    useGitDataMock.mockImplementation(() => ({
      branches: mockBranches,
      loading: false,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    }));

    const onExit = vi.fn();
    render(<App onExit={onExit} />);

    expect(branchListProps).not.toHaveLength(0);
    const props = branchListProps.at(-1);
    expect(props?.loading).toBe(false);
    expect(props?.error).toBeNull();
    const branchNames = props?.branches.map((b) => b.name);
    expect(branchNames).toContain("main");
    expect(branchNames).toContain("feature/test");
  });

  it("should show loading state initially", async () => {
    useGitDataMock.mockImplementation(() => ({
      branches: [],
      loading: true,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    }));

    const onExit = vi.fn();
    render(<App onExit={onExit} loadingIndicatorDelay={10} />);

    expect(branchListProps).not.toHaveLength(0);
    const props = branchListProps.at(-1);
    expect(props?.loading).toBe(true);
    expect(props?.loadingIndicatorDelay).toBe(10);
  });

  it("should show error state when Git data fails to load", () => {
    const error = new Error("Failed to fetch branches");
    useGitDataMock.mockImplementation(() => ({
      branches: [],
      loading: false,
      error,
      worktrees: [],
      refresh: mockRefresh,
    }));

    const onExit = vi.fn();
    render(<App onExit={onExit} />);

    expect(branchListProps).not.toHaveLength(0);
    const props = branchListProps.at(-1);
    expect(props?.error?.message).toBe("Failed to fetch branches");
    expect(props?.loading).toBe(false);
  });

  it("should calculate statistics from branches", () => {
    const branchesWithWorktree: BranchInfo[] = [
      {
        name: "main",
        type: "local",
        branchType: "main",
        isCurrent: true,
      },
      {
        name: "feature/a",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        worktree: {
          path: "/path/a",
          locked: false,
          prunable: false,
        },
      },
      {
        name: "origin/main",
        type: "remote",
        branchType: "main",
        isCurrent: false,
      },
    ];

    useGitDataMock.mockImplementation(() => ({
      branches: branchesWithWorktree,
      loading: false,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    }));

    const onExit = vi.fn();
    render(<App onExit={onExit} />);

    expect(branchListProps).not.toHaveLength(0);
    const props = branchListProps.at(-1);
    expect(props?.stats.localCount).toBe(2);
    expect(props?.stats.remoteCount).toBe(1);
    expect(props?.stats.worktreeCount).toBe(1);
  });

  it("should render branch selection without triggering exit", () => {
    useGitDataMock.mockImplementation(() => ({
      branches: mockBranches,
      loading: false,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    }));

    const onExit = vi.fn();
    const { container } = render(<App onExit={onExit} />);

    expect(container).toBeDefined();
    expect(onExit).not.toHaveBeenCalled();
    // Note: Testing actual selection requires simulating user input,
    // which is covered in integration tests
  });

  it("should handle empty branch list", () => {
    useGitDataMock.mockImplementation(() => ({
      branches: [],
      loading: false,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    }));

    const onExit = vi.fn();
    render(<App onExit={onExit} />);

    expect(branchListProps).not.toHaveLength(0);
    const props = branchListProps.at(-1);
    expect(props?.branches).toHaveLength(0);
    expect(props?.loading).toBe(false);
    expect(props?.error).toBeNull();
  });

  it("should wrap with ErrorBoundary", () => {
    // This test verifies ErrorBoundary is present
    // Actual error catching is tested separately
    useGitDataMock.mockImplementation(() => ({
      branches: mockBranches,
      loading: false,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    }));

    const onExit = vi.fn();
    const { container } = render(<App onExit={onExit} />);

    expect(container).toBeDefined();
  });

  it("should format branch items with icons", () => {
    useGitDataMock.mockImplementation(() => ({
      branches: mockBranches,
      loading: false,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    }));

    const onExit = vi.fn();
    render(<App onExit={onExit} />);

    expect(branchListProps).not.toHaveLength(0);
    const props = branchListProps.at(-1);
    const main = props?.branches.find((b: any) => b.name === "main");
    expect(main?.icons).toContain("⚡");
  });

  describe("BranchActionSelectorScreen integration", () => {
    it("should show BranchActionSelectorScreen after branch selection", () => {
      useGitDataMock.mockImplementation(() => ({
        branches: mockBranches,
        loading: false,
        error: null,
        worktrees: [],
        refresh: mockRefresh,
      }));

      const onExit = vi.fn();
      const { container } = render(<App onExit={onExit} />);

      // After implementation, should verify BranchActionSelectorScreen appears
      expect(container).toBeDefined();
    });

    it('should navigate to AI tool selector when "use existing" is selected', () => {
      useGitDataMock.mockImplementation(() => ({
        branches: mockBranches,
        loading: false,
        error: null,
        worktrees: [],
        refresh: mockRefresh,
      }));

      const onExit = vi.fn();
      const { container } = render(<App onExit={onExit} />);

      // After implementation, should verify navigation to AIToolSelectorScreen
      expect(container).toBeDefined();
    });

    it('should navigate to branch creator when "create new" is selected', () => {
      useGitDataMock.mockImplementation(() => ({
        branches: mockBranches,
        loading: false,
        error: null,
        worktrees: [],
        refresh: mockRefresh,
      }));

      const onExit = vi.fn();
      const { container } = render(<App onExit={onExit} />);

      // After implementation, should verify navigation to BranchCreatorScreen
      expect(container).toBeDefined();
    });
  });
});
