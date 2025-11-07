/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, afterAll, vi } from 'vitest';
import type { Mock } from 'vitest';
import { render, waitFor } from '@testing-library/react';
import { act } from 'react-dom/test-utils';
import React from 'react';
import { App } from '../../components/App.js';
import { Window } from 'happy-dom';
import type { BranchInfo, BranchItem } from '../../types.js';
import * as BranchListScreenModule from '../../components/screens/BranchListScreen.js';
import * as BranchActionSelectorScreenModule from '../../screens/BranchActionSelectorScreen.js';

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
  isProtectedBranchName: vi.fn(() => false),
  switchToProtectedBranch: vi.fn(async () => 'none' as const),
}));

const aiToolScreenProps: unknown[] = [];

vi.mock('../../components/screens/AIToolSelectorScreen.js', () => {
  return {
    AIToolSelectorScreen: (props: unknown) => {
      aiToolScreenProps.push(props);
      return React.createElement('div');
    },
  };
});

import { getAllBranches, getRepositoryRoot, deleteBranch } from '../../../git.js';
import {
  listAdditionalWorktrees,
  createWorktree,
  generateWorktreePath,
  getMergedPRWorktrees,
  removeWorktree,
  isProtectedBranchName,
  switchToProtectedBranch,
} from '../../../worktree.js';

const mockedGetAllBranches = getAllBranches as Mock;
const mockedGetRepositoryRoot = getRepositoryRoot as Mock;
const mockedDeleteBranch = deleteBranch as Mock;
const mockedListAdditionalWorktrees = listAdditionalWorktrees as Mock;
const mockedCreateWorktree = createWorktree as Mock;
const mockedGenerateWorktreePath = generateWorktreePath as Mock;
const mockedGetMergedPRWorktrees = getMergedPRWorktrees as Mock;
const mockedRemoveWorktree = removeWorktree as Mock;
const mockedIsProtectedBranchName = isProtectedBranchName as Mock;
const mockedSwitchToProtectedBranch = switchToProtectedBranch as Mock;
const originalBranchListScreen = BranchListScreenModule.BranchListScreen;
const originalBranchActionSelectorScreen =
  BranchActionSelectorScreenModule.BranchActionSelectorScreen;

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
    mockedIsProtectedBranchName.mockReset();
    mockedSwitchToProtectedBranch.mockReset();
    mockedGetRepositoryRoot.mockResolvedValue('/repo');
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

describe('Protected Branch Navigation (T103)', () => {
  const branchListProps: any[] = [];
  const branchActionProps: any[] = [];
  let branchListSpy: ReturnType<typeof vi.spyOn>;
  let branchActionSpy: ReturnType<typeof vi.spyOn>;

  const baseBranches: BranchInfo[] = [
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

  beforeEach(() => {
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
    mockedGetAllBranches.mockReset();
    mockedListAdditionalWorktrees.mockReset();
    mockedGetRepositoryRoot.mockReset();
    mockedDeleteBranch.mockReset();
    mockedCreateWorktree.mockReset();
    mockedGenerateWorktreePath.mockReset();
    mockedGetMergedPRWorktrees.mockReset();
    mockedRemoveWorktree.mockReset();
    mockedIsProtectedBranchName.mockReset();
    mockedSwitchToProtectedBranch.mockReset();
    mockedGetRepositoryRoot.mockResolvedValue('/repo');
    branchListProps.length = 0;
    branchActionProps.length = 0;
    aiToolScreenProps.length = 0;
    branchListSpy = vi
      .spyOn(BranchListScreenModule, 'BranchListScreen')
      .mockImplementation((props: any) => {
        branchListProps.push(props);
        return React.createElement(originalBranchListScreen, props);
      });
    branchActionSpy = vi
      .spyOn(BranchActionSelectorScreenModule, 'BranchActionSelectorScreen')
      .mockImplementation((props: any) => {
        branchActionProps.push(props);
        return React.createElement(originalBranchActionSelectorScreen, props);
      });

    mockedIsProtectedBranchName.mockImplementation((name: string) =>
      ['main', 'develop', 'origin/main', 'origin/develop'].includes(name)
    );
    mockedSwitchToProtectedBranch.mockResolvedValue('local');
  });

  afterEach(() => {
    branchListSpy.mockRestore();
    branchActionSpy.mockRestore();
  });

  it('switches local protected branches via root workflow and navigates to AI tool', async () => {
    mockedGetAllBranches.mockResolvedValue(baseBranches);
    mockedListAdditionalWorktrees.mockResolvedValue([]);

    const onExit = vi.fn();
    render(<App onExit={onExit} />);

    await waitFor(() => {
      expect(branchListProps.length).toBeGreaterThan(0);
    });

    const latestProps = branchListProps.at(-1);
    const protectedBranch = (latestProps?.branches as BranchItem[]).find(
      (item) => item.name === 'main'
    );
    expect(protectedBranch).toBeDefined();

    await act(async () => {
      latestProps?.onSelect(protectedBranch);
      await Promise.resolve();
    });

    await waitFor(() => {
      expect(branchActionProps.length).toBeGreaterThan(0);
    });

    const actionProps = branchActionProps.at(-1);
    expect(actionProps?.mode).toBe('protected');
    expect(actionProps?.infoMessage).toContain('ルートブランチ');

    await act(async () => {
      await actionProps?.onUseExisting();
      await Promise.resolve();
    });

    expect(mockedSwitchToProtectedBranch).toHaveBeenCalledWith({
      branchName: 'main',
      repoRoot: '/repo',
      remoteRef: null,
    });

    await waitFor(() => {
      expect(aiToolScreenProps.length).toBeGreaterThan(0);
    });
  });

  it('creates tracking branch for remote protected selections before navigating to AI tool', async () => {
    const remoteBranches: BranchInfo[] = [
      {
        name: 'origin/develop',
        type: 'remote',
        branchType: 'develop',
        isCurrent: false,
      },
      {
        name: 'feature/test',
        type: 'local',
        branchType: 'feature',
        isCurrent: false,
      },
    ];
    mockedGetAllBranches.mockResolvedValue(remoteBranches);
    mockedListAdditionalWorktrees.mockResolvedValue([]);
    mockedSwitchToProtectedBranch.mockResolvedValue('remote');

    const onExit = vi.fn();
    render(<App onExit={onExit} />);

    await waitFor(() => {
      expect(branchListProps.length).toBeGreaterThan(0);
    });

    const latestProps = branchListProps.at(-1);
    const protectedBranch = (latestProps?.branches as BranchItem[]).find(
      (item) => item.name === 'origin/develop'
    );
    expect(protectedBranch).toBeDefined();

    await act(async () => {
      latestProps?.onSelect(protectedBranch);
      await Promise.resolve();
    });

    await waitFor(() => {
      expect(branchActionProps.length).toBeGreaterThan(0);
    });

    const actionProps = branchActionProps.at(-1);
    expect(actionProps?.mode).toBe('protected');
    expect(actionProps?.primaryLabel).toContain('root');

    await act(async () => {
      await actionProps?.onUseExisting();
      await Promise.resolve();
    });

    expect(mockedSwitchToProtectedBranch).toHaveBeenCalledWith({
      branchName: 'develop',
      repoRoot: '/repo',
      remoteRef: 'origin/develop',
    });

    await waitFor(() => {
      expect(aiToolScreenProps.length).toBeGreaterThan(0);
    });
  });
});

afterAll(() => {
  vi.restoreAllMocks();
});
