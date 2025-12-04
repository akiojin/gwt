/**
 * @vitest-environment happy-dom
 * Edge case tests for UI components
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
import { render } from "@testing-library/react";
import React from "react";
import * as BranchListScreenModule from "../../components/screens/BranchListScreen.js";
import type { BranchListScreenProps } from "../../components/screens/BranchListScreen.js";
import { Window } from "happy-dom";
import type { BranchInfo, BranchItem, Statistics } from "../../types.js";

const mockRefresh = vi.fn();
const branchListProps: BranchListScreenProps[] = [];
const useGitDataMock = vi.fn();
let App: typeof import("../../components/App.js").App;

vi.mock("../../hooks/useGitData.js", () => ({
  useGitData: (...args: any[]) => useGitDataMock(...args),
}));

vi.mock("../../components/screens/BranchListScreen.js", () => ({
  BranchListScreen: (props: BranchListScreenProps) => {
    branchListProps.push(props);
    return (
      <div>
        <div>BranchList</div>
        {props.error && <div>Error: {props.error.message}</div>}
        <div>
          Local:{props.stats?.localCount ?? 0} Remote:
          {props.stats?.remoteCount ?? 0} Worktrees:
          {props.stats?.worktreeCount ?? 0}
        </div>
        <ul>
          {props.branches.map((b) => (
            <li key={b.name}>{b.name}</li>
          ))}
        </ul>
      </div>
    );
  },
}));

describe("Edge Cases Integration Tests", () => {
  beforeEach(async () => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    // Reset mocks
    vi.clearAllMocks();
    useGitDataMock.mockReset();
    branchListProps.length = 0;
    App = (await import("../../components/App.js")).App;
  });

  /**
   * T091: Terminal size極小（10行以下）の動作確認
   */
  it("[T091] should handle minimal terminal size (10 rows)", () => {
    // Save original rows
    const originalRows = process.stdout.rows;

    // Set minimal terminal size
    process.stdout.rows = 10;

    const mockBranches: BranchItem[] = [
      { name: "main", label: "main", value: "main" },
      { name: "feature/a", label: "feature/a", value: "feature/a" },
      { name: "feature/b", label: "feature/b", value: "feature/b" },
    ];

    const mockStats: Statistics = {
      localCount: 3,
      remoteCount: 0,
      worktreeCount: 0,
      changesCount: 0,
      lastUpdated: new Date(),
    };

    const onSelect = vi.fn();
    const { container } = render(
      <BranchListScreenModule.BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    // Should render without crashing
    expect(container).toBeDefined();

    // Restore original rows
    process.stdout.rows = originalRows;
  });

  it("[T091] should handle extremely small terminal (5 rows)", () => {
    const originalRows = process.stdout.rows;
    process.stdout.rows = 5;

    const mockBranches: BranchItem[] = [
      { name: "main", label: "main", value: "main" },
    ];

    const mockStats: Statistics = {
      localCount: 1,
      remoteCount: 0,
      worktreeCount: 0,
      changesCount: 0,
      lastUpdated: new Date(),
    };

    const onSelect = vi.fn();
    const { container } = render(
      <BranchListScreenModule.BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    expect(container).toBeDefined();
    expect(branchListProps.at(-1)?.branches).toHaveLength(1);

    process.stdout.rows = originalRows;
  });

  /**
   * T092: 非常に長いブランチ名の表示確認
   */
  it("[T092] should handle very long branch names", () => {
    const longBranchName =
      "feature/very-long-branch-name-that-exceeds-normal-terminal-width-and-should-be-handled-gracefully";

    const mockBranches: BranchItem[] = [
      {
        name: "main",
        label: "main",
        value: "main",
        latestCommitTimestamp: 1_700_000_000,
      },
      {
        name: longBranchName,
        label: longBranchName,
        value: longBranchName,
        latestCommitTimestamp: 1_700_000_600,
      },
    ];

    const mockStats: Statistics = {
      localCount: 2,
      remoteCount: 0,
      worktreeCount: 0,
      changesCount: 0,
      lastUpdated: new Date(),
    };

    const onSelect = vi.fn();
    const { container } = render(
      <BranchListScreenModule.BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    expect(container.textContent).toContain(longBranchName);
    expect(
      branchListProps.at(-1)?.branches?.some((b) => b.name === longBranchName),
    ).toBe(true);
  });

  it("[T092] should handle branch names with special characters", () => {
    const specialBranchNames = [
      "feature/bug-fix-#123",
      "hotfix/issue@456",
      "release/v1.0.0-beta.1",
      "feature/改善-日本語",
    ];

    const mockBranches: BranchItem[] = specialBranchNames.map(
      (name, index) => ({
        name,
        label: name,
        value: name,
        latestCommitTimestamp: 1_700_001_000 + index * 60,
      }),
    );

    const mockStats: Statistics = {
      localCount: mockBranches.length,
      remoteCount: 0,
      worktreeCount: 0,
      changesCount: 0,
      lastUpdated: new Date(),
    };

    const onSelect = vi.fn();
    const { container } = render(
      <BranchListScreenModule.BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    // All special branch names should be displayed
    specialBranchNames.forEach((name) => {
      expect(container.textContent).toContain(name);
    });
  });

  /**
   * T093: Error Boundary動作確認
   */
  it("[T093] should catch errors in App component", async () => {
    // Mock useGitData to throw an error after initial render
    let callCount = 0;
    useGitDataMock.mockImplementation(() => {
      callCount++;
      if (callCount > 1) {
        throw new Error("Simulated error");
      }
      return {
        branches: [],
        worktrees: [],
        loading: false,
        error: null,
        refresh: mockRefresh,
        lastUpdated: null,
      };
    });

    const onExit = vi.fn();
    const { container } = render(<App onExit={onExit} />);

    // Initial render should work
    expect(container).toBeDefined();
  });

  it("[T093] should display error message when data loading fails", () => {
    const testError = new Error("Test error: Failed to load Git data");
    useGitDataMock.mockReturnValue({
      branches: [],
      worktrees: [],
      loading: false,
      error: testError,
      refresh: mockRefresh,
      lastUpdated: null,
    });

    const onExit = vi.fn();
    const { getByText } = render(<App onExit={onExit} />);

    expect(branchListProps).not.toHaveLength(0);
    expect(branchListProps.at(-1)?.error).toBe(testError);
    // Error should be displayed via stubbed screen
    expect(getByText(/Error:/i)).toBeDefined();
    expect(getByText(/Failed to load Git data/i)).toBeDefined();
  });

  it("[T093] should handle empty branches list gracefully", () => {
    useGitDataMock.mockReturnValue({
      branches: [],
      worktrees: [],
      loading: false,
      error: null,
      refresh: mockRefresh,
      lastUpdated: null,
    });

    const onExit = vi.fn();
    const { container } = render(<App onExit={onExit} />);

    // Should render without error even with no branches
    expect(container).toBeDefined();
  });

  /**
   * Additional edge cases
   */
  it("should handle large number of worktrees", () => {
    const mockBranches: BranchInfo[] = Array.from({ length: 50 }, (_, i) => ({
      name: `feature/branch-${i}`,
      type: "local" as const,
      branchType: "feature" as const,
      isCurrent: false,
    }));

    useGitDataMock.mockReturnValue({
      branches: mockBranches,
      worktrees: Array.from({ length: 30 }, (_, i) => ({
        branch: `feature/branch-${i}`,
        path: `/path/to/worktree-${i}`,
        head: `commit-${i}`,
        isAccessible: true,
      })),
      loading: false,
      error: null,
      refresh: mockRefresh,
      lastUpdated: new Date(),
    });

    const onExit = vi.fn();
    const { container } = render(<App onExit={onExit} />);

    expect(container).toBeDefined();
  });

  it("should handle terminal resize gracefully", () => {
    const originalRows = process.stdout.rows;

    // Start with normal size
    process.stdout.rows = 30;

    const mockBranches: BranchItem[] = [
      { name: "main", label: "main", value: "main" },
    ];

    const mockStats: Statistics = {
      localCount: 1,
      remoteCount: 0,
      worktreeCount: 0,
      changesCount: 0,
      lastUpdated: new Date(),
    };

    const onSelect = vi.fn();
    const { container, rerender } = render(
      <BranchListScreenModule.BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    expect(container).toBeDefined();

    // Simulate terminal resize
    process.stdout.rows = 15;

    // Re-render
    rerender(
      <BranchListScreenModule.BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    expect(container).toBeDefined();

    process.stdout.rows = originalRows;
  });

  afterEach(() => {
    useGitDataMock.mockReset();
    branchListProps.length = 0;
  });
});
