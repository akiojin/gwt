/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, render } from '@testing-library/react';
import { render as inkRender } from 'ink-testing-library';
import React from 'react';
import { BranchListScreen } from '../../../components/screens/BranchListScreen.js';
import type { BranchInfo, BranchItem, Statistics } from '../../../types.js';
import { formatBranchItem } from '../../../utils/branchFormatter.js';
import stringWidth from 'string-width';
import { Window } from 'happy-dom';

const stripAnsi = (value: string): string => value.replace(/\u001b\[[0-9;]*m/g, '');
const stripControlSequences = (value: string): string =>
  value.replace(/\u001b\[([0-9;?]*)([A-Za-z])/g, (_, params, command) => {
    if (command === 'C') {
      const count = Number(params || '1');
      return ' '.repeat(Number.isNaN(count) ? 0 : count);
    }
    return '';
  });

describe('BranchListScreen', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  afterEach(() => {
    vi.useRealTimers();
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
      latestCommitTimestamp: 1_700_000_000,
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
      latestCommitTimestamp: 1_699_000_000,
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

    expect(getByText(/gwt/i)).toBeDefined();
  });

  it('should render statistics', () => {
    const onSelect = vi.fn();
    const { container, getByText } = render(
      <BranchListScreen branches={mockBranches} stats={mockStats} onSelect={onSelect} />
    );

    expect(container.textContent).toContain('Local: 2');
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

    // Check for enter key (main screen doesn't have q key, exit is Ctrl+C only)
    expect(getAllByText(/enter/i).length).toBeGreaterThan(0);
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

  it('should display loading indicator after the configured delay', async () => {
    const onSelect = vi.fn();
    const { queryByText, getByText } = render(
      <BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
        loading={true}
        loadingIndicatorDelay={10}
      />
    );

    await act(async () => {
      if (typeof (vi as any).advanceTimersByTime === 'function') {
        (vi as any).advanceTimersByTime(10);
      } else {
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
    });

    expect(getByText(/Loading Git information/i)).toBeDefined();
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

  it('should render latest commit timestamp for each branch', () => {
    const onSelect = vi.fn();
    const { container } = render(
      <BranchListScreen branches={mockBranches} stats={mockStats} onSelect={onSelect} />
    );

    const textContent = container.textContent ?? '';
    const matches = textContent.match(/\d{4}-\d{2}-\d{2} \d{2}:\d{2}/g) ?? [];
    expect(matches.length).toBe(mockBranches.length);
  });

  it('should highlight the selected branch with cyan background', async () => {
    process.env.FORCE_COLOR = '1';
    const onSelect = vi.fn();
    let renderResult: ReturnType<typeof inkRender>;
    await act(async () => {
      renderResult = inkRender(
        <BranchListScreen branches={mockBranches} stats={mockStats} onSelect={onSelect} />,
        { stripAnsi: false }
      );
    });

    const frame = renderResult!.lastFrame() ?? '';
    expect(frame).toContain('\u001b[46m'); // cyan background ANSI code
  });

  it('should align timestamps even when unpushed icon is displayed', async () => {
    process.env.FORCE_COLOR = '1';
    const onSelect = vi.fn();

    const originalColumns = process.stdout.columns;
    process.stdout.columns = 94;

    const branchInfos: BranchInfo[] = [
      {
        name: 'feature/update-ui',
        type: 'local',
        branchType: 'feature',
        isCurrent: false,
        hasUnpushedCommits: true,
        latestCommitTimestamp: 1_700_000_000,
      },
      {
        name: 'origin/main',
        type: 'remote',
        branchType: 'main',
        isCurrent: false,
        hasUnpushedCommits: false,
        latestCommitTimestamp: 1_699_999_000,
      },
      {
        name: 'main',
        type: 'local',
        branchType: 'main',
        isCurrent: true,
        hasUnpushedCommits: false,
        latestCommitTimestamp: 1_699_998_000,
      },
    ];

    const branchesWithUnpushed: BranchItem[] = branchInfos.map((branch) =>
      formatBranchItem(branch)
    );

    try {
      let renderResult: ReturnType<typeof inkRender>;
      await act(async () => {
        renderResult = inkRender(
          <BranchListScreen branches={branchesWithUnpushed} stats={mockStats} onSelect={onSelect} />,
          { stripAnsi: false }
        );
      });

      const frame = renderResult!.lastFrame() ?? '';
      const timestampLines = frame
        .split('\n')
        .map((line) => stripControlSequences(stripAnsi(line)))
        .filter((line) => /\d{4}-\d{2}-\d{2} \d{2}:\d{2}/.test(line));

      expect(timestampLines.length).toBeGreaterThanOrEqual(3);

      const timestampWidths = timestampLines.map((line) => {
        const match = line.match(/\d{4}-\d{2}-\d{2} \d{2}:\d{2}/);
        const index = match?.index ?? 0;
        const beforeTimestamp = line.slice(0, index);

        let width = 0;
        for (const char of Array.from(beforeTimestamp)) {
          if (char === '\u2B06' || char === '\u2601') {
            width += 1;
            continue;
          }
          width += stringWidth(char);
        }
        return width;
      });

      const uniquePositions = new Set(timestampWidths);

      expect(uniquePositions.size).toBe(1);
    } finally {
      process.stdout.columns = originalColumns;
    }
  });
});
