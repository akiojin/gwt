/**
 * @vitest-environment happy-dom
 * Acceptance tests for User Story 3: Realtime Statistics Update
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { useGitData } from '../../hooks/useGitData.js';
import { Window } from 'happy-dom';
import type { BranchInfo } from '../../types.js';

// Mock git.js and worktree.js
vi.mock('../../../git.js', () => ({
  getAllBranches: vi.fn(),
}));

vi.mock('../../../worktree.js', () => ({
  listAdditionalWorktrees: vi.fn(),
}));

import { getAllBranches } from '../../../git.js';
import { listAdditionalWorktrees } from '../../../worktree.js';

describe('Acceptance: Realtime Update (User Story 3)', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    // Reset mocks
    (getAllBranches as ReturnType<typeof vi.fn>).mockReset();
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockReset();
  });

  /**
   * T085: Acceptance Scenario 1
   * 別ターミナルでGit操作後、数秒以内に統計情報が更新される
   */
  it('[AC1] should update statistics within seconds after Git operations', async () => {
    // Simulate initial state
    const initialBranches: BranchInfo[] = [
      { name: 'main', type: 'local', branchType: 'main', isCurrent: true },
      { name: 'feature/a', type: 'local', branchType: 'feature', isCurrent: false },
    ];

    let callCount = 0;
    (getAllBranches as ReturnType<typeof vi.fn>).mockImplementation(async () => {
      callCount++;
      if (callCount === 1) {
        return initialBranches;
      }
      // Simulate Git operation: new branch created
      return [
        ...initialBranches,
        { name: 'feature/b', type: 'local', branchType: 'feature', isCurrent: false },
      ];
    });
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    // Enable auto-refresh with 1 second interval
    const { result } = renderHook(() =>
      useGitData({ enableAutoRefresh: true, refreshInterval: 1000 })
    );

    // Wait for initial load
    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.branches).toHaveLength(2);
    const initialLastUpdated = result.current.lastUpdated;

    // Simulate Git operation happening in another terminal
    // Wait for auto-refresh (slightly more than 1 second)
    await new Promise((resolve) => setTimeout(resolve, 1100));

    // Statistics should be updated
    await waitFor(
      () => {
        expect(result.current.branches).toHaveLength(3);
      },
      { timeout: 2000 }
    );

    expect(result.current.branches[2].name).toBe('feature/b');
    expect(result.current.lastUpdated!.getTime()).toBeGreaterThan(
      initialLastUpdated!.getTime()
    );
  });

  /**
   * T086: Acceptance Scenario 2
   * Worktree作成/削除後、統計情報が即座に更新される
   */
  it('[AC2] should update statistics immediately after worktree operations', async () => {
    const mockBranches: BranchInfo[] = [
      { name: 'main', type: 'local', branchType: 'main', isCurrent: true },
      { name: 'feature/test', type: 'local', branchType: 'feature', isCurrent: false },
    ];

    let worktreeCallCount = 0;
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockImplementation(async () => {
      worktreeCallCount++;
      if (worktreeCallCount === 1) {
        return [];
      }
      // Simulate worktree creation
      return [
        {
          branch: 'feature/test',
          path: '/path/to/worktree',
          head: 'abc123',
          isAccessible: true,
        },
      ];
    });

    // Enable auto-refresh with 500ms interval
    const { result } = renderHook(() =>
      useGitData({ enableAutoRefresh: true, refreshInterval: 500 })
    );

    // Wait for initial load
    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.worktrees).toHaveLength(0);

    // Simulate worktree creation in another terminal
    // Wait for auto-refresh (slightly more than 500ms)
    await new Promise((resolve) => setTimeout(resolve, 600));

    // Worktree statistics should be updated
    await waitFor(
      () => {
        expect(result.current.worktrees).toHaveLength(1);
      },
      { timeout: 1000 }
    );

    expect(result.current.worktrees[0].branch).toBe('feature/test');
    expect(result.current.worktrees[0].path).toBe('/path/to/worktree');
  });

  /**
   * Additional: Verify lastUpdated display behavior
   */
  it('[AC3] should display lastUpdated timestamp after each refresh', async () => {
    const mockBranches: BranchInfo[] = [
      { name: 'main', type: 'local', branchType: 'main', isCurrent: true },
    ];

    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const { result } = renderHook(() =>
      useGitData({ enableAutoRefresh: true, refreshInterval: 200 })
    );

    // Wait for initial load
    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    const firstTimestamp = result.current.lastUpdated;
    expect(firstTimestamp).toBeInstanceOf(Date);

    // Wait for auto-refresh
    await new Promise((resolve) => setTimeout(resolve, 250));

    await waitFor(() => {
      expect(result.current.lastUpdated!.getTime()).toBeGreaterThan(firstTimestamp!.getTime());
    });

    const secondTimestamp = result.current.lastUpdated;
    expect(secondTimestamp).toBeInstanceOf(Date);

    // Verify timestamps are different
    expect(secondTimestamp!.getTime()).toBeGreaterThan(firstTimestamp!.getTime());
  });

  /**
   * Additional: Verify manual refresh updates lastUpdated
   */
  it('[AC4] should update lastUpdated on manual refresh', async () => {
    const mockBranches: BranchInfo[] = [
      { name: 'main', type: 'local', branchType: 'main', isCurrent: true },
    ];

    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const { result } = renderHook(() => useGitData({ enableAutoRefresh: false }));

    // Wait for initial load
    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    const firstTimestamp = result.current.lastUpdated;
    expect(firstTimestamp).toBeInstanceOf(Date);

    // Wait to ensure timestamp difference
    await new Promise((resolve) => setTimeout(resolve, 50));

    // Manual refresh
    result.current.refresh();

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    const secondTimestamp = result.current.lastUpdated;
    expect(secondTimestamp).toBeInstanceOf(Date);
    expect(secondTimestamp!.getTime()).toBeGreaterThan(firstTimestamp!.getTime());
  });
});
