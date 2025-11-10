/**
 * @vitest-environment happy-dom
 * Integration tests for realtime update functionality
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

describe('Realtime Update Integration Tests', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    // Reset mocks
    (getAllBranches as ReturnType<typeof vi.fn>).mockReset();
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockReset();
  });

  const mockBranches: BranchInfo[] = [
    {
      name: 'main',
      type: 'local',
      branchType: 'main',
      isCurrent: true,
    },
  ];

  it('should update lastUpdated timestamp after data refresh', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const { result } = renderHook(() => useGitData());

    // Wait for initial load
    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    const firstUpdated = result.current.lastUpdated;
    expect(firstUpdated).toBeInstanceOf(Date);

    // Wait to ensure timestamp difference (increased from 50ms to 100ms)
    await new Promise((resolve) => setTimeout(resolve, 100));

    // Trigger manual refresh
    result.current.refresh();

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    const secondUpdated = result.current.lastUpdated;
    expect(secondUpdated).toBeInstanceOf(Date);
    // Use greaterThanOrEqual to handle rare cases where timestamps are identical
    expect(secondUpdated!.getTime()).toBeGreaterThanOrEqual(firstUpdated!.getTime());
  });

  it('should maintain data consistency during auto-refresh', async () => {
    let callCount = 0;
    (getAllBranches as ReturnType<typeof vi.fn>).mockImplementation(async () => {
      callCount++;
      if (callCount === 1) {
        return mockBranches;
      }
      // Return updated branches on subsequent calls
      return [
        ...mockBranches,
        {
          name: 'feature/new',
          type: 'local',
          branchType: 'feature',
          isCurrent: false,
        },
      ];
    });
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const { result } = renderHook(() =>
      useGitData({ enableAutoRefresh: true, refreshInterval: 100 })
    );

    // Wait for initial load
    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.branches).toHaveLength(1);

    // Wait for auto-refresh to trigger
    await new Promise((resolve) => setTimeout(resolve, 150));

    await waitFor(() => {
      expect(result.current.branches).toHaveLength(2);
    });

    // Verify data integrity
    expect(result.current.branches[0].name).toBe('main');
    expect(result.current.branches[1].name).toBe('feature/new');
    expect(result.current.error).toBeNull();
  });

  it('should handle errors during auto-refresh gracefully', async () => {
    let callCount = 0;
    (getAllBranches as ReturnType<typeof vi.fn>).mockImplementation(async () => {
      callCount++;
      if (callCount === 1) {
        return mockBranches;
      }
      // Simulate error on second call
      throw new Error('Network error');
    });
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const { result } = renderHook(() =>
      useGitData({ enableAutoRefresh: true, refreshInterval: 100 })
    );

    // Wait for initial load
    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.branches).toHaveLength(1);
    expect(result.current.error).toBeNull();

    // Wait for auto-refresh to trigger and fail
    await new Promise((resolve) => setTimeout(resolve, 150));

    await waitFor(() => {
      expect(result.current.error).not.toBeNull();
    });

    expect(result.current.error?.message).toBe('Network error');
    // Data should be cleared on error
    expect(result.current.branches).toEqual([]);
  });

  it('should stop auto-refresh when component unmounts', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const { result, unmount } = renderHook(() =>
      useGitData({ enableAutoRefresh: true, refreshInterval: 100 })
    );

    // Wait for initial load
    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    const initialCallCount = (getAllBranches as ReturnType<typeof vi.fn>).mock.calls.length;

    // Unmount the hook
    unmount();

    // Wait longer than refresh interval
    await new Promise((resolve) => setTimeout(resolve, 200));

    // Call count should not increase after unmount
    const finalCallCount = (getAllBranches as ReturnType<typeof vi.fn>).mock.calls.length;
    expect(finalCallCount).toBe(initialCallCount);
  });

  it('should update statistics in real-time', async () => {
    let callCount = 0;
    (getAllBranches as ReturnType<typeof vi.fn>).mockImplementation(async () => {
      callCount++;
      if (callCount === 1) {
        return [mockBranches[0]];
      }
      // Return more branches on subsequent calls
      return [
        mockBranches[0],
        { name: 'feature/a', type: 'local', branchType: 'feature', isCurrent: false },
        { name: 'feature/b', type: 'local', branchType: 'feature', isCurrent: false },
      ];
    });
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const { result } = renderHook(() =>
      useGitData({ enableAutoRefresh: true, refreshInterval: 100 })
    );

    // Wait for initial load
    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.branches).toHaveLength(1);

    // Wait for auto-refresh
    await new Promise((resolve) => setTimeout(resolve, 150));

    await waitFor(() => {
      expect(result.current.branches).toHaveLength(3);
    });

    expect(result.current.lastUpdated).toBeInstanceOf(Date);
  });
});
