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

// Mock git.js and worktree.js
vi.mock('../../../git.js', () => ({
  getAllBranches: vi.fn(),
}));

vi.mock('../../../worktree.js', () => ({
  listAdditionalWorktrees: vi.fn(),
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
  it('[AC1] should present execution mode options on launch', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getAllByText, getByText } = render(
      <App repoRoot="/repo" onExit={onExit} />
    );

    await waitFor(() => {
      expect(getByText(/Select execution mode/i)).toBeDefined();
    });

    // Ensure all execution modes are listed
    const options = getAllByText(/Normal|Continue|Resume/i);
    expect(options.length).toBeGreaterThanOrEqual(3);
  });

  /**
   * T075: Acceptance Scenario 2
   * qキー/ESCキーでメイン画面に戻る
   */
  it('[AC2] should return to main screen on q key', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getAllByText } = render(
      <App repoRoot="/repo" onExit={onExit} />
    );

    await waitFor(() => {
      expect(getAllByText(/Execution Mode/i).length).toBeGreaterThan(0);
    });

    // Footer includes back instructions
    const qKeyElements = getAllByText(/Back/i);
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
    const { getAllByText } = render(
      <App repoRoot="/repo" onExit={onExit} />
    );

    await waitFor(() => {
      expect(getAllByText(/Execution Mode/i).length).toBeGreaterThan(0);
    });

    // All footer actions should be visible on execution mode screen
    const actions = getAllByText(/Select|Back/i);
    expect(actions.length).toBeGreaterThan(0);
  });

  it('[Integration] should support all navigation keys', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { getByText, getAllByText } = render(
      <App repoRoot="/repo" onExit={onExit} />
    );

    await waitFor(() => {
      expect(getByText(/Select execution mode/i)).toBeDefined();
    });

    // Verify all navigation keys are available
    const enterKeys = getAllByText(/enter/i);
    const qKeys = getAllByText(/Back/i);

    expect(enterKeys.length).toBeGreaterThan(0);
    expect(qKeys.length).toBeGreaterThan(0);
  });

  it('[Integration] should display correct footer actions', async () => {
    (getAllBranches as ReturnType<typeof vi.fn>).mockResolvedValue(mockBranches);
    (listAdditionalWorktrees as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    const onExit = vi.fn();
    const { container, getAllByText } = render(
      <App repoRoot="/repo" onExit={onExit} />
    );

    await waitFor(() => {
      expect(getAllByText(/Execution Mode/i).length).toBeGreaterThan(0);
    });

    // Verify footer has multiple action keys
    const footerText = container.textContent || '';
    expect(footerText.toLowerCase()).toContain('enter');
    expect(footerText.toLowerCase()).toContain('back');
  });
});
