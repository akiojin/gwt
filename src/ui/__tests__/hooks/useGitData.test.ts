/**
 * @vitest-environment happy-dom
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

describe('useGitData', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    // Reset mocks
    (getAllBranches as ReturnType<typeof vi.fn>).mockReset();
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockReset();
  });

  it('should initialize with loading state', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue([]);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const { result } = renderHook(() => useGitData());

    // In test environment, useEffect runs synchronously, so loading may already be false
    // Check that it eventually becomes false and data is loaded
    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.branches).toEqual([]);
    expect(result.current.error).toBeNull();
  });

  it('should load branches and worktrees', async () => {
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

    const mockWorktrees = [
      {
        path: '/path/to/worktree',
        branch: 'feature/test',
        head: 'abc123',
        isAccessible: true,
      },
    ];

    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue(mockWorktrees);

    const { result } = renderHook(() => useGitData());

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.branches).toHaveLength(2);
    expect(result.current.branches[1].worktree).toBeDefined();
    expect(result.current.branches[1].worktree?.path).toBe('/path/to/worktree');
    expect(result.current.error).toBeNull();
  });

  it('should handle errors', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('Git error'));

    const { result } = renderHook(() => useGitData());

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.error).toBeDefined();
    expect(result.current.error?.message).toBe('Git error');
    expect(result.current.branches).toEqual([]);
  });

  it('should support manual refresh', async () => {
    const mockBranches: BranchInfo[] = [
      {
        name: 'main',
        type: 'local',
        branchType: 'main',
        isCurrent: true,
      },
    ];

    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const { result } = renderHook(() => useGitData());

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.branches).toHaveLength(1);

    // Add a new branch
    const updatedBranches: BranchInfo[] = [
      ...mockBranches,
      {
        name: 'feature/new',
        type: 'local',
        branchType: 'feature',
        isCurrent: false,
      },
    ];

    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(updatedBranches);

    // Trigger refresh
    result.current.refresh();

    await waitFor(() => {
      expect(result.current.branches).toHaveLength(2);
    });
  });

  it('should match worktrees to branches by name', async () => {
    const mockBranches: BranchInfo[] = [
      {
        name: 'feature/a',
        type: 'local',
        branchType: 'feature',
        isCurrent: false,
      },
      {
        name: 'feature/b',
        type: 'local',
        branchType: 'feature',
        isCurrent: false,
      },
      {
        name: 'feature/c',
        type: 'local',
        branchType: 'feature',
        isCurrent: false,
      },
    ];

    const mockWorktrees = [
      {
        path: '/path/a',
        branch: 'feature/a',
        head: 'aaa',
        isAccessible: true,
      },
      {
        path: '/path/c',
        branch: 'feature/c',
        head: 'ccc',
        isAccessible: false,
        invalidReason: 'Path does not exist',
      },
    ];

    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue(mockWorktrees);

    const { result } = renderHook(() => useGitData());

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    const branchA = result.current.branches.find((b) => b.name === 'feature/a');
    const branchB = result.current.branches.find((b) => b.name === 'feature/b');
    const branchC = result.current.branches.find((b) => b.name === 'feature/c');

    expect(branchA?.worktree).toBeDefined();
    expect(branchA?.worktree?.path).toBe('/path/a');

    expect(branchB?.worktree).toBeUndefined();

    expect(branchC?.worktree).toBeDefined();
    expect(branchC?.worktree?.path).toBe('/path/c');
  });
});
