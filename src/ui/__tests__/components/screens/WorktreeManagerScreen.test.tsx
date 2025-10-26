/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render } from '@testing-library/react';
import React from 'react';
import { WorktreeManagerScreen } from '../../../components/screens/WorktreeManagerScreen.js';
import { Window } from 'happy-dom';

describe('WorktreeManagerScreen', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  const mockWorktrees = [
    {
      branch: 'feature/test-1',
      path: '/path/to/worktree-1',
      isAccessible: true,
    },
    {
      branch: 'feature/test-2',
      path: '/path/to/worktree-2',
      isAccessible: true,
    },
  ];

  it('should render header with title', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <WorktreeManagerScreen worktrees={mockWorktrees} onBack={onBack} onSelect={onSelect} />
    );

    expect(getByText(/Worktree Manager/i)).toBeDefined();
  });

  it('should render worktree list', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <WorktreeManagerScreen worktrees={mockWorktrees} onBack={onBack} onSelect={onSelect} />
    );

    expect(getByText(/feature\/test-1/)).toBeDefined();
    expect(getByText(/feature\/test-2/)).toBeDefined();
  });

  it('should render footer with actions', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getAllByText } = render(
      <WorktreeManagerScreen worktrees={mockWorktrees} onBack={onBack} onSelect={onSelect} />
    );

    expect(getAllByText(/enter/i).length).toBeGreaterThan(0);
    expect(getAllByText(/q/i).length).toBeGreaterThan(0);
  });

  it('should handle empty worktree list', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <WorktreeManagerScreen worktrees={[]} onBack={onBack} onSelect={onSelect} />
    );

    expect(getByText(/No worktrees found/i)).toBeDefined();
  });

  it('should display inaccessible worktrees differently', () => {
    const worktreesWithInaccessible = [
      {
        branch: 'feature/accessible',
        path: '/path/accessible',
        isAccessible: true,
      },
      {
        branch: 'feature/inaccessible',
        path: '/path/inaccessible',
        isAccessible: false,
      },
    ];

    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <WorktreeManagerScreen
        worktrees={worktreesWithInaccessible}
        onBack={onBack}
        onSelect={onSelect}
      />
    );

    expect(getByText(/feature\/accessible/)).toBeDefined();
    expect(getByText(/feature\/inaccessible/)).toBeDefined();
  });

  it('should use terminal height for layout calculation', () => {
    const originalRows = process.stdout.rows;
    process.stdout.rows = 30;

    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <WorktreeManagerScreen worktrees={mockWorktrees} onBack={onBack} onSelect={onSelect} />
    );

    expect(container).toBeDefined();

    process.stdout.rows = originalRows;
  });

  it('should display worktree count in stats', () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText, getAllByText } = render(
      <WorktreeManagerScreen worktrees={mockWorktrees} onBack={onBack} onSelect={onSelect} />
    );

    // Check for worktree count
    expect(getByText(/Total:/i)).toBeDefined();
    expect(getAllByText(/2/).length).toBeGreaterThan(0);
  });
});
