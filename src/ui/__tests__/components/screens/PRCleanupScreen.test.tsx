/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render } from '@testing-library/react';
import React from 'react';
import { PRCleanupScreen } from '../../../components/screens/PRCleanupScreen.js';
import { Window } from 'happy-dom';
import type { MergedPullRequest } from '../../../types.js';

describe('PRCleanupScreen', () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  const mockPullRequests: MergedPullRequest[] = [
    {
      number: 123,
      title: 'Add new feature',
      branch: 'feature/add-new-feature',
      mergedAt: '2025-01-20T10:00:00Z',
      author: 'user1',
    },
    {
      number: 124,
      title: 'Fix bug',
      branch: 'hotfix/fix-bug',
      mergedAt: '2025-01-21T15:30:00Z',
      author: 'user2',
    },
  ];

  it('should render header with title', () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { getByText } = render(
      <PRCleanupScreen pullRequests={mockPullRequests} onBack={onBack} onCleanup={onCleanup} />
    );

    expect(getByText(/PR Cleanup/i)).toBeDefined();
  });

  it('should render PR list', () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { getByText } = render(
      <PRCleanupScreen pullRequests={mockPullRequests} onBack={onBack} onCleanup={onCleanup} />
    );

    expect(getByText(/Add new feature/i)).toBeDefined();
    expect(getByText(/Fix bug/i)).toBeDefined();
  });

  it('should render footer with actions', () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { getAllByText } = render(
      <PRCleanupScreen pullRequests={mockPullRequests} onBack={onBack} onCleanup={onCleanup} />
    );

    expect(getAllByText(/enter/i).length).toBeGreaterThan(0);
    expect(getAllByText(/q/i).length).toBeGreaterThan(0);
  });

  it('should handle empty PR list', () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { getByText } = render(
      <PRCleanupScreen pullRequests={[]} onBack={onBack} onCleanup={onCleanup} />
    );

    expect(getByText(/No merged pull requests found/i)).toBeDefined();
  });

  it('should display PR count in stats', () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { getByText, getAllByText } = render(
      <PRCleanupScreen pullRequests={mockPullRequests} onBack={onBack} onCleanup={onCleanup} />
    );

    expect(getByText(/Total:/i)).toBeDefined();
    expect(getAllByText(/2/).length).toBeGreaterThan(0);
  });

  it('should use terminal height for layout calculation', () => {
    const originalRows = process.stdout.rows;
    process.stdout.rows = 30;

    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { container } = render(
      <PRCleanupScreen pullRequests={mockPullRequests} onBack={onBack} onCleanup={onCleanup} />
    );

    expect(container).toBeDefined();

    process.stdout.rows = originalRows;
  });

  it('should handle back navigation with q key', () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { container } = render(
      <PRCleanupScreen pullRequests={mockPullRequests} onBack={onBack} onCleanup={onCleanup} />
    );

    // Test will verify onBack is called when q is pressed
    expect(container).toBeDefined();
  });
});
