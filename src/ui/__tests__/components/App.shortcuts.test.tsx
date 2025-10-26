/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, act, waitFor } from '@testing-library/react';
import React from 'react';
import type { CleanupTarget } from '../../types.js';

const navigateToMock = vi.fn();
const goBackMock = vi.fn();
const resetMock = vi.fn();

const worktreeScreenProps: any[] = [];
const branchCreatorProps: any[] = [];
const prCleanupProps: any[] = [];

const hoisted = vi.hoisted(() => {
  const mockedGetMergedPRWorktrees = vi.fn(async (): Promise<CleanupTarget[]> => [
    {
      branch: 'feature/add-new-feature',
      cleanupType: 'worktree-and-branch',
      pullRequest: {
        number: 123,
        title: 'Add new feature',
        branch: 'feature/add-new-feature',
        mergedAt: '2025-01-20T10:00:00Z',
        author: 'user1',
      },
      worktreePath: '/worktrees/feature-add-new-feature',
      hasUncommittedChanges: false,
      hasUnpushedCommits: false,
      hasRemoteBranch: true,
      isAccessible: true,
    },
  ]);
  return {
    mockedGetMergedPRWorktrees,
    mockedGenerateWorktreePath: vi.fn(async () => '/worktrees/new-branch'),
    mockedCreateWorktree: vi.fn(async () => undefined),
    mockedRemoveWorktree: vi.fn(async () => undefined),
    mockedGetRepositoryRoot: vi.fn(async () => '/repo'),
    mockedDeleteBranch: vi.fn(async () => undefined),
  };
});

const {
  mockedGetMergedPRWorktrees,
  mockedGenerateWorktreePath,
  mockedCreateWorktree,
  mockedRemoveWorktree,
} = hoisted;

vi.mock('../../hooks/useGitData.js', () => ({
  useGitData: vi.fn(() => ({
    branches: [],
    worktrees: [
      {
        branch: 'feature/existing',
        path: '/worktrees/feature-existing',
        isAccessible: true,
      },
    ],
    loading: false,
    error: null,
    refresh: vi.fn(),
    lastUpdated: null,
  })),
}));

vi.mock('../../hooks/useScreenState.js', () => ({
  useScreenState: vi.fn(() => ({
    currentScreen: 'worktree-manager',
    navigateTo: navigateToMock,
    goBack: goBackMock,
    reset: resetMock,
  })),
}));

vi.mock('../../components/screens/WorktreeManagerScreen.js', () => ({
  WorktreeManagerScreen: (props: any) => {
    worktreeScreenProps.push(props);
    return null;
  },
}));

vi.mock('../../components/screens/BranchCreatorScreen.js', () => ({
  BranchCreatorScreen: (props: any) => {
    branchCreatorProps.push(props);
    return null;
  },
}));

vi.mock('../../components/screens/PRCleanupScreen.js', () => ({
  PRCleanupScreen: (props: any) => {
    prCleanupProps.push(props);
    return null;
  },
}));

vi.mock('../../../worktree.js', () => ({
  __esModule: true,
  getMergedPRWorktrees: hoisted.mockedGetMergedPRWorktrees,
  generateWorktreePath: hoisted.mockedGenerateWorktreePath,
  createWorktree: hoisted.mockedCreateWorktree,
  removeWorktree: hoisted.mockedRemoveWorktree,
}));

vi.mock('../../../git.js', () => ({
  __esModule: true,
  getRepositoryRoot: hoisted.mockedGetRepositoryRoot,
  deleteBranch: hoisted.mockedDeleteBranch,
}));

import { useScreenState } from '../../hooks/useScreenState.js';
import { App } from '../../components/App.js';

const mockedUseScreenState = useScreenState as unknown as vi.Mock;

describe('App shortcuts integration', () => {
  beforeEach(() => {
    worktreeScreenProps.length = 0;
    branchCreatorProps.length = 0;
    prCleanupProps.length = 0;
    navigateToMock.mockClear();
    goBackMock.mockClear();
    resetMock.mockClear();
    hoisted.mockedGetMergedPRWorktrees.mockClear();
    hoisted.mockedGenerateWorktreePath.mockClear();
    hoisted.mockedCreateWorktree.mockClear();
    hoisted.mockedRemoveWorktree.mockClear();
    hoisted.mockedGetRepositoryRoot.mockClear();
    hoisted.mockedDeleteBranch.mockClear();
    mockedUseScreenState.mockReturnValue({
      currentScreen: 'worktree-manager',
      navigateTo: navigateToMock,
      goBack: goBackMock,
      reset: resetMock,
    });
  });

  it('navigates to AI tool selector when worktree is selected', () => {
    const onExit = vi.fn();
    render(<App onExit={onExit} />);

    expect(worktreeScreenProps).not.toHaveLength(0);
    const { onSelect, worktrees } = worktreeScreenProps[0];
    expect(worktrees).toHaveLength(1);

    onSelect(worktrees[0]);

    expect(navigateToMock).toHaveBeenCalledWith('ai-tool-selector');
  });

  it('creates new worktree when branch creator submits', async () => {
    const onExit = vi.fn();

    // Update screen state mock to branch-creator for this test
    mockedUseScreenState.mockReturnValue({
      currentScreen: 'branch-creator',
      navigateTo: navigateToMock,
      goBack: goBackMock,
      reset: resetMock,
    });

    render(<App onExit={onExit} />);

    expect(branchCreatorProps).not.toHaveLength(0);
    const { onCreate } = branchCreatorProps[0];

    await act(async () => {
      await onCreate('feature/new-branch');
    });

    expect(hoisted.mockedCreateWorktree).toHaveBeenCalledWith(
      expect.objectContaining({
        branchName: 'feature/new-branch',
        isNewBranch: true,
      })
    );
    expect(navigateToMock).toHaveBeenCalledWith('ai-tool-selector');
  });

  it('loads cleanup targets when PR cleanup screen is active', async () => {
    const onExit = vi.fn();

    mockedUseScreenState.mockReturnValue({
      currentScreen: 'pr-cleanup',
      navigateTo: navigateToMock,
      goBack: goBackMock,
      reset: resetMock,
    });

    render(<App onExit={onExit} />);

    await act(async () => {
      await Promise.resolve();
    });

    await waitFor(() => {
      expect(prCleanupProps).not.toHaveLength(0);
      const props = prCleanupProps.at(-1);
      expect(props).toBeDefined();
      if (!props) throw new Error('PRCleanupScreen props missing');
      expect(props.targets).toHaveLength(1);
      expect(props.loading).toBe(false);
    });
  });
});
