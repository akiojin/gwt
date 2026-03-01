import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, fireEvent, waitFor } from "@testing-library/svelte";
import type { GitHubIssueInfo, GhCliStatus, FetchIssuesResponse } from "../types";

// Mock $lib/tauriInvoke
const mockInvoke = vi.fn();
vi.mock("$lib/tauriInvoke", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// Mock @tauri-apps/plugin-shell
vi.mock("@tauri-apps/plugin-shell", () => ({
  open: vi.fn(),
}));

// Mock openExternalUrl
const mockOpenExternalUrl = vi.fn();
vi.mock("../openExternalUrl", () => ({
  openExternalUrl: (...args: unknown[]) => mockOpenExternalUrl(...args),
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
  onIssueCountChange?: (count: number) => void;
}) {
  const { default: IssueListPanel } = await import("./IssueListPanel.svelte");
  return render(IssueListPanel, {
    props: {
      projectPath: props?.projectPath ?? "/tmp/project",
      onWorkOnIssue: props?.onWorkOnIssue ?? vi.fn(),
      onSwitchToWorktree: props?.onSwitchToWorktree ?? vi.fn(),
      ...(props?.onIssueCountChange ? { onIssueCountChange: props.onIssueCountChange } : {}),
    },
  });
}

describe("IssueListPanel", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    mockOpenExternalUrl.mockReset();
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

  it("filters issues by label click and clears on re-click", async () => {
    const issues: GitHubIssueInfo[] = [
      makeIssue({ number: 1, title: "Bug report", labels: [{ name: "bug", color: "d73a4a" }] }),
      makeIssue({ number: 2, title: "New feature", labels: [{ name: "enhancement", color: "a2eeef" }] }),
    ];

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues") return { issues, hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Bug report")).toBeTruthy();
      expect(rendered.getByText("New feature")).toBeTruthy();
    });

    // Click "bug" label chip to filter
    const labelChips = rendered.container.querySelectorAll(".ilp-label-chip");
    const bugChip = Array.from(labelChips).find((el) => el.textContent?.trim() === "bug");
    expect(bugChip).toBeTruthy();
    await fireEvent.click(bugChip!);

    await waitFor(() => {
      expect(rendered.getByText("Bug report")).toBeTruthy();
      expect(rendered.queryByText("New feature")).toBeNull();
    });

    // Click "bug" chip again to clear filter
    await fireEvent.click(bugChip!);

    await waitFor(() => {
      expect(rendered.getByText("Bug report")).toBeTruthy();
      expect(rendered.getByText("New feature")).toBeTruthy();
    });
  });

  it("toggles active class on open/closed state buttons", async () => {
    const issues = [makeIssue({ number: 1, title: "Open Issue" })];

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues") return { issues, hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Open Issue")).toBeTruthy();
    });

    const openBtn = rendered.getByText("Open");
    const closedBtn = rendered.getByText("Closed");

    // Initially Open is active
    expect(openBtn.classList.contains("active")).toBe(true);
    expect(closedBtn.classList.contains("active")).toBe(false);

    // Click Closed
    await fireEvent.click(closedBtn);

    await waitFor(() => {
      expect(closedBtn.classList.contains("active")).toBe(true);
      expect(openBtn.classList.contains("active")).toBe(false);
    });
  });

  it("clears issue list on refresh button click", async () => {
    const issues = [makeIssue({ number: 1, title: "Issue A" })];

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues") return { issues, hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Issue A")).toBeTruthy();
    });

    // Verify refresh button exists
    const refreshBtn = rendered.container.querySelector(".ilp-refresh-btn");
    expect(refreshBtn).toBeTruthy();

    // handleRefresh sets issues=[] then calls fetchIssues(1).
    // The fetchIssues re-import may fail in JSDOM, but handleRefresh
    // immediately clears the list, so "Issue A" should disappear.
    await fireEvent.click(refreshBtn!);

    await waitFor(() => {
      expect(rendered.queryByText("Issue A")).toBeNull();
    });
  });

  it("revalidates branch linkage on refresh even when previously cached", async () => {
    const issue = makeIssue({ number: 5, title: "Refresh Linkage Issue" });
    let branchLookupCount = 0;

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues") return { issues: [issue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") {
        branchLookupCount += 1;
        return branchLookupCount === 1 ? null : "feature/issue-5";
      }
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Refresh Linkage Issue")).toBeTruthy();
      expect(branchLookupCount).toBe(1);
    });
    expect(rendered.queryByText("WT")).toBeNull();

    const refreshBtn = rendered.container.querySelector(".ilp-refresh-btn");
    expect(refreshBtn).toBeTruthy();
    await fireEvent.click(refreshBtn!);

    await waitFor(() => {
      expect(branchLookupCount).toBeGreaterThanOrEqual(2);
      expect(rendered.getByText("WT")).toBeTruthy();
    });
  });

  it("navigates to detail view on issue click and back to list", async () => {
    const issue = makeIssue({ number: 42, title: "Detail Test Issue", body: "Issue body content" });

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues") return { issues: [issue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      if (cmd === "fetch_github_issue_detail") return issue;
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Detail Test Issue")).toBeTruthy();
    });

    // Click issue row to open detail
    const issueRow = rendered.container.querySelector(".ilp-issue-row");
    expect(issueRow).toBeTruthy();
    await fireEvent.click(issueRow!);

    await waitFor(() => {
      // Back button should be visible in detail view
      expect(rendered.getByText(/Back/)).toBeTruthy();
    });

    // Click back button
    await fireEvent.click(rendered.getByText(/Back/));

    await waitFor(() => {
      // Should be back in list view
      expect(rendered.getByText("Detail Test Issue")).toBeTruthy();
      expect(rendered.queryByText(/Back/)).toBeNull();
    });
  });

  it("preserves search filter after navigating to detail and back", async () => {
    const issues: GitHubIssueInfo[] = [
      makeIssue({ number: 1, title: "Fix login bug" }),
      makeIssue({ number: 2, title: "Add feature" }),
    ];

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues") return { issues, hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      if (cmd === "fetch_github_issue_detail") return issues[0];
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Fix login bug")).toBeTruthy();
      expect(rendered.getByText("Add feature")).toBeTruthy();
    });

    // Apply search filter
    const searchInput = rendered.container.querySelector('input[placeholder*="Search"]') as HTMLInputElement;
    await fireEvent.input(searchInput, { target: { value: "login" } });

    await waitFor(() => {
      expect(rendered.getByText("Fix login bug")).toBeTruthy();
      expect(rendered.queryByText("Add feature")).toBeNull();
    });

    // Click the filtered issue to go to detail
    const issueRow = rendered.container.querySelector(".ilp-issue-row");
    await fireEvent.click(issueRow!);

    await waitFor(() => {
      expect(rendered.getByText(/Back/)).toBeTruthy();
    });

    // Click Back to return to list
    await fireEvent.click(rendered.getByText(/Back/));

    // Filter should still be applied
    await waitFor(() => {
      expect(rendered.getByText("Fix login bug")).toBeTruthy();
      expect(rendered.queryByText("Add feature")).toBeNull();
    });

    // Search input should retain its value
    const searchInputAfter = rendered.container.querySelector('input[placeholder*="Search"]') as HTMLInputElement;
    expect(searchInputAfter.value).toBe("login");
  });

  it("calls onWorkOnIssue when 'Work on this' is clicked in detail", async () => {
    const issue = makeIssue({ number: 7, title: "Work Issue", body: "Some body" });
    const onWorkOnIssue = vi.fn();

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues") return { issues: [issue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      if (cmd === "fetch_github_issue_detail") return issue;
      return null;
    });

    const rendered = await renderIssueListPanel({ onWorkOnIssue });

    await waitFor(() => {
      expect(rendered.getByText("Work Issue")).toBeTruthy();
    });

    // Open detail
    await fireEvent.click(rendered.container.querySelector(".ilp-issue-row")!);

    await waitFor(() => {
      expect(rendered.getByText("Work on this")).toBeTruthy();
    });

    // Click "Work on this"
    await fireEvent.click(rendered.getByText("Work on this"));

    expect(onWorkOnIssue).toHaveBeenCalledTimes(1);
    expect(onWorkOnIssue).toHaveBeenCalledWith(expect.objectContaining({ number: 7 }));
  });

  it("shows 'Switch to Worktree' when branch exists for issue", async () => {
    const issue = makeIssue({ number: 5, title: "Linked Issue", body: "body" });
    const onSwitchToWorktree = vi.fn();

    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues") return { issues: [issue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") {
        const issueNum = (args as { issueNumber?: number })?.issueNumber;
        return issueNum === 5 ? "feature/issue-5" : null;
      }
      if (cmd === "fetch_github_issue_detail") return issue;
      return null;
    });

    const rendered = await renderIssueListPanel({ onSwitchToWorktree });

    await waitFor(() => {
      expect(rendered.getByText("Linked Issue")).toBeTruthy();
    });

    // WT button in list row
    await waitFor(() => {
      expect(rendered.getByText("WT")).toBeTruthy();
    });

    // Open detail
    await fireEvent.click(rendered.container.querySelector(".ilp-issue-row")!);

    await waitFor(() => {
      expect(rendered.getByText("Switch to Worktree")).toBeTruthy();
      // "Work on this" should NOT be visible
      expect(rendered.queryByText("Work on this")).toBeNull();
    });

    await fireEvent.click(rendered.getByText("Switch to Worktree"));
    expect(onSwitchToWorktree).toHaveBeenCalledWith("feature/issue-5");
  });

  it("calls onIssueCountChange when issues are loaded", async () => {
    const issues = [
      makeIssue({ number: 1, title: "A" }),
      makeIssue({ number: 2, title: "B" }),
      makeIssue({ number: 3, title: "C" }),
    ];
    const onIssueCountChange = vi.fn();

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues") return { issues, hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      return null;
    });

    await renderIssueListPanel({ onIssueCountChange });

    await waitFor(() => {
      expect(onIssueCountChange).toHaveBeenCalledWith(3);
    });
  });

  it("displays assignees, milestones, and comments in list view", async () => {
    const issue = makeIssue({
      number: 10,
      title: "Full meta Issue",
      assignees: [
        { login: "alice", avatarUrl: "https://avatars.example.com/alice.png" },
        { login: "bob", avatarUrl: "https://avatars.example.com/bob.png" },
      ],
      milestone: { title: "v2.0", number: 20 },
      commentsCount: 7,
      labels: [{ name: "bug", color: "d73a4a" }],
    });

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues")
        return { issues: [issue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Full meta Issue")).toBeTruthy();
    });

    // Assignee avatars
    const avatars = rendered.container.querySelectorAll(".ilp-avatar");
    expect(avatars.length).toBeGreaterThanOrEqual(2);
    expect(avatars[0]?.getAttribute("alt")).toBe("alice");
    expect(avatars[1]?.getAttribute("alt")).toBe("bob");

    // Milestone
    expect(rendered.container.textContent).toContain("v2.0");

    // Comments count
    expect(rendered.getByText("7")).toBeTruthy();
  });

  it("displays detail view with assignees, milestone, and comments count", async () => {
    const issue = makeIssue({
      number: 20,
      title: "Detail Meta Issue",
      state: "closed",
      body: "Some body text",
      assignees: [
        { login: "charlie", avatarUrl: "https://avatars.example.com/charlie.png" },
      ],
      milestone: { title: "Sprint 5", number: 5 },
      commentsCount: 12,
      labels: [{ name: "enhancement", color: "a2eeef" }],
    });

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues")
        return { issues: [issue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      if (cmd === "fetch_github_issue_detail") return issue;
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Detail Meta Issue")).toBeTruthy();
    });

    // Open detail
    await fireEvent.click(rendered.container.querySelector(".ilp-issue-row")!);

    await waitFor(() => {
      expect(rendered.getByText(/Back/)).toBeTruthy();
      // Wait until detail is fully loaded (not in loading state)
      expect(rendered.container.querySelector(".ilp-detail-state")).toBeTruthy();
    });

    // State badge should show "closed" with the closed class
    const stateBadge = rendered.container.querySelector(".ilp-detail-state");
    expect(stateBadge?.textContent?.trim()).toBe("closed");
    expect(stateBadge?.classList.contains("closed")).toBe(true);

    // Assignee avatar in detail
    const detailAvatars = rendered.container.querySelectorAll(
      ".ilp-detail-meta .ilp-avatar"
    );
    expect(detailAvatars.length).toBeGreaterThanOrEqual(1);

    // Milestone in detail
    expect(rendered.container.querySelector(".ilp-detail-milestone")).toBeTruthy();
    expect(rendered.container.textContent).toContain("Sprint 5");

    // Comments in detail
    expect(rendered.container.textContent).toContain("12 comments");

    // Labels in detail
    expect(rendered.container.querySelector(".ilp-detail-meta .ilp-issue-label")).toBeTruthy();
  });

  it("shows 'No description provided.' when detail issue has no body", async () => {
    const issue = makeIssue({
      number: 30,
      title: "No Body Issue",
      body: undefined,
    });

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues")
        return { issues: [issue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      if (cmd === "fetch_github_issue_detail") return issue;
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("No Body Issue")).toBeTruthy();
    });

    await fireEvent.click(rendered.container.querySelector(".ilp-issue-row")!);

    await waitFor(() => {
      expect(rendered.getByText("No description provided.")).toBeTruthy();
    });
  });

  it("shows detail error from fetch_github_issue_detail failure", async () => {
    const issue = makeIssue({ number: 40, title: "Error Detail Issue", body: "body" });

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues")
        return { issues: [issue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      if (cmd === "fetch_github_issue_detail") throw new Error("Detail fetch failed");
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Error Detail Issue")).toBeTruthy();
    });

    await fireEvent.click(rendered.container.querySelector(".ilp-issue-row")!);

    // The detail view should show - fetchIssueDetail catches the error and
    // sets detailError while also falling back to the initial issue.
    // The error message may be from the dynamic import or from the mock throw.
    await waitFor(() => {
      expect(rendered.getByText(/Back/)).toBeTruthy();
      const errorDiv = rendered.container.querySelector(".ilp-error");
      expect(errorDiv).toBeTruthy();
    });
  });

  it("clicks 'Open in GitHub' in detail view", async () => {
    const issue = makeIssue({
      number: 50,
      title: "GitHub Link Issue",
      body: "body text",
      htmlUrl: "https://github.com/test/repo/issues/50",
    });

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues")
        return { issues: [issue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      if (cmd === "fetch_github_issue_detail") return issue;
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("GitHub Link Issue")).toBeTruthy();
    });

    await fireEvent.click(rendered.container.querySelector(".ilp-issue-row")!);

    await waitFor(() => {
      expect(rendered.getByText("Open in GitHub")).toBeTruthy();
    });

    // Click the button - it should invoke openExternalUrl
    await fireEvent.click(rendered.getByText("Open in GitHub"));

    await waitFor(() => {
      expect(mockOpenExternalUrl).toHaveBeenCalledWith(
        "https://github.com/test/repo/issues/50"
      );
    });
  });

  it("handles non-Error throws in toErrorMessage", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues") throw 42;
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("42");
    });
  });

  it("handles string error in toErrorMessage", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues") throw "plain string error";
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("plain string error");
    });
  });

  it("handles check_gh_cli_status exception gracefully", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") throw new Error("CLI check failed");
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      // Should fall back to ghCliStatus = { available: false, authenticated: false }
      expect(rendered.getByText(/GitHub CLI.*not available/i)).toBeTruthy();
    });
  });

  it("does not re-fetch when clicking already active state filter", async () => {
    const issues = [makeIssue({ number: 1, title: "Open Issue" })];
    let fetchCallCount = 0;

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues") {
        fetchCallCount++;
        return { issues, hasNextPage: false } as FetchIssuesResponse;
      }
      if (cmd === "find_existing_issue_branch") return null;
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Open Issue")).toBeTruthy();
    });

    const initialCallCount = fetchCallCount;

    // Click the already active "Open" button
    const openBtn = rendered.getByText("Open");
    await fireEvent.click(openBtn);

    // Should not trigger additional fetch
    expect(fetchCallCount).toBe(initialCallCount);
  });

  it("clears label filter with Clear button", async () => {
    const issues: GitHubIssueInfo[] = [
      makeIssue({ number: 1, title: "Bug report", labels: [{ name: "bug", color: "d73a4a" }] }),
      makeIssue({ number: 2, title: "New feature", labels: [{ name: "enhancement", color: "a2eeef" }] }),
    ];

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues")
        return { issues, hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Bug report")).toBeTruthy();
    });

    // Click a label chip to filter
    const labelChips = rendered.container.querySelectorAll(".ilp-label-chip");
    const bugChip = Array.from(labelChips).find((el) => el.textContent?.trim() === "bug");
    await fireEvent.click(bugChip!);

    await waitFor(() => {
      expect(rendered.queryByText("New feature")).toBeNull();
    });

    // Clear button should appear
    const clearBtn = rendered.container.querySelector(".ilp-label-clear");
    expect(clearBtn).toBeTruthy();
    expect(clearBtn?.textContent?.trim()).toBe("Clear");

    await fireEvent.click(clearBtn!);

    await waitFor(() => {
      expect(rendered.getByText("Bug report")).toBeTruthy();
      expect(rendered.getByText("New feature")).toBeTruthy();
    });
  });

  it("shows WT button in list and prevents event propagation on click", async () => {
    const issue = makeIssue({ number: 5, title: "WT Click Issue" });
    const onSwitchToWorktree = vi.fn();

    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues")
        return { issues: [issue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") {
        const issueNum = (args as { issueNumber?: number })?.issueNumber;
        return issueNum === 5 ? "feature/issue-5" : null;
      }
      if (cmd === "fetch_github_issue_detail") return issue;
      return null;
    });

    const rendered = await renderIssueListPanel({ onSwitchToWorktree });

    await waitFor(() => {
      expect(rendered.getByText("WT")).toBeTruthy();
    });

    // Click WT button - should not navigate to detail
    const wtBtn = rendered.getByText("WT");
    await fireEvent.click(wtBtn);

    expect(onSwitchToWorktree).toHaveBeenCalledWith("feature/issue-5");
    // Should still be in list view (not detail)
    expect(rendered.queryByText(/Back/)).toBeNull();
  });

  it("displays issue with invalid label color gracefully", async () => {
    const issue = makeIssue({
      number: 60,
      title: "Invalid Color Issue",
      labels: [{ name: "weird", color: "xyz123" }],
    });

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues")
        return { issues: [issue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Invalid Color Issue")).toBeTruthy();
    });

    // The label should still render even with invalid color
    const labels = rendered.container.querySelectorAll(".ilp-issue-label");
    const weirdLabel = Array.from(labels).find((el) => el.textContent?.trim() === "weird");
    expect(weirdLabel).toBeTruthy();
    // With invalid hex, labelStyle returns "" so style should be empty
    expect(weirdLabel?.getAttribute("style")).toBe("");
  });

  it("shows spec issue detail with IssueSpecPanel", async () => {
    const specIssue = makeIssue({
      number: 70,
      title: "Spec Issue",
      body: "Some spec body",
      labels: [{ name: "gwt-spec", color: "0075ca" }],
    });

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues")
        return { issues: [specIssue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      if (cmd === "fetch_github_issue_detail") return specIssue;
      if (cmd === "fetch_issue_spec") return { markdown: "# Spec content", error: null };
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Spec Issue")).toBeTruthy();
    });

    await fireEvent.click(rendered.container.querySelector(".ilp-issue-row")!);

    await waitFor(() => {
      expect(rendered.getByText(/Back/)).toBeTruthy();
      // The IssueSpecPanel component should be rendered for spec issues
      // We can check the detail body exists
      expect(rendered.container.querySelector(".ilp-detail-body")).toBeTruthy();
    });
  });

  it("shows loading indicator when issues are being loaded", async () => {
    // Use a never-resolving promise to keep the loading state visible
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues") {
        // Never resolve - keeps loading=true
        return new Promise<FetchIssuesResponse>(() => {});
      }
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Loading issues...")).toBeTruthy();
    });
  });

  it("handles detail loading indicator while fetching issue detail", async () => {
    const issue = makeIssue({ number: 80, title: "Slow Detail Issue", body: "body" });

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues")
        return { issues: [issue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") return null;
      if (cmd === "fetch_github_issue_detail") {
        // Never resolve - keeps detail loading state
        return new Promise<GitHubIssueInfo>(() => {});
      }
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Slow Detail Issue")).toBeTruthy();
    });

    await fireEvent.click(rendered.container.querySelector(".ilp-issue-row")!);

    await waitFor(() => {
      expect(rendered.getByText("Loading issue details...")).toBeTruthy();
    });
  });

  it("handles find_existing_issue_branch error gracefully", async () => {
    const issue = makeIssue({ number: 90, title: "Branch Error Issue" });

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status")
        return { available: true, authenticated: true } as GhCliStatus;
      if (cmd === "fetch_github_issues")
        return { issues: [issue], hasNextPage: false } as FetchIssuesResponse;
      if (cmd === "find_existing_issue_branch") throw new Error("Branch lookup failed");
      return null;
    });

    const rendered = await renderIssueListPanel();

    await waitFor(() => {
      expect(rendered.getByText("Branch Error Issue")).toBeTruthy();
    });

    // WT button should not be shown (error falls back to null)
    expect(rendered.queryByText("WT")).toBeNull();
  });
});
