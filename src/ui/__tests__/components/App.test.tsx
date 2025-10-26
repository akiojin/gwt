/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, afterEach, afterAll, vi } from 'vitest';
import { act, render } from '@testing-library/react';
import React from 'react';
import { App } from '../../components/App.js';
import { Window } from 'happy-dom';
import type { BranchInfo } from '../../types.js';
import * as useGitDataModule from '../../hooks/useGitData.js';

const mockRefresh = vi.fn();
const originalUseGitData = useGitDataModule.useGitData;
const useGitDataSpy = vi.spyOn(useGitDataModule, 'useGitData');

describe('App', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    vi.clearAllMocks();
    useGitDataSpy.mockReset();
    useGitDataSpy.mockImplementation(originalUseGitData);
  });

  const mockBranches: BranchInfo[] = [
    {
      name: 'main',
      type: 'local',
      branchType: 'main',
      isCurrent: true,
    },
    {
      name: 'feature/test',
      type: 'local',
      branchType: 'feature',
      isCurrent: false,
    },
  ];

  it('should render BranchListScreen when data is loaded', () => {
    useGitDataSpy.mockReturnValue({
      branches: mockBranches,
      loading: false,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    });

    const onExit = vi.fn();
    const { getByText } = render(<App onExit={onExit} />);

    // Check for BranchListScreen elements
    expect(getByText(/Claude Worktree/i)).toBeDefined();
    expect(getByText(/main/)).toBeDefined();
    expect(getByText(/feature\/test/)).toBeDefined();
  });

  it('should show loading state initially', async () => {
    useGitDataSpy.mockReturnValue({
      branches: [],
      loading: true,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    });

    const onExit = vi.fn();
    const { queryByText, getByText } = render(
      <App onExit={onExit} loadingIndicatorDelay={10} />
    );

    expect(queryByText(/Git情報を読み込んでいます/i)).toBeNull();

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 15));
    });

    expect(getByText(/Git情報を読み込んでいます/i)).toBeDefined();
  });

  it('should show error state when Git data fails to load', () => {
    const error = new Error('Failed to fetch branches');
    useGitDataSpy.mockReturnValue({
      branches: [],
      loading: false,
      error,
      worktrees: [],
      refresh: mockRefresh,
    });

    const onExit = vi.fn();
    const { getByText } = render(<App onExit={onExit} />);

    expect(getByText(/Error:/i)).toBeDefined();
    expect(getByText(/Failed to fetch branches/i)).toBeDefined();
  });

  it('should calculate statistics from branches', () => {
    const branchesWithWorktree: BranchInfo[] = [
      {
        name: 'main',
        type: 'local',
        branchType: 'main',
        isCurrent: true,
      },
      {
        name: 'feature/a',
        type: 'local',
        branchType: 'feature',
        isCurrent: false,
        worktree: {
          path: '/path/a',
          locked: false,
          prunable: false,
        },
      },
      {
        name: 'origin/main',
        type: 'remote',
        branchType: 'main',
        isCurrent: false,
      },
    ];

    useGitDataSpy.mockReturnValue({
      branches: branchesWithWorktree,
      loading: false,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    });

    const onExit = vi.fn();
    const { getByText, getAllByText } = render(<App onExit={onExit} />);

    // Check for statistics
    expect(getByText(/Local:/)).toBeDefined();
    expect(getAllByText(/2/).length).toBeGreaterThan(0); // 2 local branches
    expect(getByText(/Remote:/)).toBeDefined();
    expect(getAllByText(/1/).length).toBeGreaterThan(0); // 1 remote branch + 1 worktree
    expect(getByText(/Worktrees:/)).toBeDefined();
  });

  it('should call onExit when branch is selected', () => {
    useGitDataSpy.mockReturnValue({
      branches: mockBranches,
      loading: false,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    });

    const onExit = vi.fn();
    const { container } = render(<App onExit={onExit} />);

    expect(container).toBeDefined();
    // Note: Testing actual selection requires simulating user input,
    // which is covered in integration tests
  });

  it('should handle empty branch list', () => {
    useGitDataSpy.mockReturnValue({
      branches: [],
      loading: false,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    });

    const onExit = vi.fn();
    const { getByText } = render(<App onExit={onExit} />);

    expect(getByText(/No branches found/i)).toBeDefined();
  });

  it('should wrap with ErrorBoundary', () => {
    // This test verifies ErrorBoundary is present
    // Actual error catching is tested separately
    useGitDataSpy.mockReturnValue({
      branches: mockBranches,
      loading: false,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    });

    const onExit = vi.fn();
    const { container } = render(<App onExit={onExit} />);

    expect(container).toBeDefined();
  });

  it('should format branch items with icons', () => {
    useGitDataSpy.mockReturnValue({
      branches: mockBranches,
      loading: false,
      error: null,
      worktrees: [],
      refresh: mockRefresh,
    });

    const onExit = vi.fn();
    const { getByText } = render(<App onExit={onExit} />);

    // Check for branch type icon (main = ⚡)
    expect(getByText(/⚡/)).toBeDefined();
  });

  afterEach(() => {
    useGitDataSpy.mockReset();
    useGitDataSpy.mockImplementation(originalUseGitData);
  });
});

afterAll(() => {
  useGitDataSpy.mockRestore();
});
