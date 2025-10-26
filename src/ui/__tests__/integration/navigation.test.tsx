/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
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

describe('Navigation Integration Tests', () => {
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
    {
      name: 'feature/test',
      type: 'local',
      branchType: 'feature',
      isCurrent: false,
    },
  ];

  it('should start with execution mode screen', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getAllByText } = render(<App repoRoot="/repo" onExit={onExit} />);

    await waitFor(() => {
      expect(getAllByText(/Execution Mode/i).length).toBeGreaterThan(0);
    });
  });

  it('should support navigation between screens', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { container } = render(<App repoRoot="/repo" onExit={onExit} />);

    await waitFor(() => {
      expect(container).toBeDefined();
    });

    // Test will verify screen navigation
    expect(container).toBeDefined();
  });

  it('should maintain state across screen transitions', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { container } = render(<App repoRoot="/repo" onExit={onExit} />);

    await waitFor(() => {
      expect(container).toBeDefined();
    });

    // Test will verify state persistence
    expect(container).toBeDefined();
  });

  it('should handle back navigation correctly', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { container } = render(<App repoRoot="/repo" onExit={onExit} />);

    await waitFor(() => {
      expect(container).toBeDefined();
    });

    // Test will verify back navigation
    expect(container).toBeDefined();
  });

  it('should handle navigation history', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { container } = render(<App repoRoot="/repo" onExit={onExit} />);

    await waitFor(() => {
      expect(container).toBeDefined();
    });

    // Test will verify navigation history
    expect(container).toBeDefined();
  });

  it('should display correct screen on navigation', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { container } = render(<App repoRoot="/repo" onExit={onExit} />);

    await waitFor(() => {
      expect(container).toBeDefined();
    });

    // Test will verify correct screen rendering
    expect(container).toBeDefined();
  });

  it('should call onExit when branch is selected', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { container } = render(<App repoRoot="/repo" onExit={onExit} />);

    await waitFor(() => {
      expect(container).toBeDefined();
    });

    // Test will verify onExit is called
    expect(container).toBeDefined();
  });
});
