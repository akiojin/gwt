/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, afterEach, afterAll, vi } from 'vitest';
import { act, render } from '@testing-library/react';
import React from 'react';
import { Window } from 'happy-dom';
import { App } from '../../components/App.js';
import type { BranchInfo, BranchItem } from '../../types.js';
import * as useGitDataModule from '../../hooks/useGitData.js';
import * as useScreenStateModule from '../../hooks/useScreenState.js';
import * as BranchListScreenModule from '../../components/screens/BranchListScreen.js';
import * as BranchActionSelectorScreenModule from '../../components/screens/BranchActionSelectorScreen.js';
import * as worktreeModule from '../../../worktree.js';

const navigateToMock = vi.fn();
const originalUseGitData = useGitDataModule.useGitData;
const originalUseScreenState = useScreenStateModule.useScreenState;
const originalBranchListScreen = BranchListScreenModule.BranchListScreen;
const originalBranchActionSelector = BranchActionSelectorScreenModule.BranchActionSelectorScreen;

const useGitDataSpy = vi.spyOn(useGitDataModule, 'useGitData');
const useScreenStateSpy = vi.spyOn(useScreenStateModule, 'useScreenState');
const branchListScreenSpy = vi.spyOn(BranchListScreenModule, 'BranchListScreen');
const branchActionSelectorSpy = vi.spyOn(BranchActionSelectorScreenModule, 'BranchActionSelectorScreen');
const switchToProtectedBranchSpy = vi.spyOn(worktreeModule, 'switchToProtectedBranch');

const branchListProps: any[] = [];
const branchActionProps: any[] = [];

describe('App protected branch handling', () => {
  beforeEach(() => {
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    navigateToMock.mockReset();
    branchListProps.length = 0;
    branchActionProps.length = 0;
    branchActionProps.length = 0;

    useGitDataSpy.mockReset();
    useScreenStateSpy.mockReset();
    switchToProtectedBranchSpy.mockReset();

    useScreenStateSpy.mockImplementation(() => ({
      currentScreen: 'branch-list',
      navigateTo: navigateToMock,
      goBack: vi.fn(),
      reset: vi.fn(),
    }));

    branchListScreenSpy.mockImplementation((props: any) => {
      branchListProps.push(props);
      return React.createElement(originalBranchListScreen, props);
    });
    branchActionSelectorSpy.mockImplementation((props: any) => {
      branchActionProps.push(props);
      return React.createElement(originalBranchActionSelector, props);
    });
    switchToProtectedBranchSpy.mockResolvedValue('local');
  });

  afterEach(() => {
    useGitDataSpy.mockReset();
    useGitDataSpy.mockImplementation(originalUseGitData);
    useScreenStateSpy.mockReset();
    useScreenStateSpy.mockImplementation(originalUseScreenState);
    branchListScreenSpy.mockImplementation(originalBranchListScreen as any);
    branchActionSelectorSpy.mockImplementation(originalBranchActionSelector as any);
    switchToProtectedBranchSpy.mockReset();
    branchActionProps.length = 0;
  });

  afterAll(() => {
    useGitDataSpy.mockRestore();
    useScreenStateSpy.mockRestore();
    branchListScreenSpy.mockRestore();
    branchActionSelectorSpy.mockRestore();
    switchToProtectedBranchSpy.mockRestore();
  });

  it('shows protected branch warning and switches root without launching AI tool', async () => {
    const branches: BranchInfo[] = [
      {
        name: 'main',
        type: 'local',
        branchType: 'main',
        isCurrent: false,
      },
      {
        name: 'feature/example',
        type: 'local',
        branchType: 'feature',
        isCurrent: true,
      },
    ];

    useGitDataSpy.mockImplementation(() => ({
      branches,
      worktrees: [],
      loading: false,
      error: null,
      refresh: vi.fn(),
      lastUpdated: null,
    }));

    render(<App onExit={vi.fn()} />);

    expect(branchListProps).not.toHaveLength(0);
    const latestProps = branchListProps.at(-1);
    expect(latestProps).toBeDefined();
    if (!latestProps) {
      throw new Error('BranchListScreen props missing');
    }

    const protectedBranch = (latestProps.branches as BranchItem[]).find(
      (item) => item.name === 'main'
    );
    expect(protectedBranch).toBeDefined();
    if (!protectedBranch) {
      throw new Error('Protected branch item not found');
    }

    await act(async () => {
      latestProps.onSelect(protectedBranch);
      await Promise.resolve();
    });

    expect(navigateToMock).toHaveBeenCalledWith('branch-action-selector');
    expect(branchActionProps).not.toHaveLength(0);
    const actionProps = branchActionProps.at(-1);
    expect(actionProps?.mode).toBe('protected');
    expect(actionProps?.infoMessage).toContain('ルートブランチ');

    const nextProps = branchListProps.at(-1);
    expect(nextProps?.cleanupUI?.footerMessage?.text).toContain(
      'ルートブランチはワークツリーを作成せず'
    );
    expect(nextProps?.cleanupUI?.footerMessage?.color).toBe('yellow');

    await act(async () => {
      actionProps?.onUseExisting();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(switchToProtectedBranchSpy).toHaveBeenCalledWith({
      branchName: 'main',
      repoRoot: expect.any(String),
      remoteRef: null,
    });

    const postSwitchProps = branchListProps.at(-1);
    expect(postSwitchProps?.cleanupUI?.footerMessage?.color).toBe('green');
  });
});
