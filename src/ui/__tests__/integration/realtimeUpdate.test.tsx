import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render } from 'ink-testing-library';
import React from 'react';
import { App } from '../../components/App.js';

/**
 * Real-time update integration tests
 * Tests auto-refresh functionality and lastUpdated display
 */

// Create mock functions using vi.hoisted() for Bun compatibility
const mockListBranches = vi.hoisted(() => vi.fn());
const mockListWorktrees = vi.hoisted(() => vi.fn());
const mockCreateBranch = vi.hoisted(() => vi.fn());
const mockIsGitRepository = vi.hoisted(() => vi.fn());
const mockGetRepositoryRoot = vi.hoisted(() => vi.fn());
const mockWorktreeExists = vi.hoisted(() => vi.fn());
const mockGenerateWorktreePath = vi.hoisted(() => vi.fn());
const mockCreateWorktree = vi.hoisted(() => vi.fn());

// Mock Git data functions
vi.mock('../../../git.js', () => ({
  listBranches: mockListBranches,
  listWorktrees: mockListWorktrees,
  createBranch: mockCreateBranch,
  isGitRepository: mockIsGitRepository,
  getRepositoryRoot: mockGetRepositoryRoot,
}));

// Mock worktree functions
vi.mock('../../../worktree.js', () => ({
  worktreeExists: mockWorktreeExists,
  generateWorktreePath: mockGenerateWorktreePath,
  createWorktree: mockCreateWorktree,
}));

describe('Real-time Update Integration', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it('T084: should auto-refresh data at specified intervals', async () => {
    // Initial mock data
    mockListBranches.mockResolvedValue([
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
    ]);

    mockListWorktrees.mockResolvedValue([]);

    let exitCalled = false;
    const { unmount } = render(
      <App
        onExit={() => {
          exitCalled = true;
        }}
      />
    );

    // Wait for initial load
    await vi.advanceTimersByTimeAsync(100);

    const initialCallCount = mockListBranches.mock.calls.length;

    // Advance time by 5 seconds (refresh interval)
    await vi.advanceTimersByTimeAsync(5000);

    const afterFirstRefreshCallCount = mockListBranches.mock.calls.length;

    // Should have called listBranches again
    expect(afterFirstRefreshCallCount).toBeGreaterThan(initialCallCount);

    // Advance time by another 5 seconds
    await vi.advanceTimersByTimeAsync(5000);

    const afterSecondRefreshCallCount = mockListBranches.mock.calls.length;

    // Should have called listBranches one more time
    expect(afterSecondRefreshCallCount).toBeGreaterThan(afterFirstRefreshCallCount);

    unmount();
  });

  it('T085: should update statistics after Git operations', async () => {
    // Initial data: 2 branches
    mockListBranches.mockResolvedValue([
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
    ]);

    mockListWorktrees.mockResolvedValue([]);

    let exitCalled = false;
    const { lastFrame, unmount } = render(
      <App
        onExit={() => {
          exitCalled = true;
        }}
      />
    );

    // Wait for initial load
    await vi.advanceTimersByTimeAsync(100);

    // Initial state should show "Total: 2"
    expect(lastFrame()).toContain('Total: 2');

    // Simulate Git operation: add a new branch
    mockListBranches.mockResolvedValue([
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
      {
        name: 'feature/test-2',
        branchType: 'feature',
        type: 'local',
        isCurrent: false,
      },
    ]);

    // Wait for auto-refresh (5 seconds)
    await vi.advanceTimersByTimeAsync(5000);

    // Should now show "Total: 3"
    expect(lastFrame()).toContain('Total: 3');

    unmount();
  });

  it('T086: should update statistics after Worktree creation/deletion', async () => {
    // Initial data: 2 branches, 0 worktrees
    mockListBranches.mockResolvedValue([
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
    ]);

    mockListWorktrees.mockResolvedValue([]);

    let exitCalled = false;
    const { lastFrame, unmount } = render(
      <App
        onExit={() => {
          exitCalled = true;
        }}
      />
    );

    // Wait for initial load
    await vi.advanceTimersByTimeAsync(100);

    // Initial state should show "Worktree: 0"
    expect(lastFrame()).toContain('Worktree: 0');

    // Simulate Worktree creation
    mockListBranches.mockResolvedValue([
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
    ]);

    mockListWorktrees.mockResolvedValue([
      {
        path: '/mock/worktree/feature-test-1',
        branch: 'feature/test-1',
        isAccessible: true,
      },
    ]);

    // Wait for auto-refresh (5 seconds)
    await vi.advanceTimersByTimeAsync(5000);

    // Should now show "Worktree: 1"
    expect(lastFrame()).toContain('Worktree: 1');

    unmount();
  });

  it('should display lastUpdated timestamp', async () => {
    const { listBranches, listWorktrees } = await import('../../../git.js');
    const mockListBranches = listBranches as any;
    const mockListWorktrees = listWorktrees as any;

    mockListBranches.mockResolvedValue([
      {
        name: 'main',
        branchType: 'main',
        type: 'local',
        isCurrent: true,
      },
    ]);

    mockListWorktrees.mockResolvedValue([]);

    let exitCalled = false;
    const { lastFrame, unmount } = render(
      <App
        onExit={() => {
          exitCalled = true;
        }}
      />
    );

    // Wait for initial load
    await vi.advanceTimersByTimeAsync(100);

    // Should display "Updated:" text
    expect(lastFrame()).toContain('Updated:');

    // Should display relative time (e.g., "just now")
    expect(lastFrame()).toMatch(/just now|seconds ago/);

    unmount();
  });

  it('should handle refresh errors gracefully', async () => {
    const { listBranches, listWorktrees } = await import('../../../git.js');
    const mockListBranches = listBranches as any;
    const mockListWorktrees = listWorktrees as any;

    // Initial successful load
    mockListBranches.mockResolvedValue([
      {
        name: 'main',
        branchType: 'main',
        type: 'local',
        isCurrent: true,
      },
    ]);

    mockListWorktrees.mockResolvedValue([]);

    let exitCalled = false;
    const { lastFrame, unmount } = render(
      <App
        onExit={() => {
          exitCalled = true;
        }}
      />
    );

    // Wait for initial load
    await vi.advanceTimersByTimeAsync(100);

    // Simulate error on refresh
    mockListBranches.mockRejectedValue(new Error('Git command failed'));

    // Wait for auto-refresh (5 seconds)
    await vi.advanceTimersByTimeAsync(5000);

    // Should display error message
    expect(lastFrame()).toContain('Error:');
    expect(lastFrame()).toContain('Git command failed');

    unmount();
  });
});
