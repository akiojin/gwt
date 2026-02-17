import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, fireEvent, waitFor } from "@testing-library/svelte";
import type { GitHubIssueInfo, GhCliStatus, FetchIssuesResponse } from "../types";

// Mock @tauri-apps/api/core
const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// Mock @tauri-apps/plugin-shell
vi.mock("@tauri-apps/plugin-shell", () => ({
  open: vi.fn(),
}));

function makeIssue(overrides: Partial<GitHubIssueInfo> = {}): GitHubIssueInfo {
  return {
    number: 1,
    title: "Test Issue",
    state: "open",
    updatedAt: "2025-01-01T00:00:00Z",
    htmlUrl: "https://github.com/test/repo/issues/1",
    labels: [],
    assignees: [],
    commentsCount: 0,
    ...overrides,
  };
}

async function renderIssueListPanel(props?: {
  projectPath?: string;
  onWorkOnIssue?: (issue: GitHubIssueInfo) => void;
  onSwitchToWorktree?: (branchName: string) => void;
}) {
  const { default: IssueListPanel } = await import("./IssueListPanel.svelte");
  return render(IssueListPanel, {
    props: {
      projectPath: props?.projectPath ?? "/tmp/project",
      onWorkOnIssue: props?.onWorkOnIssue ?? vi.fn(),
      onSwitchToWorktree: props?.onSwitchToWorktree ?? vi.fn(),
    },
  });
}

describe("IssueListPanel", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("renders issue list after loading", async () => {
    const issues: GitHubIssueInfo[] = [
      makeIssue({ number: 10, title: "First Issue", labels: [{ name: "bug", color: "d73a4a" }] }),
      makeIssue({ number: 20, title: "Second Issue", commentsCount: 5 }),
    ];

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") {
        return { available: true, authenticated: true } as GhCliStatus;
      }
      if (cmd === "fetch_github_issues") {
        return { issues, hasNextPage: false } as FetchIssuesResponse;
      }
      if (cmd === "find_existing_issue_branch") {
        return null;
      }
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("First Issue")).toBeTruthy();
    });

    expect(rendered.getByText("#10")).toBeTruthy();
    expect(rendered.getByText("Second Issue")).toBeTruthy();
    expect(rendered.getByText("#20")).toBeTruthy();
    // "bug" label appears in both issue row and filter chips
    expect(rendered.getAllByText("bug").length).toBeGreaterThanOrEqual(1);
  });

  it("shows error when gh CLI is not available", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") {
        return { available: false, authenticated: false } as GhCliStatus;
      }
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText(/GitHub CLI.*not available/i)).toBeTruthy();
    });
  });

  it("shows empty state when no issues found", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") {
        return { available: true, authenticated: true } as GhCliStatus;
      }
      if (cmd === "fetch_github_issues") {
        return { issues: [], hasNextPage: false } as FetchIssuesResponse;
      }
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText(/No issues found/i)).toBeTruthy();
    });
  });

  it("shows error when fetch_github_issues fails", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") {
        return { available: true, authenticated: true } as GhCliStatus;
      }
      if (cmd === "fetch_github_issues") {
        throw new Error("Network error");
      }
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText(/Network error/i)).toBeTruthy();
    });
  });

  it("filters issues by title search", async () => {
    const issues: GitHubIssueInfo[] = [
      makeIssue({ number: 1, title: "Fix login bug" }),
      makeIssue({ number: 2, title: "Add feature" }),
    ];

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") {
        return { available: true, authenticated: true } as GhCliStatus;
      }
      if (cmd === "fetch_github_issues") {
        return { issues, hasNextPage: false } as FetchIssuesResponse;
      }
      if (cmd === "find_existing_issue_branch") return null;
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Fix login bug")).toBeTruthy();
    });

    const searchInput = rendered.container.querySelector('input[placeholder*="Search"]') as HTMLInputElement;
    expect(searchInput).toBeTruthy();

    await fireEvent.input(searchInput, { target: { value: "login" } });

    await waitFor(() => {
      expect(rendered.getByText("Fix login bug")).toBeTruthy();
      expect(rendered.queryByText("Add feature")).toBeNull();
    });
  });
});
