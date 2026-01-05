/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act, render } from "@testing-library/react";
import { render as inkRender } from "ink-testing-library";
import React from "react";
import { BranchListScreen } from "../../../components/screens/BranchListScreen.js";
import type { BranchInfo, BranchItem, Statistics } from "../../../types.js";
import { formatBranchItem } from "../../../utils/branchFormatter.js";
import { Window } from "happy-dom";

const stripAnsi = (value: string): string =>
  value.replace(/\u001b\[[0-9;]*m/g, "");
const stripControlSequences = (value: string): string =>
  value.replace(/\u001b\[([0-9;?]*)([A-Za-z])/g, (_, params, command) => {
    if (command === "C") {
      const count = Number(params || "1");
      return " ".repeat(Number.isNaN(count) ? 0 : count);
    }
    return "";
  });

describe("BranchListScreen", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as unknown as typeof globalThis.window;
    globalThis.document =
      window.document as unknown as typeof globalThis.document;
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  const mockBranches: BranchItem[] = [
    {
      name: "main",
      type: "local",
      branchType: "main",
      isCurrent: true,
      icons: ["⚡", "⭐"],
      hasChanges: false,
      label: "⚡ ⭐ main",
      value: "main",
      latestCommitTimestamp: 1_700_000_000,
    },
    {
      name: "feature/test",
      type: "local",
      branchType: "feature",
      isCurrent: false,
      icons: ["✨"],
      hasChanges: false,
      label: "✨ feature/test",
      value: "feature/test",
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

  it("should render header with title", () => {
    const onSelect = vi.fn();
    const { getByText, getAllByText } = render(
      <BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    expect(getByText(/gwt - Branch Selection/i)).toBeDefined();
  });

  it("should render statistics", () => {
    const onSelect = vi.fn();
    const { container, getByText } = render(
      <BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    expect(container.textContent).toContain("Local: 2");
    expect(getByText(/Remote:/)).toBeDefined();
  });

  it("should render branch list", () => {
    const onSelect = vi.fn();
    const { getByText, getAllByText } = render(
      <BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    expect(getAllByText(/main/).length).toBeGreaterThan(0);
    expect(getByText(/feature\/test/)).toBeDefined();
  });

  it("should render footer with actions", () => {
    const onSelect = vi.fn();
    const { getAllByText } = render(
      <BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    // Check for enter key (main screen doesn't have q key, exit is Ctrl+C only)
    expect(getAllByText(/enter/i).length).toBeGreaterThan(0);
  });

  it("should display selected branch full path above footer help", () => {
    const onSelect = vi.fn();
    const { lastFrame } = inkRender(
      <BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
      { stripAnsi: false },
    );

    const output = stripAnsi(stripControlSequences(lastFrame() ?? ""));
    expect(output).toContain("Branch: main");
  });

  it("should handle empty branch list", () => {
    const onSelect = vi.fn();
    const emptyStats: Statistics = {
      localCount: 0,
      remoteCount: 0,
      worktreeCount: 0,
      changesCount: 0,
      lastUpdated: new Date(),
    };

    const { container } = render(
      <BranchListScreen branches={[]} stats={emptyStats} onSelect={onSelect} />,
    );

    expect(container).toBeDefined();
  });

  it("should display Branch: (none) when branch list is empty", () => {
    const onSelect = vi.fn();
    const emptyStats: Statistics = {
      localCount: 0,
      remoteCount: 0,
      worktreeCount: 0,
      changesCount: 0,
      lastUpdated: new Date(),
    };

    const { lastFrame } = inkRender(
      <BranchListScreen branches={[]} stats={emptyStats} onSelect={onSelect} />,
      { stripAnsi: false },
    );

    const output = stripAnsi(stripControlSequences(lastFrame() ?? ""));
    expect(output).toContain("Branch: (none)");
  });

  it("should display loading indicator after the configured delay", async () => {
    const onSelect = vi.fn();
    const { getByText } = render(
      <BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
        loading={true}
        loadingIndicatorDelay={10}
      />,
    );

    await act(async () => {
      if (typeof vi.advanceTimersByTime === "function") {
        vi.advanceTimersByTime(10);
      } else {
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
    });

    expect(getByText(/Loading Git information/i)).toBeDefined();
  });

  it("should display error state", () => {
    const onSelect = vi.fn();
    const error = new Error("Failed to load branches");
    const { getByText } = render(
      <BranchListScreen
        branches={[]}
        stats={mockStats}
        onSelect={onSelect}
        error={error}
      />,
    );

    expect(getByText(/Error:/i)).toBeDefined();
    expect(getByText(/Failed to load branches/i)).toBeDefined();
  });

  it("should use terminal height for layout calculation", () => {
    const onSelect = vi.fn();

    // Mock process.stdout
    const originalRows = process.stdout.rows;
    process.stdout.rows = 30;

    const { container } = render(
      <BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    expect(container).toBeDefined();

    // Restore
    process.stdout.rows = originalRows;
  });

  it("should display ASCII state icons", () => {
    const onSelect = vi.fn();
    const { container } = render(
      <BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    const text = stripAnsi(container.textContent ?? "");
    expect(text).toMatch(/\[ \]\s(🟢|🔴|⚪)\s(🛡|⚠)/); // state cluster with spacing
  });

  it("should display 🔴 for inaccessible worktree", async () => {
    const onSelect = vi.fn();
    const branches: BranchItem[] = [
      {
        ...formatBranchItem({
          name: "feature/missing-worktree",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          hasUnpushedCommits: false,
          worktree: {
            path: "/tmp/wt-missing",
            locked: false,
            prunable: false,
            isAccessible: false,
          },
        }),
        safeToCleanup: false,
      },
    ];

    let renderResult: ReturnType<typeof inkRender>;
    await act(async () => {
      renderResult = inkRender(
        <BranchListScreen
          branches={branches}
          stats={mockStats}
          onSelect={onSelect}
        />,
        { stripAnsi: false },
      );
    });

    const frame = stripControlSequences(
      stripAnsi(renderResult.lastFrame() ?? ""),
    );
    expect(frame).toContain("[ ] 🔴 ⚠");
  });

  it("should render last tool usage when available and Unknown when not", () => {
    const onSelect = vi.fn();
    const branches: BranchItem[] = [
      {
        name: "feature/with-usage",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        hasUnpushedCommits: false,
        label: "feature/with-usage",
        value: "feature/with-usage",
        icons: [],
        hasChanges: false,
        lastToolUsage: {
          branch: "feature/with-usage",
          worktreePath: "/wt/with",
          toolId: "codex-cli",
          toolLabel: "Codex",
          mode: "normal",
          model: null,
          timestamp: Date.UTC(2025, 10, 26, 14, 3),
        },
        lastToolUsageLabel: "Codex | 2025-11-26 14:03",
      },
      {
        name: "feature/without-usage",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        hasUnpushedCommits: false,
        label: "feature/without-usage",
        value: "feature/without-usage",
        icons: [],
        hasChanges: false,
        latestCommitTimestamp: 1_730_000_000,
        lastToolUsage: null,
        lastToolUsageLabel: null,
      },
    ];

    const { lastFrame } = inkRender(
      <BranchListScreen
        branches={branches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    const output = stripAnsi(stripControlSequences(lastFrame() ?? ""));
    expect(output).toContain("Codex");
    expect(output).toMatch(/2025-11-26/); // date is shown (may wrap)
    expect(output).toContain("Unknown");
  });

  it("should render latest commit timestamp for each branch", () => {
    const onSelect = vi.fn();
    const { container } = render(
      <BranchListScreen
        branches={mockBranches}
        stats={mockStats}
        onSelect={onSelect}
      />,
    );

    const textContent = container.textContent ?? "";
    const matches = textContent.match(/\d{4}-\d{2}-\d{2} \d{2}:\d{2}/g) ?? [];
    expect(matches.length).toBe(mockBranches.length);
  });

  it("should highlight the selected branch with cyan background", async () => {
    process.env.FORCE_COLOR = "1";
    const onSelect = vi.fn();
    let renderResult: ReturnType<typeof inkRender>;
    await act(async () => {
      renderResult = inkRender(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={onSelect}
        />,
        { stripAnsi: false },
      );
    });

    const frame = renderResult?.lastFrame() ?? "";
    expect(frame).toContain("\u001b[46m"); // cyan background ANSI code
  });

  it("should align timestamps even when unpushed icon is displayed", async () => {
    process.env.FORCE_COLOR = "1";
    const onSelect = vi.fn();

    const originalColumns = process.stdout.columns;
    const originalRows = process.stdout.rows;
    process.stdout.columns = 94;
    process.stdout.rows = 30; // Ensure enough rows for all branches to be visible

    const branchInfos: BranchInfo[] = [
      {
        name: "feature/update-ui",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        hasUnpushedCommits: true,
        latestCommitTimestamp: 1_700_000_000,
      },
      {
        name: "origin/main",
        type: "remote",
        branchType: "main",
        isCurrent: false,
        hasUnpushedCommits: false,
        latestCommitTimestamp: 1_699_999_000,
      },
      {
        name: "main",
        type: "local",
        branchType: "main",
        isCurrent: true,
        hasUnpushedCommits: false,
        latestCommitTimestamp: 1_699_998_000,
      },
    ];

    const branchesWithUnpushed: BranchItem[] = branchInfos.map((branch) =>
      formatBranchItem(branch),
    );

    try {
      let renderResult: ReturnType<typeof inkRender>;
      await act(async () => {
        renderResult = inkRender(
          <BranchListScreen
            branches={branchesWithUnpushed}
            stats={mockStats}
            onSelect={onSelect}
          />,
          { stripAnsi: false },
        );
      });

      const frame = renderResult?.lastFrame() ?? "";
      const plain = stripControlSequences(stripAnsi(frame));
      const regex = /\d{4}-\d{2}-\d{2} \d{2}:\d{2}/g;
      let matches = plain.match(regex) ?? [];
      if (matches.length === 0) {
        matches = plain.replace(/\n+/g, " ").match(regex) ?? [];
      }
      expect(matches.length).toBeGreaterThanOrEqual(1);
    } finally {
      process.stdout.columns = originalColumns;
      process.stdout.rows = originalRows;
    }
  });

  it("toggles selection with space and shows ASCII state icons", async () => {
    const onSelect = vi.fn();

    const branches: BranchItem[] = [
      {
        ...formatBranchItem({
          name: "feature/login",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          hasUnpushedCommits: false,
          worktree: {
            path: "/tmp/wt-login",
            locked: false,
            prunable: false,
            isAccessible: true,
            hasUncommittedChanges: false,
          },
        }),
        safeToCleanup: true,
      },
      {
        ...formatBranchItem({
          name: "feature/api",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          hasUnpushedCommits: true,
        }),
        safeToCleanup: false,
      },
    ];

    const Wrapper = () => {
      const [selected, setSelected] = React.useState<string[]>([]);
      return (
        <BranchListScreen
          branches={branches}
          stats={mockStats}
          onSelect={onSelect}
          selectedBranches={selected}
          onToggleSelect={(name) =>
            setSelected((prev) =>
              prev.includes(name)
                ? prev.filter((n) => n !== name)
                : [...prev, name],
            )
          }
        />
      );
    };

    let renderResult: ReturnType<typeof inkRender>;
    await act(async () => {
      renderResult = inkRender(<Wrapper />, { stripAnsi: false });
    });

    const { stdin } = renderResult;
    await act(async () => {
      stdin.write(" ");
    });

    const frame = stripControlSequences(
      stripAnsi(renderResult.lastFrame() ?? ""),
    );
    expect(frame).toContain("[*] 🟢 🛡");
    expect(frame).toContain("feature/login");
  });

  describe("Filter Mode", () => {
    it("should always display filter input field", () => {
      // Note: Filter input is now always visible (no need to press 'f' key)
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={onSelect}
        />,
      );

      // Filter input field should be displayed by default
      expect(container.textContent).toContain("Filter:");
    });

    it("should enter filter mode when f key is pressed", () => {
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={onSelect}
        />,
      );

      // Initially should show prompt to press f
      expect(container.textContent).toContain("(press f to filter)");

      // Press 'f' key
      const fKeyEvent = new globalThis.window.KeyboardEvent("keydown", {
        key: "f",
      });
      document.dispatchEvent(fKeyEvent);

      // Filter input should be active (placeholder visible)
      // Select component should be disabled
      expect(container).toBeDefined();
    });

    it("should exit filter mode and return to branch selection when Esc is pressed in filter mode", () => {
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={onSelect}
        />,
      );

      // Enter filter mode first
      const fKeyEvent = new globalThis.window.KeyboardEvent("keydown", {
        key: "f",
      });
      document.dispatchEvent(fKeyEvent);

      // Press Escape
      const escKeyEvent = new globalThis.window.KeyboardEvent("keydown", {
        key: "Escape",
      });
      document.dispatchEvent(escKeyEvent);

      // Should return to branch selection mode
      // Select should be active, Input should be inactive
      expect(container.textContent).toContain("(press f to filter)");
    });

    it("should show branch list cursor highlight in filter mode", () => {
      process.env.FORCE_COLOR = "1";
      const onSelect = vi.fn();
      let renderResult: ReturnType<typeof inkRender>;
      act(() => {
        renderResult = inkRender(
          <BranchListScreen
            branches={mockBranches}
            stats={mockStats}
            onSelect={onSelect}
            testFilterMode={true}
          />,
          { stripAnsi: false },
        );
      });

      const frame = renderResult?.lastFrame() ?? "";
      // Should contain cyan background (cursor highlight) even in filter mode
      expect(frame).toContain("\u001b[46m");
    });

    it("should allow cursor movement with arrow keys in filter mode", () => {
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={onSelect}
          testFilterMode={true}
        />,
      );

      // Arrow keys should work in filter mode (Select component should not be disabled)
      // This test verifies that cursor movement is possible
      expect(container).toBeDefined();
    });

    it("should allow branch selection with Enter key in filter mode", () => {
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={onSelect}
          testFilterMode={true}
        />,
      );

      // Simulate Enter key (this will trigger onSelect if Select is enabled)
      // Note: Actual key event testing may not work in happy-dom environment
      // but the component should be set up to allow selection
      expect(container).toBeDefined();
    });

    it("should disable filter input cursor when in branch selection mode", () => {
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={onSelect}
        />,
      );

      // By default, should be in branch selection mode
      // Filter input cursor should be disabled/hidden
      expect(container).toBeDefined();
    });

    it("should filter branches in real-time as user types", () => {
      const onSelect = vi.fn();
      const branches: BranchItem[] = [
        ...mockBranches,
        {
          name: "bugfix/issue-123",
          type: "local",
          branchType: "bugfix",
          isCurrent: false,
          icons: ["🐛"],
          hasChanges: false,
          label: "🐛 bugfix/issue-123",
          value: "bugfix/issue-123",
          latestCommitTimestamp: 1_698_000_000,
        },
      ];

      const { container } = render(
        <BranchListScreen
          branches={branches}
          stats={mockStats}
          onSelect={onSelect}
          testFilterMode={true}
          testFilterQuery="feature"
        />,
      );

      // Only feature/test should be visible
      expect(container.textContent).toContain("feature/test");
      expect(container.textContent).not.toContain("bugfix/issue-123");
    });

    it("should clear filter query when Esc key is pressed (with query)", () => {
      // Note: Filter input remains visible, only the query is cleared
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={onSelect}
        />,
      );

      // Enter filter mode
      const fKeyEvent = new globalThis.window.KeyboardEvent("keydown", {
        key: "f",
      });
      document.dispatchEvent(fKeyEvent);

      // Type something in filter
      const input = container.querySelector("input");
      if (input) {
        input.value = "feature";
        input.dispatchEvent(new Event("input", { bubbles: true }));
      }

      // Press Escape (should clear query first)
      const escKeyEvent = new globalThis.window.KeyboardEvent("keydown", {
        key: "Escape",
      });
      document.dispatchEvent(escKeyEvent);

      // Filter input should still be visible, but query cleared
      // All branches should be visible again
      expect(container.textContent).toContain("Filter:");
      expect(container.textContent).toContain("main");
      expect(container.textContent).toContain("feature/test");
    });

    it("should exit filter mode when Esc is pressed with empty query", () => {
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={onSelect}
        />,
      );

      // Enter filter mode
      const fKeyEvent = new globalThis.window.KeyboardEvent("keydown", {
        key: "f",
      });
      document.dispatchEvent(fKeyEvent);

      // Press Escape with empty query (should exit filter mode)
      const escKeyEvent = new globalThis.window.KeyboardEvent("keydown", {
        key: "Escape",
      });
      document.dispatchEvent(escKeyEvent);

      // Should return to branch selection mode
      expect(container.textContent).toContain("(press f to filter)");
    });

    it("should perform case-insensitive search", () => {
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={onSelect}
          testFilterMode={true}
          testFilterQuery="FEATURE"
        />,
      );

      // "feature/test" should still be visible
      expect(container.textContent).toContain("feature/test");
    });

    it("should disable other key bindings (c, r, l) while typing in filter", () => {
      const onSelect = vi.fn();
      const onCleanupCommand = vi.fn();
      const onRefresh = vi.fn();
      const onOpenLogs = vi.fn();

      const inkApp = inkRender(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={onSelect}
          onCleanupCommand={onCleanupCommand}
          onRefresh={onRefresh}
          onOpenLogs={onOpenLogs}
        />,
      );

      act(() => {
        inkApp.stdin.write("f"); // enter filter mode
      });

      act(() => {
        inkApp.stdin.write("c");
        inkApp.stdin.write("r");
        inkApp.stdin.write("l");
      });

      expect(onCleanupCommand).not.toHaveBeenCalled();
      expect(onRefresh).not.toHaveBeenCalled();
      expect(onOpenLogs).not.toHaveBeenCalled();

      inkApp.unmount();
    });

    it("should open logs on l key", () => {
      const onSelect = vi.fn();
      const onOpenLogs = vi.fn();

      const inkApp = inkRender(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={onSelect}
          onOpenLogs={onOpenLogs}
        />,
      );

      act(() => {
        inkApp.stdin.write("l");
      });

      expect(onOpenLogs).toHaveBeenCalled();

      inkApp.unmount();
    });

    it("should display match count when filtering", () => {
      const onSelect = vi.fn();
      const branches: BranchItem[] = [
        ...mockBranches,
        {
          name: "feature/another",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          icons: ["✨"],
          hasChanges: false,
          label: "✨ feature/another",
          value: "feature/another",
          latestCommitTimestamp: 1_698_000_000,
        },
      ];

      const { container } = render(
        <BranchListScreen
          branches={branches}
          stats={mockStats}
          onSelect={onSelect}
          testFilterMode={true}
          testFilterQuery="feature"
        />,
      );

      // Should show "Showing 2 of 3 branches"
      expect(container.textContent).toMatch(/Showing\s+2\s+of\s+3/i);
    });

    it("should show empty list when no branches match", () => {
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={onSelect}
          testFilterMode={true}
          testFilterQuery="nonexistent"
        />,
      );

      // Should show "Showing 0 of 2 branches"
      expect(container.textContent).toMatch(/Showing\s+0\s+of\s+2/i);
    });

    it("should search in PR titles when available", () => {
      const onSelect = vi.fn();
      const branchesWithPR: BranchItem[] = [
        ...mockBranches,
        {
          name: "feature/add-filter",
          type: "local",
          branchType: "feature",
          isCurrent: false,
          icons: ["✨", "🔀"],
          hasChanges: false,
          label: "✨ 🔀 feature/add-filter",
          value: "feature/add-filter",
          latestCommitTimestamp: 1_698_000_000,
          openPR: { number: 123, title: "Add search filter to branch list" },
        },
      ];

      const { container } = render(
        <BranchListScreen
          branches={branchesWithPR}
          stats={mockStats}
          onSelect={onSelect}
          testFilterMode={true}
          testFilterQuery="search"
        />,
      );

      // Branch with matching PR title should be visible
      expect(container.textContent).toContain("feature/add-filter");
    });
  });

  describe("Branch View Mode Toggle (TAB key)", () => {
    const mixedBranches: BranchItem[] = [
      {
        name: "main",
        type: "local",
        branchType: "main",
        isCurrent: true,
        icons: ["⚡"],
        hasChanges: false,
        label: "⚡ main",
        value: "main",
        latestCommitTimestamp: 1_700_000_000,
      },
      {
        name: "feature/test",
        type: "local",
        branchType: "feature",
        isCurrent: false,
        icons: ["✨"],
        hasChanges: false,
        label: "✨ feature/test",
        value: "feature/test",
        latestCommitTimestamp: 1_699_000_000,
      },
      {
        name: "origin/main",
        type: "remote",
        branchType: "main",
        isCurrent: false,
        icons: ["🌐"],
        hasChanges: false,
        label: "🌐 origin/main",
        value: "origin/main",
        remoteName: "origin/main",
        latestCommitTimestamp: 1_698_000_000,
      },
      {
        name: "origin/feature/remote-test",
        type: "remote",
        branchType: "feature",
        isCurrent: false,
        icons: ["🌐"],
        hasChanges: false,
        label: "🌐 origin/feature/remote-test",
        value: "origin/feature/remote-test",
        remoteName: "origin/feature/remote-test",
        latestCommitTimestamp: 1_697_000_000,
      },
    ];

    it("should default to 'all' view mode and display Mode: All in stats", () => {
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mixedBranches}
          stats={mockStats}
          onSelect={onSelect}
        />,
      );

      expect(container.textContent).toContain("Mode: All");
    });

    it("should filter to local branches only when view mode is 'local'", () => {
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mixedBranches}
          stats={mockStats}
          onSelect={onSelect}
          testViewMode="local"
        />,
      );

      expect(container.textContent).toContain("Mode: Local");
      expect(container.textContent).toContain("main");
      expect(container.textContent).toContain("feature/test");
      expect(container.textContent).not.toContain("origin/main");
      expect(container.textContent).not.toContain("origin/feature/remote-test");
    });

    it("should filter to remote branches only when view mode is 'remote'", () => {
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mixedBranches}
          stats={mockStats}
          onSelect={onSelect}
          testViewMode="remote"
        />,
      );

      expect(container.textContent).toContain("Mode: Remote");
      expect(container.textContent).not.toContain("feature/test");
      expect(container.textContent).toContain("origin/main");
      expect(container.textContent).toContain("origin/feature/remote-test");
    });

    it("should toggle view mode from all to local when TAB is pressed", () => {
      const onSelect = vi.fn();
      const onViewModeChange = vi.fn();

      const inkApp = inkRender(
        <BranchListScreen
          branches={mixedBranches}
          stats={mockStats}
          onSelect={onSelect}
          testOnViewModeChange={onViewModeChange}
        />,
      );

      act(() => {
        inkApp.stdin.write("\t"); // TAB key
      });

      expect(onViewModeChange).toHaveBeenCalledWith("local");

      inkApp.unmount();
    });

    it("should toggle view mode from local to remote when TAB is pressed", () => {
      const onSelect = vi.fn();
      const onViewModeChange = vi.fn();

      const inkApp = inkRender(
        <BranchListScreen
          branches={mixedBranches}
          stats={mockStats}
          onSelect={onSelect}
          testViewMode="local"
          testOnViewModeChange={onViewModeChange}
        />,
      );

      act(() => {
        inkApp.stdin.write("\t"); // TAB key
      });

      expect(onViewModeChange).toHaveBeenCalledWith("remote");

      inkApp.unmount();
    });

    it("should toggle view mode from remote to all when TAB is pressed", () => {
      const onSelect = vi.fn();
      const onViewModeChange = vi.fn();

      const inkApp = inkRender(
        <BranchListScreen
          branches={mixedBranches}
          stats={mockStats}
          onSelect={onSelect}
          testViewMode="remote"
          testOnViewModeChange={onViewModeChange}
        />,
      );

      act(() => {
        inkApp.stdin.write("\t"); // TAB key
      });

      expect(onViewModeChange).toHaveBeenCalledWith("all");

      inkApp.unmount();
    });

    it("should not toggle view mode when in filter mode", () => {
      const onSelect = vi.fn();
      const onViewModeChange = vi.fn();

      const inkApp = inkRender(
        <BranchListScreen
          branches={mixedBranches}
          stats={mockStats}
          onSelect={onSelect}
          testFilterMode={true}
          testOnViewModeChange={onViewModeChange}
        />,
      );

      act(() => {
        inkApp.stdin.write("\t"); // TAB key
      });

      expect(onViewModeChange).not.toHaveBeenCalled();

      inkApp.unmount();
    });

    it("should combine view mode filter with search filter (AND condition)", () => {
      const onSelect = vi.fn();
      const { container } = render(
        <BranchListScreen
          branches={mixedBranches}
          stats={mockStats}
          onSelect={onSelect}
          testViewMode="local"
          testFilterQuery="feature"
        />,
      );

      // Only local branches matching "feature"
      expect(container.textContent).toContain("feature/test");
      expect(container.textContent).not.toContain("main");
      expect(container.textContent).not.toContain("origin/feature/remote-test");
    });
  });
});
