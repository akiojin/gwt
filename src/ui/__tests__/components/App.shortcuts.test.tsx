/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, afterEach, afterAll, vi } from 'vitest';
import type { Mock } from 'vitest';
import { render, act, waitFor } from '@testing-library/react';
import React from 'react';
import type { CleanupTarget } from '../../types.js';
import { Window } from 'happy-dom';
import * as useGitDataModule from '../../hooks/useGitData.js';
import * as useScreenStateModule from '../../hooks/useScreenState.js';
import * as WorktreeManagerScreenModule from '../../components/screens/WorktreeManagerScreen.js';
import * as BranchCreatorScreenModule from '../../components/screens/BranchCreatorScreen.js';
import * as PRCleanupScreenModule from '../../components/screens/PRCleanupScreen.js';
import * as worktreeModule from '../../../worktree.js';
import * as gitModule from '../../../git.js';
import { App } from '../../components/App.js';

const navigateToMock = vi.fn();
const goBackMock = vi.fn();
const resetMock = vi.fn();

const worktreeScreenProps: any[] = [];
const branchCreatorProps: any[] = [];
const prCleanupProps: any[] = [];

const originalUseGitData = useGitDataModule.useGitData;
const originalUseScreenState = useScreenStateModule.useScreenState;
const originalWorktreeManagerScreen = WorktreeManagerScreenModule.WorktreeManagerScreen;
const originalBranchCreatorScreen = BranchCreatorScreenModule.BranchCreatorScreen;
const originalPRCleanupScreen = PRCleanupScreenModule.PRCleanupScreen;
const originalGetMergedPRWorktrees = worktreeModule.getMergedPRWorktrees;
const originalGenerateWorktreePath = worktreeModule.generateWorktreePath;
const originalCreateWorktree = worktreeModule.createWorktree;
const originalRemoveWorktree = worktreeModule.removeWorktree;
const originalGetRepositoryRoot = gitModule.getRepositoryRoot;
const originalDeleteBranch = gitModule.deleteBranch;

const useGitDataSpy = vi.spyOn(useGitDataModule, 'useGitData');
const useScreenStateSpy = vi.spyOn(useScreenStateModule, 'useScreenState');
const worktreeManagerScreenSpy = vi.spyOn(WorktreeManagerScreenModule, 'WorktreeManagerScreen');
const branchCreatorScreenSpy = vi.spyOn(BranchCreatorScreenModule, 'BranchCreatorScreen');
const prCleanupScreenSpy = vi.spyOn(PRCleanupScreenModule, 'PRCleanupScreen');
const getMergedPRWorktreesSpy = vi.spyOn(worktreeModule, 'getMergedPRWorktrees');
const generateWorktreePathSpy = vi.spyOn(worktreeModule, 'generateWorktreePath');
const createWorktreeSpy = vi.spyOn(worktreeModule, 'createWorktree');
const removeWorktreeSpy = vi.spyOn(worktreeModule, 'removeWorktree');
const getRepositoryRootSpy = vi.spyOn(gitModule, 'getRepositoryRoot');
const deleteBranchSpy = vi.spyOn(gitModule, 'deleteBranch');

describe('App shortcuts integration', () => {
  beforeEach(() => {
    if (typeof globalThis.document === 'undefined') {
      const window = new Window();
      globalThis.window = window as any;
      globalThis.document = window.document as any;
    }
    worktreeScreenProps.length = 0;
    branchCreatorProps.length = 0;
    prCleanupProps.length = 0;
    navigateToMock.mockClear();
    goBackMock.mockClear();
    resetMock.mockClear();
    useGitDataSpy.mockImplementation(() => ({
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
    }));
    useScreenStateSpy.mockImplementation(() => ({
      currentScreen: 'worktree-manager',
      navigateTo: navigateToMock,
      goBack: goBackMock,
      reset: resetMock,
    }));
    worktreeManagerScreenSpy.mockImplementation((props: any) => {
      worktreeScreenProps.push(props);
      return React.createElement(originalWorktreeManagerScreen, props);
    });
    branchCreatorScreenSpy.mockImplementation((props: any) => {
      branchCreatorProps.push(props);
      return React.createElement(originalBranchCreatorScreen, props);
    });
    prCleanupScreenSpy.mockImplementation((props: any) => {
      prCleanupProps.push(props);
      return React.createElement(originalPRCleanupScreen, props);
    });
    getMergedPRWorktreesSpy.mockResolvedValue([
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
    ] as CleanupTarget[]);
    generateWorktreePathSpy.mockResolvedValue('/worktrees/new-branch');
    createWorktreeSpy.mockResolvedValue(undefined);
    removeWorktreeSpy.mockResolvedValue(undefined);
    getRepositoryRootSpy.mockResolvedValue('/repo');
    deleteBranchSpy.mockResolvedValue(undefined);
  });

  afterEach(() => {
    useGitDataSpy.mockReset();
    useScreenStateSpy.mockReset();
    worktreeManagerScreenSpy.mockReset();
    branchCreatorScreenSpy.mockReset();
    prCleanupScreenSpy.mockReset();
    getMergedPRWorktreesSpy.mockReset();
    generateWorktreePathSpy.mockReset();
    createWorktreeSpy.mockReset();
    removeWorktreeSpy.mockReset();
    getRepositoryRootSpy.mockReset();
    deleteBranchSpy.mockReset();
    useGitDataSpy.mockImplementation(originalUseGitData);
    useScreenStateSpy.mockImplementation(originalUseScreenState);
    worktreeManagerScreenSpy.mockImplementation(originalWorktreeManagerScreen as any);
    branchCreatorScreenSpy.mockImplementation(originalBranchCreatorScreen as any);
    prCleanupScreenSpy.mockImplementation(originalPRCleanupScreen as any);
    getMergedPRWorktreesSpy.mockImplementation(originalGetMergedPRWorktrees as any);
    generateWorktreePathSpy.mockImplementation(originalGenerateWorktreePath as any);
    createWorktreeSpy.mockImplementation(originalCreateWorktree as any);
    removeWorktreeSpy.mockImplementation(originalRemoveWorktree as any);
    getRepositoryRootSpy.mockImplementation(originalGetRepositoryRoot as any);
    deleteBranchSpy.mockImplementation(originalDeleteBranch as any);
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
    useScreenStateSpy.mockReturnValue({
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

    expect(createWorktreeSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        branchName: 'feature/new-branch',
        isNewBranch: true,
      })
    );
    expect(navigateToMock).toHaveBeenCalledWith('ai-tool-selector');
  });

  it('loads cleanup targets when PR cleanup screen is active', async () => {
    const onExit = vi.fn();

    useScreenStateSpy.mockReturnValue({
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

afterAll(() => {
  useGitDataSpy.mockRestore();
  useScreenStateSpy.mockRestore();
  worktreeManagerScreenSpy.mockRestore();
  branchCreatorScreenSpy.mockRestore();
  prCleanupScreenSpy.mockRestore();
  getMergedPRWorktreesSpy.mockRestore();
  generateWorktreePathSpy.mockRestore();
  createWorktreeSpy.mockRestore();
  removeWorktreeSpy.mockRestore();
  getRepositoryRootSpy.mockRestore();
  deleteBranchSpy.mockRestore();
});
