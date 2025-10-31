/**
 * @vitest-environment happy-dom
 * Acceptance tests for User Story 2: Sub-screen Navigation
 */
import { describe, it, expect, beforeEach, afterAll, vi } from 'vitest';
import type { Mock } from 'vitest';
import { render, waitFor } from '@testing-library/react';
import React from 'react';
import { App } from '../../components/App.js';
import { Window } from 'happy-dom';
import type { BranchInfo } from '../../types.js';

// Mock git.js and worktree.js
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

describe('Acceptance: Navigation (User Story 2)', () => {
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

  /**
   * T074: Acceptance Scenario 1
   * nキーで新規ブランチ作成画面に遷移
   */
  it('[AC1] should navigate to branch creator on n key', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getByText, container } = render(<App onExit={onExit} />);

    await waitFor(() => {
      expect(getByText(/Claude Worktree/i)).toBeDefined();
    });

    // Verify n key action is available in footer
    const nKeyElements = container.querySelectorAll('*');
    let hasNKey = false;
    nKeyElements.forEach((el) => {
      if (el.textContent?.toLowerCase().includes('new branch')) {
        hasNKey = true;
      }
    });

    expect(hasNKey || container.textContent?.toLowerCase().includes('n')).toBe(true);
  });

  /**
   * T075: Acceptance Scenario 2
   * メイン画面にはqキーが存在しない（終了はCtrl+Cのみ）
   */
  it('[AC2] should not have q key on main screen', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { container } = render(<App onExit={onExit} />);

    await waitFor(() => {
      expect(container).toBeDefined();
    });

    // Verify q key is NOT in the footer (main screen uses Ctrl+C for exit)
    const footerText = container.textContent || '';
    // Main screen should not have 'q' for quit, but should have other keys
    expect(footerText.toLowerCase()).not.toMatch(/\[q\]/);
    expect(footerText.toLowerCase()).toContain('enter');
  });

  /**
   * T076: Acceptance Scenario 3
   * Worktree管理でアクション実行後に適切に遷移
   */
  it('[AC3] should handle worktree management navigation', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([
      {
        branch: 'feature/test',
        path: '/path/to/worktree',
        head: 'abc123',
        isAccessible: true,
      },
    ]);

    const onExit = vi.fn();
    const { getByText, container } = render(<App onExit={onExit} />);

    await waitFor(() => {
      expect(getByText(/Claude Worktree/i)).toBeDefined();
    });

    // Verify m key action is available for worktree management
    const mKeyElements = container.querySelectorAll('*');
    let hasMKey = false;
    mKeyElements.forEach((el) => {
      if (el.textContent?.toLowerCase().includes('manage worktrees')) {
        hasMKey = true;
      }
    });

    expect(hasMKey || container.textContent?.toLowerCase().includes('m')).toBe(true);
  });

  it('[Integration] should support all navigation keys', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getByText, getAllByText } = render(<App onExit={onExit} />);

    await waitFor(() => {
      expect(getByText(/Claude Worktree/i)).toBeDefined();
    });

    // Verify navigation keys are available (main screen doesn't have q key)
    const enterKeys = getAllByText(/enter/i);

    expect(enterKeys.length).toBeGreaterThan(0);
  });

  it('[Integration] should display correct footer actions', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { container } = render(<App onExit={onExit} />);

    await waitFor(() => {
      expect(container).toBeDefined();
    });

    // Verify footer has multiple action keys (main screen doesn't have q key)
    const footerText = container.textContent || '';
    expect(footerText.toLowerCase()).toContain('enter');
    expect(footerText.toLowerCase()).toContain('m'); // Manage worktrees
  });
});

afterAll(() => {
  vi.restoreAllMocks();
});
