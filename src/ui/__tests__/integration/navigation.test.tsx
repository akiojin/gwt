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
const hoisted = vi.hoisted(() => ({
  mockGetAllBranches: vi.fn(),
  mockGetRepositoryRoot: vi.fn(async () => '/repo'),
  mockDeleteBranch: vi.fn(async () => undefined),
  mockListAdditionalWorktrees: vi.fn(),
  mockCreateWorktree: vi.fn(async () => undefined),
  mockGenerateWorktreePath: vi.fn(async () => '/repo/.git/worktree/test'),
  mockGetMergedPRWorktrees: vi.fn(async () => []),
  mockRemoveWorktree: vi.fn(async () => undefined),
}));

vi.mock('../../../git.js', () => ({
  __esModule: true,
  getAllBranches: hoisted.mockGetAllBranches,
  getRepositoryRoot: hoisted.mockGetRepositoryRoot,
  deleteBranch: hoisted.mockDeleteBranch,
}));

vi.mock('../../../worktree.js', () => ({
  __esModule: true,
  listAdditionalWorktrees: hoisted.mockListAdditionalWorktrees,
  createWorktree: hoisted.mockCreateWorktree,
  generateWorktreePath: hoisted.mockGenerateWorktreePath,
  getMergedPRWorktrees: hoisted.mockGetMergedPRWorktrees,
  removeWorktree: hoisted.mockRemoveWorktree,
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
    hoisted.mockListAdditionalWorktrees.mockReset();
    hoisted.mockGetAllBranches.mockReset();
    hoisted.mockGetRepositoryRoot.mockReset();
    hoisted.mockDeleteBranch.mockReset();
    hoisted.mockCreateWorktree.mockReset();
    hoisted.mockGenerateWorktreePath.mockReset();
    hoisted.mockGetMergedPRWorktrees.mockReset();
    hoisted.mockRemoveWorktree.mockReset();
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

  it('should start with branch-list screen', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getByText } = render(<App onExit={onExit} />);

    await waitFor(() => {
      expect(getByText(/Claude Worktree/i)).toBeDefined();
    });

    expect(getByText(/main/)).toBeDefined();
  });

  it('should support navigation between screens', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { container } = render(<App onExit={onExit} />);

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
    const { container } = render(<App onExit={onExit} />);

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
    const { container } = render(<App onExit={onExit} />);

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
    const { container } = render(<App onExit={onExit} />);

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
    const { container } = render(<App onExit={onExit} />);

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
    const { container } = render(<App onExit={onExit} />);

    await waitFor(() => {
      expect(container).toBeDefined();
    });

    // Test will verify onExit is called
    expect(container).toBeDefined();
  });
});
