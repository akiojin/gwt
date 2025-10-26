/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render } from '@testing-library/react';
import React from 'react';
import { BranchListScreen } from '../../../components/screens/BranchListScreen.js';
import type { BranchItem, Statistics } from '../../../types.js';
import { Window } from 'happy-dom';

describe('BranchListScreen', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  const mockBranches: BranchItem[] = [
    {
      name: 'main',
      type: 'local',
      branchType: 'main',
      isCurrent: true,
      icons: ['⚡', '⭐'],
      hasChanges: false,
      label: '⚡ ⭐ main',
      value: 'main',
    },
    {
      name: 'feature/test',
      type: 'local',
      branchType: 'feature',
      isCurrent: false,
      icons: ['✨'],
      hasChanges: false,
      label: '✨ feature/test',
      value: 'feature/test',
    },
  ];

  const mockStats: Statistics = {
    localCount: 2,
    remoteCount: 1,
    worktreeCount: 0,
    changesCount: 0,
    lastUpdated: new Date(),
  };

  it('should render header with title', () => {
    const onSelect = vi.fn();
    const { getByText } = render(
      <BranchListScreen branches={mockBranches} stats={mockStats} onSelect={onSelect} />
    );

    expect(getByText(/Claude Worktree/i)).toBeDefined();
  });

  it('should render statistics', () => {
    const onSelect = vi.fn();
    const { getByText } = render(
      <BranchListScreen branches={mockBranches} stats={mockStats} onSelect={onSelect} />
    );

    expect(getByText(/Local:/)).toBeDefined();
    expect(getByText(/2/)).toBeDefined();
    expect(getByText(/Remote:/)).toBeDefined();
  });

  it('should render branch list', () => {
    const onSelect = vi.fn();
    const { getByText } = render(
      <BranchListScreen branches={mockBranches} stats={mockStats} onSelect={onSelect} />
    );

    expect(getByText(/main/)).toBeDefined();
    expect(getByText(/feature\/test/)).toBeDefined();
  });

  it('should render footer with actions', () => {
    const onSelect = vi.fn();
    const { getAllByText } = render(
      <BranchListScreen branches={mockBranches} stats={mockStats} onSelect={onSelect} />
    );

    // Check for enter and q keys
    expect(getAllByText(/enter/i).length).toBeGreaterThan(0);
    expect(getAllByText(/q/i).length).toBeGreaterThan(0);
    expect(getAllByText(/Quit/i).length).toBeGreaterThan(0);
  });

  it('should handle empty branch list', () => {
    const onSelect = vi.fn();
    const emptyStats: Statistics = {
      localCount: 0,
      remoteCount: 0,
      worktreeCount: 0,
      changesCount: 0,
      lastUpdated: new Date(),
    };

    const { container } = render(
      <BranchListScreen branches={[]} stats={emptyStats} onSelect={onSelect} />
    );

    expect(container).toBeDefined();
  });

  it('should display loading state', () => {
    const onSelect = vi.fn();
    const { getByText } = render(
      <BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
        loading={true}
      />
    );

    expect(getByText(/Loading/i)).toBeDefined();
  });

  it('should display error state', () => {
    const onSelect = vi.fn();
    const error = new Error('Failed to load branches');
    const { getByText } = render(
      <BranchListScreen branches={[]} stats={mockStats} onSelect={onSelect} error={error} />
    );

    expect(getByText(/Error:/i)).toBeDefined();
    expect(getByText(/Failed to load branches/i)).toBeDefined();
  });

  it('should use terminal height for layout calculation', () => {
    const onSelect = vi.fn();

    // Mock process.stdout
    const originalRows = process.stdout.rows;
    process.stdout.rows = 30;

    const { container } = render(
      <BranchListScreen branches={mockBranches} stats={mockStats} onSelect={onSelect} />
    );

    expect(container).toBeDefined();

    // Restore
    process.stdout.rows = originalRows;
  });

  it('should display branch icons', () => {
    const onSelect = vi.fn();
    const { getByText } = render(
      <BranchListScreen branches={mockBranches} stats={mockStats} onSelect={onSelect} />
    );

    // Check for icons in labels
    expect(getByText(/⚡/)).toBeDefined(); // main icon
    expect(getByText(/⭐/)).toBeDefined(); // current icon
    expect(getByText(/✨/)).toBeDefined(); // feature icon
  });
});
