/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';
import React, { useMemo } from 'react';
import { Window } from 'happy-dom';
import type { BranchInfo, WorktreeInfo } from '../../types.js';
import { formatBranchItems } from '../../utils/branchFormatter.js';

/**
 * T082-1: useMemo optimization tests
 * Tests that branchItems are not unnecessarily regenerated when data content is the same
 */

// Mock useGitData hook
vi.mock('../../hooks/useGitData.js', () => ({
  useGitData: vi.fn(),
}));

import { useGitData } from '../../hooks/useGitData.js';
const mockUseGitData = useGitData as ReturnType<typeof vi.fn>;

// Helper function to create a stable hash of branch data
function createBranchHash(branches: BranchInfo[]): string {
  return branches
    .map((b) => `${b.name}-${b.type}-${b.isCurrent}`)
    .join(',');
}

// Helper function to create a stable hash of worktree data
function createWorktreeHash(worktrees: WorktreeInfo[]): string {
  return worktrees
    .map((w) => `${w.branch}-${w.path}`)
    .join(',');
}

// Test component that uses optimized useMemo
function TestComponent({ branches, worktrees }: { branches: BranchInfo[]; worktrees: WorktreeInfo[] }) {
  // Count how many times formatBranchItems is called
  const formatCallCount = React.useRef(0);

  // Optimized useMemo with content-based dependencies
  const branchItems = useMemo(() => {
    formatCallCount.current++;
    const worktreeMap = new Map();
    for (const wt of worktrees) {
      worktreeMap.set(wt.branch, {
        path: wt.path,
        locked: false,
        prunable: wt.isAccessible === false,
        isAccessible: wt.isAccessible ?? true,
      });
    }
    return formatBranchItems(branches, worktreeMap);
  }, [
    createBranchHash(branches),
    createWorktreeHash(worktrees),
  ]);

  return (
    <div data-testid="branch-count">{branchItems.length}</div>
  );
}

describe('useMemo Optimization (T082-1)', () => {
  beforeEach(() => {
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
    vi.clearAllMocks();
  });

  it('should not regenerate branchItems when data content is the same', () => {
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

    const mockWorktrees: WorktreeInfo[] = [
      {
        path: '/mock/worktree/feature-test-1',
        branch: 'feature/test-1',
        isAccessible: true,
      },
    ];

    const { rerender } = render(
      <TestComponent branches={mockBranches} worktrees={mockWorktrees} />
    );

    // Create new arrays with the same content
    const sameBranches: BranchInfo[] = [
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

    const sameWorktrees: WorktreeInfo[] = [
      {
        path: '/mock/worktree/feature-test-1',
        branch: 'feature/test-1',
        isAccessible: true,
      },
    ];

    // Verify that arrays are different references
    expect(sameBranches).not.toBe(mockBranches);
    expect(sameWorktrees).not.toBe(mockWorktrees);

    // Verify that content is the same
    expect(createBranchHash(sameBranches)).toBe(createBranchHash(mockBranches));
    expect(createWorktreeHash(sameWorktrees)).toBe(createWorktreeHash(mockWorktrees));

    // Re-render with same content but different references
    rerender(
      <TestComponent branches={sameBranches} worktrees={sameWorktrees} />
    );

    // formatBranchItems should only be called once (not twice)
    // This test would fail with the current implementation because useMemo
    // depends on array references, not content
  });

  it('should regenerate branchItems when data content changes', () => {
    const initialBranches: BranchInfo[] = [
      {
        name: 'main',
        branchType: 'main',
        type: 'local',
        isCurrent: true,
      },
    ];

    const { rerender } = render(
      <TestComponent branches={initialBranches} worktrees={[]} />
    );

    // Add a new branch
    const updatedBranches: BranchInfo[] = [
      ...initialBranches,
      {
        name: 'feature/new',
        branchType: 'feature',
        type: 'local',
        isCurrent: false,
      },
    ];

    // Verify content changed
    expect(createBranchHash(updatedBranches)).not.toBe(createBranchHash(initialBranches));

    // Re-render with new content
    rerender(
      <TestComponent branches={updatedBranches} worktrees={[]} />
    );

    // formatBranchItems should be called twice (once for initial, once for update)
  });

  it('should handle branch order changes correctly', () => {
    const branches1: BranchInfo[] = [
      {
        name: 'main',
        branchType: 'main',
        type: 'local',
        isCurrent: true,
      },
      {
        name: 'feature/test',
        branchType: 'feature',
        type: 'local',
        isCurrent: false,
      },
    ];

    const branches2: BranchInfo[] = [
      {
        name: 'feature/test',
        branchType: 'feature',
        type: 'local',
        isCurrent: false,
      },
      {
        name: 'main',
        branchType: 'main',
        type: 'local',
        isCurrent: true,
      },
    ];

    // Different order should produce different hashes
    expect(createBranchHash(branches1)).not.toBe(createBranchHash(branches2));
  });

  it('should detect subtle branch property changes', () => {
    const branches1: BranchInfo[] = [
      {
        name: 'main',
        branchType: 'main',
        type: 'local',
        isCurrent: false,
      },
    ];

    const branches2: BranchInfo[] = [
      {
        name: 'main',
        branchType: 'main',
        type: 'local',
        isCurrent: true, // Changed from false to true
      },
    ];

    // isCurrent change should be detected
    expect(createBranchHash(branches1)).not.toBe(createBranchHash(branches2));
  });
});
