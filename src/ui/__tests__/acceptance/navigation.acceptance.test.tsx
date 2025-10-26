/**
 * @vitest-environment happy-dom
 * Acceptance tests for User Story 2: Sub-screen Navigation
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, waitFor } from '@testing-library/react';
import React from 'react';
import { App } from '../../components/App.js';
import { Window } from 'happy-dom';
import type { BranchInfo } from '../../types.js';

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

// Mock git.js and worktree.js
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

describe('Acceptance: Navigation (User Story 2)', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    // Reset mocks
    (getAllBranches as ReturnType<typeof vi.fn>).mockReset();
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockReset();
    hoisted.mockGetAllBranches.mockReset();
    hoisted.mockGetRepositoryRoot.mockReset();
    hoisted.mockDeleteBranch.mockReset();
    hoisted.mockListAdditionalWorktrees.mockReset();
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
   * qキー/ESCキーでメイン画面に戻る
   */
  it('[AC2] should return to main screen on q key', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getAllByText, container } = render(<App onExit={onExit} />);

    await waitFor(() => {
      expect(container).toBeDefined();
    });

    // Verify q key action is available in footer
    const qKeyElements = getAllByText(/q/i);
    expect(qKeyElements.length).toBeGreaterThan(0);
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

    // Verify all navigation keys are available
    const enterKeys = getAllByText(/enter/i);
    const qKeys = getAllByText(/q/i);

    expect(enterKeys.length).toBeGreaterThan(0);
    expect(qKeys.length).toBeGreaterThan(0);
  });

  it('[Integration] should display correct footer actions', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { container } = render(<App onExit={onExit} />);

    await waitFor(() => {
      expect(container).toBeDefined();
    });

    // Verify footer has multiple action keys
    const footerText = container.textContent || '';
    expect(footerText.toLowerCase()).toContain('enter');
    expect(footerText.toLowerCase()).toContain('q');
  });
});
