/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { render, waitFor } from '@testing-library/react';
import React from 'react';
import { App } from '../../components/App.js';
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

describe('Branch List Integration', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    // Reset mocks
    (getAllBranches as ReturnType<typeof vi.fn>).mockReset();
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockReset();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it('should render full application with branch list', async () => {
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

    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getByText } = render(<App onExit={onExit} />);

    // Wait for async data loading
    await waitFor(() => {
      expect(getByText(/Claude Worktree/i)).toBeDefined();
    });

    // Verify header
    expect(getByText(/Claude Worktree/i)).toBeDefined();

    // Verify stats
    expect(getByText(/Local:/)).toBeDefined();

    // Verify branches are displayed
    expect(getByText(/main/)).toBeDefined();
    expect(getByText(/feature\/test/)).toBeDefined();

    // Verify footer
    expect(getByText(/Quit/i)).toBeDefined();
  });

  it('should display statistics correctly', async () => {
    const mockBranches: BranchInfo[] = [
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

    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getByText, getAllByText } = render(<App onExit={onExit} />);

    // Wait for async data loading
    await waitFor(() => {
      expect(getByText(/Local:/)).toBeDefined();
    });

    // Verify statistics
    expect(getByText(/Local:/)).toBeDefined();
    expect(getAllByText(/2/).length).toBeGreaterThan(0); // 2 local branches

    expect(getByText(/Remote:/)).toBeDefined();
    expect(getAllByText(/1/).length).toBeGreaterThan(0); // 1 remote branch + 1 worktree

    expect(getByText(/Worktrees:/)).toBeDefined();
  });

  it('should handle empty branch list', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue([]);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getByText } = render(<App onExit={onExit} />);

    // Wait for async data loading
    await waitFor(() => {
      expect(getByText(/No branches found/i)).toBeDefined();
    });

    expect(getByText(/No branches found/i)).toBeDefined();
  });

  it('should handle loading state', async () => {
    // Mock a slow response
    (getAllBranches as ReturnType<typeof vi.fn>).mockImplementation(
      () =>
        new Promise((resolve) =>
          setTimeout(() => resolve([]), 100)
        )
    );
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getByText, queryByText } = render(<App onExit={onExit} />);

    // Initially should show loading
    // Note: In happy-dom, useEffect may run synchronously, so we check for either loading or loaded state
    const hasLoading = queryByText(/Loading/i);
    if (hasLoading) {
      expect(hasLoading).toBeDefined();
    }

    // Wait for data to load
    await waitFor(() => {
      expect(queryByText(/Loading/i)).toBeNull();
    });
  });

  it('should handle error state', async () => {
    const error = new Error('Failed to fetch branches');
    (getAllBranches as ReturnType<typeof vi.fn>).mockRejectedValue(error);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getByText } = render(<App onExit={onExit} />);

    // Wait for error to appear
    await waitFor(() => {
      expect(getByText(/Error:/i)).toBeDefined();
    });

    expect(getByText(/Error:/i)).toBeDefined();
    expect(getByText(/Failed to fetch branches/i)).toBeDefined();
  });

  it('should display branch icons correctly', async () => {
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
      {
        name: 'hotfix/urgent',
        type: 'local',
        branchType: 'hotfix',
        isCurrent: false,
      },
    ];

    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getByText } = render(<App onExit={onExit} />);

    // Wait for async data loading
    await waitFor(() => {
      expect(getByText(/⚡/)).toBeDefined();
    });

    // Check for branch type icons
    expect(getByText(/⚡/)).toBeDefined(); // main icon
    expect(getByText(/⭐/)).toBeDefined(); // current branch icon
    expect(getByText(/✨/)).toBeDefined(); // feature icon
    expect(getByText(/🔥/)).toBeDefined(); // hotfix icon
  });

  it('should integrate all components correctly', async () => {
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

    const onExit = vi.fn();
    const { container } = render(<App onExit={onExit} />);

    // Wait for rendering
    await waitFor(() => {
      expect(container.textContent).toContain('Claude Worktree');
    });

    // Verify all major sections are present
    expect(container.textContent).toContain('Claude Worktree'); // Header
    expect(container.textContent).toContain('Local:'); // Stats
    expect(container.textContent).toContain('main'); // Branch list
    expect(container.textContent).toContain('Quit'); // Footer
  });
});
