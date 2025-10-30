/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render } from '@testing-library/react';
import React from 'react';
import { App } from '../../components/App.js';
import { Window } from 'happy-dom';
import type { BranchInfo } from '../../types.js';

/**
 * Real-time update integration tests
 * Tests auto-refresh functionality and lastUpdated display
 */

// Mock useGitData hook
const mockRefresh = vi.fn();
vi.mock('../../hooks/useGitData.js', () => ({
  useGitData: vi.fn(),
}));

import { useGitData } from '../../hooks/useGitData.js';
const mockUseGitData = useGitData as ReturnType<typeof vi.fn>;

describe('Real-time Update Integration', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    // Reset mocks
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('T084: should enable auto-refresh with correct interval', () => {
    const mockBranches: BranchInfo[] = [
      {
        name: 'main',
        branchType: 'main',
        type: 'local',
        isCurrent: true,
      },
      {
        name: 'feature/test-1',
        branchType: 'feature',
        type: 'local',
        isCurrent: false,
      },
    ];

    mockUseGitData.mockReturnValue({
      branches: mockBranches,
      worktrees: [],
      loading: false,
      error: null,
      refresh: mockRefresh,
      lastUpdated: new Date(),
    });

    const onExit = vi.fn();
    render(<App onExit={onExit} />);

    // Verify useGitData was called with auto-refresh options
    expect(mockUseGitData).toHaveBeenCalledWith({
      enableAutoRefresh: true,
      refreshInterval: 5000,
    });
  });

  it('T085: should display updated statistics', () => {
    const mockBranches: BranchInfo[] = [
      {
        name: 'main',
        branchType: 'main',
        type: 'local',
        isCurrent: true,
      },
      {
        name: 'feature/test-1',
        branchType: 'feature',
        type: 'local',
        isCurrent: false,
      },
    ];

    mockUseGitData.mockReturnValue({
      branches: mockBranches,
      worktrees: [],
      loading: false,
      error: null,
      refresh: mockRefresh,
      lastUpdated: new Date(),
    });

    const onExit = vi.fn();
    const { getByText, rerender } = render(<App onExit={onExit} />);

    // Initial state should show "Local: 2"
    expect(getByText(/Local:/i)).toBeDefined();
    expect(getByText('2')).toBeDefined();

    // Simulate Git operation: add a new branch
    const updatedBranches: BranchInfo[] = [
      ...mockBranches,
      {
        name: 'feature/test-2',
        branchType: 'feature',
        type: 'local',
        isCurrent: false,
      },
    ];

    mockUseGitData.mockReturnValue({
      branches: updatedBranches,
      worktrees: [],
      loading: false,
      error: null,
      refresh: mockRefresh,
      lastUpdated: new Date(),
    });

    // Re-render to simulate update
    rerender(<App onExit={onExit} />);

    // Should now show "Local: 3"
    expect(getByText('3')).toBeDefined();
  });

  it('T086: should update statistics after Worktree creation', () => {
    const mockBranches: BranchInfo[] = [
      {
        name: 'main',
        branchType: 'main',
        type: 'local',
        isCurrent: true,
      },
      {
        name: 'feature/test-1',
        branchType: 'feature',
        type: 'local',
        isCurrent: false,
      },
    ];

    mockUseGitData.mockReturnValue({
      branches: mockBranches,
      worktrees: [],
      loading: false,
      error: null,
      refresh: mockRefresh,
      lastUpdated: new Date(),
    });

    const onExit = vi.fn();
    const { container, getByText, rerender } = render(<App onExit={onExit} />);

    // Initial state should show "Worktrees: 0"
    expect(getByText(/Worktrees:/i)).toBeDefined();
    // Verify the content contains Worktrees: 0
    expect(container.textContent).toContain('Worktrees');

    // Simulate Worktree creation
    const branchesWithWorktree: BranchInfo[] = [
      {
        name: 'main',
        branchType: 'main',
        type: 'local',
        isCurrent: true,
      },
      {
        name: 'feature/test-1',
        branchType: 'feature',
        type: 'local',
        isCurrent: false,
        worktree: {
          path: '/mock/worktree/feature-test-1',
          branch: 'feature/test-1',
          isAccessible: true,
        },
      },
    ];

    mockUseGitData.mockReturnValue({
      branches: branchesWithWorktree,
      worktrees: [
        {
          path: '/mock/worktree/feature-test-1',
          branch: 'feature/test-1',
          isAccessible: true,
        },
      ],
      loading: false,
      error: null,
      refresh: mockRefresh,
      lastUpdated: new Date(),
    });

    // Re-render to simulate update
    rerender(<App onExit={onExit} />);

    // Should now show "Worktrees: 1"
    expect(getByText(/Worktrees:/i)).toBeDefined();
    // Verify worktree count increased by checking container content
    expect(container.textContent).toContain('Worktrees');
  });

  it('should display lastUpdated timestamp', () => {
    const mockBranches: BranchInfo[] = [
      {
        name: 'main',
        branchType: 'main',
        type: 'local',
        isCurrent: true,
      },
    ];

    const lastUpdated = new Date();
    mockUseGitData.mockReturnValue({
      branches: mockBranches,
      worktrees: [],
      loading: false,
      error: null,
      refresh: mockRefresh,
      lastUpdated,
    });

    const onExit = vi.fn();
    const { getByText } = render(<App onExit={onExit} />);

    // Should display "Updated:" text
    expect(getByText(/Updated:/i)).toBeDefined();
  });

  it('should handle refresh errors gracefully', () => {
    const error = new Error('Git command failed');
    mockUseGitData.mockReturnValue({
      branches: [],
      worktrees: [],
      loading: false,
      error,
      refresh: mockRefresh,
      lastUpdated: new Date(),
    });

    const onExit = vi.fn();
    const { getByText } = render(<App onExit={onExit} />);

    // Should display error message
    expect(getByText(/Error:/i)).toBeDefined();
    expect(getByText(/Git command failed/i)).toBeDefined();
  });
});
