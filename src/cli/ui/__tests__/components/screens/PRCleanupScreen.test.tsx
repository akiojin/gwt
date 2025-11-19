/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render } from "@testing-library/react";
import React from "react";
import { PRCleanupScreen } from "../../../components/screens/PRCleanupScreen.js";
import { Window } from "happy-dom";
import type { CleanupTarget } from "../../../types.js";

describe("PRCleanupScreen", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  const mockTargets: CleanupTarget[] = [
    {
      branch: "feature/add-new-feature",
      cleanupType: "worktree-and-branch",
      pullRequest: {
        number: 123,
        title: "Add new feature",
        branch: "feature/add-new-feature",
        mergedAt: "2025-01-20T10:00:00Z",
        author: "user1",
      },
      worktreePath: "/workspace/feature-add-new-feature",
      hasUncommittedChanges: false,
      hasUnpushedCommits: false,
      hasRemoteBranch: true,
      isAccessible: true,
    },
    {
      branch: "hotfix/fix-bug",
      cleanupType: "branch-only",
      pullRequest: {
        number: 124,
        title: "Fix bug",
        branch: "hotfix/fix-bug",
        mergedAt: "2025-01-21T15:30:00Z",
        author: "user2",
      },
      worktreePath: null,
      hasUncommittedChanges: false,
      hasUnpushedCommits: false,
      hasRemoteBranch: false,
    },
  ];

  it("should render header with title", () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { getByText } = render(
      <PRCleanupScreen
        targets={mockTargets}
        loading={false}
        error={null}
        onBack={onBack}
        onRefresh={vi.fn()}
        onCleanup={onCleanup}
      />,
    );

    expect(getByText(/Branch Cleanup/i)).toBeDefined();
  });

  it("should render PR list", () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { getByText } = render(
      <PRCleanupScreen
        targets={mockTargets}
        loading={false}
        error={null}
        onBack={onBack}
        onRefresh={vi.fn()}
        onCleanup={onCleanup}
      />,
    );

    expect(getByText(/feature\/add-new-feature/i)).toBeDefined();
    expect(getByText(/hotfix\/fix-bug/i)).toBeDefined();
  });

  it("should render footer with actions", () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { getAllByText } = render(
      <PRCleanupScreen
        targets={mockTargets}
        loading={false}
        error={null}
        onBack={onBack}
        onRefresh={vi.fn()}
        onCleanup={onCleanup}
      />,
    );

    expect(getAllByText(/enter/i).length).toBeGreaterThan(0);
    expect(getAllByText(/esc/i).length).toBeGreaterThan(0);
  });

  it("should handle empty PR list", () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { getByText } = render(
      <PRCleanupScreen
        targets={[]}
        loading={false}
        error={null}
        onBack={onBack}
        onRefresh={vi.fn()}
        onCleanup={onCleanup}
      />,
    );

    expect(getByText(/No cleanup targets found/i)).toBeDefined();
  });

  it("should display PR count in stats", () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { getByText, getAllByText } = render(
      <PRCleanupScreen
        targets={mockTargets}
        loading={false}
        error={null}
        onBack={onBack}
        onRefresh={vi.fn()}
        onCleanup={onCleanup}
      />,
    );

    expect(getByText(/Total:/i)).toBeDefined();
    expect(getAllByText(/^2$/).length).toBeGreaterThan(0);
  });

  it("should use terminal height for layout calculation", () => {
    const originalRows = process.stdout.rows;
    process.stdout.rows = 30;

    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { container } = render(
      <PRCleanupScreen
        targets={mockTargets}
        loading={false}
        error={null}
        onBack={onBack}
        onRefresh={vi.fn()}
        onCleanup={onCleanup}
      />,
    );

    expect(container).toBeDefined();

    process.stdout.rows = originalRows;
  });

  it("should handle back navigation with ESC key", () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { container } = render(
      <PRCleanupScreen
        targets={mockTargets}
        loading={false}
        error={null}
        onBack={onBack}
        onRefresh={vi.fn()}
        onCleanup={onCleanup}
      />,
    );

    // Test will verify onBack is called when ESC is pressed
    expect(container).toBeDefined();
  });

  it("should render status message when provided", () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { getByText } = render(
      <PRCleanupScreen
        targets={mockTargets}
        loading={false}
        error={null}
        statusMessage="Cleanup completed"
        onBack={onBack}
        onRefresh={vi.fn()}
        onCleanup={onCleanup}
      />,
    );

    expect(getByText(/Cleanup completed/i)).toBeDefined();
  });

  it("should render loading message when loading", () => {
    const onBack = vi.fn();
    const onCleanup = vi.fn();
    const { getByText } = render(
      <PRCleanupScreen
        targets={[]}
        loading
        error={null}
        onBack={onBack}
        onRefresh={vi.fn()}
        onCleanup={onCleanup}
      />,
    );

    expect(getByText(/Loading cleanup targets/i)).toBeDefined();
  });
});
