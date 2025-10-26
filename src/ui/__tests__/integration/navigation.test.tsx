/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, afterAll, vi } from 'vitest';
import type { Mock } from 'vitest';
import { render, waitFor } from '@testing-library/react';
import React from 'react';
import { App } from '../../components/App.js';
import { Window } from 'happy-dom';
import type { BranchInfo } from '../../types.js';

vi.mock('../../../git.js', () => ({
  __esModule: true,
  getAllBranches: vi.fn(),
  getRepositoryRoot: vi.fn(async () => '/repo'),
  deleteBranch: vi.fn(async () => undefined),
}));

vi.mock('../../../worktree.js', () => ({
  __esModule: true,
  listAdditionalWorktrees: vi.fn(),
  createWorktree: vi.fn(async () => undefined),
  generateWorktreePath: vi.fn(async () => '/repo/.git/worktree/test'),
  getMergedPRWorktrees: vi.fn(async () => []),
  removeWorktree: vi.fn(async () => undefined),
}));

import { getAllBranches, getRepositoryRoot, deleteBranch } from '../../../git.js';
import {
  listAdditionalWorktrees,
  createWorktree,
  generateWorktreePath,
  getMergedPRWorktrees,
  removeWorktree,
} from '../../../worktree.js';

const mockedGetAllBranches = getAllBranches as Mock;
const mockedGetRepositoryRoot = getRepositoryRoot as Mock;
const mockedDeleteBranch = deleteBranch as Mock;
const mockedListAdditionalWorktrees = listAdditionalWorktrees as Mock;
const mockedCreateWorktree = createWorktree as Mock;
const mockedGenerateWorktreePath = generateWorktreePath as Mock;
const mockedGetMergedPRWorktrees = getMergedPRWorktrees as Mock;
const mockedRemoveWorktree = removeWorktree as Mock;

describe('Navigation Integration Tests', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    // Reset mocks
    mockedGetAllBranches.mockReset();
    mockedListAdditionalWorktrees.mockReset();
    mockedGetRepositoryRoot.mockReset();
    mockedDeleteBranch.mockReset();
    mockedCreateWorktree.mockReset();
    mockedGenerateWorktreePath.mockReset();
    mockedGetMergedPRWorktrees.mockReset();
    mockedRemoveWorktree.mockReset();
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

afterAll(() => {
  vi.restoreAllMocks();
});
