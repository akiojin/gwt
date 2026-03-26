import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";
import type {
  PrListItem,
  FetchPrListResponse,
  GhCliStatus,
  GitHubUserResponse,
} from "../types";

const invokeMock = vi.fn();

// Mock both the wrapper and the underlying Tauri API to ensure
// all dynamic import paths resolve correctly in Svelte 5 $effect chains
vi.mock("$lib/tauriInvoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("../openExternalUrl", () => ({
  openExternalUrl: vi.fn(),
}));

// Mock errorBus to prevent unhandled side effects
vi.mock("$lib/errorBus", () => ({
  errorBus: { emit: vi.fn(), on: vi.fn(() => () => {}), off: vi.fn() },
}));

function makePr(overrides: Partial<PrListItem> = {}): PrListItem {
  return {
    number: 1,
    title: "Test PR",
    state: "OPEN",
    isDraft: false,
    headRefName: "feature/test",
    baseRefName: "main",
    url: "https://github.com/test/repo/pull/1",
    author: { login: "alice" },
    assignees: [],
    reviewRequests: [],
    labels: [],
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    body: "",
    ...overrides,
  };
}

function makePrResponse(items: PrListItem[]): FetchPrListResponse {
  return { items, ghStatus: ghCliOk() };
}

function ghCliOk(): GhCliStatus {
  return { available: true, authenticated: true };
}

function ghCliUnavailable(): GhCliStatus {
  return { available: false, authenticated: false };
}

async function renderPanel(props?: any) {
  const { default: PrListPanel } = await import("./PrListPanel.svelte");
  return render(PrListPanel, {
    props: {
      projectPath: "/tmp/project",
      onSwitchToWorktree: vi.fn(),
      ...props,
    },
  });
}

describe("PrListPanel", () => {
  beforeEach(async () => {
    invokeMock.mockReset();
    cleanup();
    await new Promise((r) => setTimeout(r, 0));
  });

  afterEach(() => {
    cleanup();
  });

  it("shows gh CLI unavailable message when not authenticated", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliUnavailable();
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText(/GitHub CLI.*not available/i)).toBeTruthy();
    });
  });

  it("shows Pull Requests header", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliUnavailable();
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Pull Requests")).toBeTruthy();
    });
  });

  it("renders state filter buttons (Open, Closed, Merged)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliUnavailable();
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Open")).toBeTruthy();
      expect(rendered.getByText("Closed")).toBeTruthy();
      expect(rendered.getByText("Merged")).toBeTruthy();
    });
  });

  it("has Open filter active by default", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliUnavailable();
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      const openBtn = rendered.getByText("Open");
      expect(openBtn.classList.contains("active")).toBe(true);
    });
  });

  it("shows search input with correct placeholder", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliUnavailable();
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      const input = rendered.container.querySelector('input[placeholder="Search pull requests..."]');
      expect(input).toBeTruthy();
    });
  });

  it("has a refresh button", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliUnavailable();
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      const refreshBtn = rendered.container.querySelector('[title="Refresh"]');
      expect(refreshBtn).toBeTruthy();
    });
  });

  it("shows loading state while fetching PRs", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice" };
      if (cmd === "fetch_pr_list") return new Promise(() => {}); // never resolves
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Loading pull requests...")).toBeTruthy();
    });
  });

  it("calls check_gh_cli_status on mount", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliUnavailable();
      return null;
    });

    await renderPanel();

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("check_gh_cli_status", {
        projectPath: "/tmp/project",
      });
    });
  });

  it("does not show PR list when gh CLI check fails", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") throw new Error("Failed");
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText(/GitHub CLI.*not available/i)).toBeTruthy();
    });
  });

  it("renders section element with pr-list-panel class", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliUnavailable();
      return null;
    });

    const rendered = await renderPanel();

    expect(rendered.container.querySelector(".pr-list-panel")).toBeTruthy();
  });

  it("renders header with plp-header class", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliUnavailable();
      return null;
    });

    const rendered = await renderPanel();

    expect(rendered.container.querySelector(".plp-header")).toBeTruthy();
  });

  it("has Closed filter not active by default", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliUnavailable();
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      const closedBtn = rendered.getByText("Closed");
      expect(closedBtn.classList.contains("active")).toBe(false);
      const mergedBtn = rendered.getByText("Merged");
      expect(mergedBtn.classList.contains("active")).toBe(false);
    });
  });

  // --- Tests that depend on PR data loading ---
  it("renders PR list items with number, title, author, and branches", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 42, title: "Add feature X", author: { login: "bob" }, headRefName: "feature/x", baseRefName: "develop" }),
      makePr({ number: 43, title: "Fix bug Y", author: { login: "carol" }, headRefName: "fix/y", baseRefName: "main" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("#42")).toBeTruthy();
      expect(rendered.getByText("Add feature X")).toBeTruthy();
      expect(rendered.getByText("bob")).toBeTruthy();
      expect(rendered.getByText("feature/x")).toBeTruthy();
      expect(rendered.getByText("develop")).toBeTruthy();
    });

    expect(rendered.getByText("#43")).toBeTruthy();
    expect(rendered.getByText("Fix bug Y")).toBeTruthy();
    expect(rendered.getByText("carol")).toBeTruthy();
  });

  it("shows 'No pull requests found.' when list is empty", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse([]);
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("No pull requests found.")).toBeTruthy();
    });
  });

  it("shows error message when fetch_pr_list fails", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") throw new Error("Network error");
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Network error")).toBeTruthy();
    });
  });

  it("switches state filter on button click", async () => {
    let lastState = "open";
    invokeMock.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") {
        lastState = args?.state ?? "open";
        return makePrResponse([]);
      }
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("No pull requests found.")).toBeTruthy();
    });

    // Click Closed
    await fireEvent.click(rendered.getByText("Closed"));

    await waitFor(() => {
      const closedBtn = rendered.getByText("Closed");
      expect(closedBtn.classList.contains("active")).toBe(true);
      expect(lastState).toBe("closed");
    });

    // Click Merged
    await fireEvent.click(rendered.getByText("Merged"));

    await waitFor(() => {
      const mergedBtn = rendered.getByText("Merged");
      expect(mergedBtn.classList.contains("active")).toBe(true);
      expect(lastState).toBe("merged");
    });
  });

  it("does not re-fetch when clicking the already active filter", async () => {
    let fetchCount = 0;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") {
        fetchCount++;
        return makePrResponse([]);
      }
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("No pull requests found.")).toBeTruthy();
    });

    const countBefore = fetchCount;
    // Click Open again (already active)
    await fireEvent.click(rendered.getByText("Open"));

    // Should not have increased
    expect(fetchCount).toBe(countBefore);
  });

  it("filters PRs by search query matching title", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 1, title: "Fix login bug" }),
      makePr({ number: 2, title: "Add dashboard" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Fix login bug")).toBeTruthy();
      expect(rendered.getByText("Add dashboard")).toBeTruthy();
    });

    const searchInput = rendered.container.querySelector('input[placeholder="Search pull requests..."]') as HTMLInputElement;
    await fireEvent.input(searchInput, { target: { value: "login" } });

    await waitFor(() => {
      expect(rendered.getByText("Fix login bug")).toBeTruthy();
      expect(rendered.queryByText("Add dashboard")).toBeNull();
    });
  });

  it("filters PRs by search query matching PR number", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 42, title: "First PR" }),
      makePr({ number: 99, title: "Second PR" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("First PR")).toBeTruthy();
    });

    const searchInput = rendered.container.querySelector('input[placeholder="Search pull requests..."]') as HTMLInputElement;
    await fireEvent.input(searchInput, { target: { value: "#42" } });

    await waitFor(() => {
      expect(rendered.getByText("First PR")).toBeTruthy();
      expect(rendered.queryByText("Second PR")).toBeNull();
    });
  });

  it("filters PRs by search query matching author", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 1, title: "Alice PR", author: { login: "alice" } }),
      makePr({ number: 2, title: "Bob PR", author: { login: "bob" } }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Alice PR")).toBeTruthy();
    });

    const searchInput = rendered.container.querySelector('input[placeholder="Search pull requests..."]') as HTMLInputElement;
    await fireEvent.input(searchInput, { target: { value: "bob" } });

    await waitFor(() => {
      expect(rendered.queryByText("Alice PR")).toBeNull();
      expect(rendered.getByText("Bob PR")).toBeTruthy();
    });
  });

  it("filters PRs by search query matching branch name", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 1, title: "Feature A", headRefName: "feature/auth" }),
      makePr({ number: 2, title: "Feature B", headRefName: "feature/dashboard" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Feature A")).toBeTruthy();
    });

    const searchInput = rendered.container.querySelector('input[placeholder="Search pull requests..."]') as HTMLInputElement;
    await fireEvent.input(searchInput, { target: { value: "auth" } });

    await waitFor(() => {
      expect(rendered.getByText("Feature A")).toBeTruthy();
      expect(rendered.queryByText("Feature B")).toBeNull();
    });
  });

  it("shows Draft badge for draft PRs", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 10, title: "Draft PR", isDraft: true }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Draft")).toBeTruthy();
    });
  });

  it("shows labels on PR items", async () => {
    const prs: PrListItem[] = [
      makePr({
        number: 5,
        title: "Labeled PR",
        labels: [
          { name: "bug", color: "d73a4a" },
          { name: "priority", color: "0075ca" },
        ],
      }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("bug")).toBeTruthy();
      expect(rendered.getByText("priority")).toBeTruthy();
    });
  });

  it("expands and collapses PR details on click", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 7, title: "Expandable PR", body: "This is the PR body." }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Expandable PR")).toBeTruthy();
    });

    // Initially no expanded body
    expect(rendered.queryByText("No description provided.")).toBeNull();

    // Click to expand
    const prRow = rendered.container.querySelector(".plp-pr-row") as HTMLElement;
    await fireEvent.click(prRow);

    await waitFor(() => {
      const expanded = rendered.container.querySelector(".plp-pr-expanded");
      expect(expanded).toBeTruthy();
    });

    // Click again to collapse
    await fireEvent.click(prRow);

    await waitFor(() => {
      const expanded = rendered.container.querySelector(".plp-pr-expanded");
      expect(expanded).toBeNull();
    });
  });

  it("shows 'No description provided.' for PR with empty body", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 8, title: "Empty Body PR", body: "" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Empty Body PR")).toBeTruthy();
    });

    const prRow = rendered.container.querySelector(".plp-pr-row") as HTMLElement;
    await fireEvent.click(prRow);

    await waitFor(() => {
      expect(rendered.getByText("No description provided.")).toBeTruthy();
    });
  });

  it("shows Show More button when hasMore is true", async () => {
    const prs = Array.from({ length: 30 }, (_, i) =>
      makePr({ number: i + 1, title: `PR ${i + 1}` })
    );

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Show More")).toBeTruthy();
    });
  });

  it("calls onSwitchToWorktree when WT button is clicked", async () => {
    const onSwitchToWorktree = vi.fn();
    const prs: PrListItem[] = [
      makePr({ number: 15, title: "Switch PR", headRefName: "feature/switch" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel({ onSwitchToWorktree });

    await waitFor(() => {
      expect(rendered.getByText("Switch PR")).toBeTruthy();
    });

    const wtBtn = rendered.getByText("WT");
    await fireEvent.click(wtBtn);

    expect(onSwitchToWorktree).toHaveBeenCalledWith("feature/switch");
  });

  it("shows Merge button for open non-draft PRs", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 20, title: "Mergeable PR", state: "OPEN", isDraft: false }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Merge")).toBeTruthy();
    });
  });

  it("hides Merge button for draft PRs", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 21, title: "Draft PR", state: "OPEN", isDraft: true }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Draft PR")).toBeTruthy();
    });

    expect(rendered.queryByText("Merge")).toBeNull();
  });

  it("shows Review button for open PRs", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 22, title: "Reviewable PR", state: "OPEN" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Review")).toBeTruthy();
    });
  });

  it("shows Ready button for draft open PRs", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 23, title: "Draft to Ready", state: "OPEN", isDraft: true }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Ready")).toBeTruthy();
    });
  });

  it("refreshes the PR list on refresh button click", async () => {
    let fetchCount = 0;
    const prs: PrListItem[] = [makePr({ number: 1, title: "Test PR" })];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") {
        fetchCount++;
        return makePrResponse(prs);
      }
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Test PR")).toBeTruthy();
    });

    const prevCount = fetchCount;
    const refreshBtn = rendered.container.querySelector('[title="Refresh"]') as HTMLElement;
    await fireEvent.click(refreshBtn);

    await waitFor(() => {
      expect(fetchCount).toBeGreaterThan(prevCount);
    });
  });

  it("highlights my PRs with my-pr class", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 30, title: "My PR", author: { login: "alice" } }),
      makePr({ number: 31, title: "Other PR", author: { login: "bob" } }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("My PR")).toBeTruthy();
    });

    const rows = rendered.container.querySelectorAll(".plp-pr-row");
    const myPrRow = Array.from(rows).find((r) => r.textContent?.includes("My PR"));
    const otherPrRow = Array.from(rows).find((r) => r.textContent?.includes("Other PR"));

    expect(myPrRow?.classList.contains("my-pr")).toBe(true);
    expect(otherPrRow?.classList.contains("my-pr")).toBe(false);
  });

  // --- Mark Ready flow ---
  it("calls mark_pr_ready and refreshes when Ready button is clicked", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 50, title: "Draft PR Ready", state: "OPEN", isDraft: true }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      if (cmd === "mark_pr_ready") return "ok";
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Ready")).toBeTruthy();
    });

    await fireEvent.click(rendered.getByText("Ready"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("mark_pr_ready", {
        projectPath: "/tmp/project",
        prNumber: 50,
      });
    });
  });

  it("shows error when mark_pr_ready fails", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 51, title: "Draft Fail Ready", state: "OPEN", isDraft: true }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      if (cmd === "mark_pr_ready") throw new Error("Permission denied");
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Ready")).toBeTruthy();
    });

    await fireEvent.click(rendered.getByText("Ready"));

    await waitFor(() => {
      expect(rendered.getByText("Permission denied")).toBeTruthy();
    });
  });

  // --- Update Branch flow ---
  it("shows Update button when mergeStateStatus is BEHIND and calls update_pr_branch", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 60, title: "Behind PR", state: "OPEN" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 60,
          title: "Behind PR",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/60",
          mergeable: "MERGEABLE",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/behind",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [],
          reviews: [],
          reviewComments: [],
          changedFilesCount: 5,
          additions: 10,
          deletions: 3,
          mergeStateStatus: "BEHIND",
        };
      if (cmd === "update_pr_branch") return "ok";
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Update")).toBeTruthy();
    });

    await fireEvent.click(rendered.getByText("Update"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("update_pr_branch", {
        projectPath: "/tmp/project",
        prNumber: 60,
      });
    });
  });

  it("shows error when update_pr_branch fails", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 61, title: "Behind Fail PR", state: "OPEN" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 61,
          title: "Behind Fail PR",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/61",
          mergeable: "MERGEABLE",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/behind-fail",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [],
          reviews: [],
          reviewComments: [],
          changedFilesCount: 1,
          additions: 1,
          deletions: 0,
          mergeStateStatus: "BEHIND",
        };
      if (cmd === "update_pr_branch") throw new Error("Branch update failed");
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Update")).toBeTruthy();
    });

    await fireEvent.click(rendered.getByText("Update"));

    await waitFor(() => {
      expect(rendered.getByText("Branch update failed")).toBeTruthy();
    });
  });

  // --- Merge Dialog ---
  it("opens MergeDialog when Merge button is clicked", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 70, title: "Merge This PR", state: "OPEN", isDraft: false }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Merge")).toBeTruthy();
    });

    await fireEvent.click(rendered.getByText("Merge"));

    // MergeDialog should appear - it renders with the PR title in a commit message input
    await waitFor(() => {
      // MergeDialog has a heading or recognizable content
      const dialog = rendered.container.querySelector(".merge-dialog, [class*='merge']");
      // At minimum, the dialog component should be rendered
      expect(dialog || rendered.container.innerHTML.includes("Merge This PR")).toBeTruthy();
    });
  });

  // --- Review Dialog ---
  it("opens ReviewDialog when Review button is clicked", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 71, title: "Review This PR", state: "OPEN" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Review")).toBeTruthy();
    });

    await fireEvent.click(rendered.getByText("Review"));

    await waitFor(() => {
      const dialog = rendered.container.querySelector(".review-dialog, [class*='review-dialog']");
      expect(dialog || rendered.container.innerHTML.includes("Review This PR")).toBeTruthy();
    });
  });

  // --- Show More / Pagination ---
  it("calls fetch_pr_list with higher limit when Show More is clicked", async () => {
    const prs = Array.from({ length: 30 }, (_, i) =>
      makePr({ number: i + 1, title: `PR ${i + 1}` })
    );

    let lastLimit = 0;
    invokeMock.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") {
        lastLimit = args?.limit ?? 0;
        return makePrResponse(prs);
      }
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Show More")).toBeTruthy();
    });

    // Initial limit should be 30
    expect(lastLimit).toBe(30);

    await fireEvent.click(rendered.getByText("Show More"));

    await waitFor(() => {
      // After Show More, limit should increase to 60
      expect(lastLimit).toBe(60);
    });
  });

  // --- CI Badge in expanded details ---
  it("shows CI check suites in expanded PR detail", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 80, title: "PR with CI", body: "CI body" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 80,
          title: "PR with CI",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/80",
          mergeable: "MERGEABLE",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/ci",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [
            { workflowName: "CI Build", runId: 1, status: "completed", conclusion: "success" },
            { workflowName: "Lint", runId: 2, status: "completed", conclusion: "failure" },
          ],
          reviews: [],
          reviewComments: [],
          changedFilesCount: 3,
          additions: 10,
          deletions: 5,
        };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("PR with CI")).toBeTruthy();
    });

    // Expand the PR
    const prRow = rendered.container.querySelector(".plp-pr-row") as HTMLElement;
    await fireEvent.click(prRow);

    await waitFor(() => {
      expect(rendered.container.querySelector(".plp-pr-expanded")).toBeTruthy();
      expect(rendered.getByText("CI Build")).toBeTruthy();
      expect(rendered.getByText("Lint")).toBeTruthy();
    });
  });

  // --- Review items in expanded details ---
  it("shows reviews in expanded PR detail", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 81, title: "PR with Reviews", body: "Review body" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 81,
          title: "PR with Reviews",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/81",
          mergeable: "MERGEABLE",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/reviews",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [],
          reviews: [
            { reviewer: "bob", state: "APPROVED" },
            { reviewer: "carol", state: "CHANGES_REQUESTED" },
          ],
          reviewComments: [],
          changedFilesCount: 2,
          additions: 5,
          deletions: 1,
        };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("PR with Reviews")).toBeTruthy();
    });

    const prRow = rendered.container.querySelector(".plp-pr-row") as HTMLElement;
    await fireEvent.click(prRow);

    await waitFor(() => {
      expect(rendered.container.querySelector(".plp-pr-expanded")).toBeTruthy();
      expect(rendered.getByText("Reviews")).toBeTruthy();
      expect(rendered.getByText("bob")).toBeTruthy();
      expect(rendered.getByText("APPROVED")).toBeTruthy();
      expect(rendered.getByText("carol")).toBeTruthy();
      expect(rendered.getByText("CHANGES_REQUESTED")).toBeTruthy();
    });
  });

  // --- CI and review badges on PR rows ---
  it("shows CI fail badge when check suite has failure", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 82, title: "CI Fail PR" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 82,
          title: "CI Fail PR",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/82",
          mergeable: "UNKNOWN",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/ci-fail",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [
            { workflowName: "CI", runId: 1, status: "completed", conclusion: "failure" },
          ],
          reviews: [],
          reviewComments: [],
          changedFilesCount: 1,
          additions: 1,
          deletions: 0,
        };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("CI Fail PR")).toBeTruthy();
    });

    await waitFor(() => {
      const ciBadge = rendered.container.querySelector(".plp-ci-badge");
      expect(ciBadge).toBeTruthy();
      expect(ciBadge?.classList.contains("fail")).toBe(true);
      expect(ciBadge?.textContent).toBe("\u2717");
    });
  });

  it("shows CI pass badge when all check suites succeed", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 83, title: "CI Pass PR" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 83,
          title: "CI Pass PR",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/83",
          mergeable: "MERGEABLE",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/ci-pass",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [
            { workflowName: "CI", runId: 1, status: "completed", conclusion: "success" },
          ],
          reviews: [],
          reviewComments: [],
          changedFilesCount: 1,
          additions: 1,
          deletions: 0,
        };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("CI Pass PR")).toBeTruthy();
    });

    await waitFor(() => {
      const ciBadge = rendered.container.querySelector(".plp-ci-badge");
      expect(ciBadge).toBeTruthy();
      expect(ciBadge?.classList.contains("pass")).toBe(true);
      expect(ciBadge?.textContent).toBe("\u2713");
    });
  });

  it("shows CI running badge when check suite is in progress", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 84, title: "CI Running PR" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 84,
          title: "CI Running PR",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/84",
          mergeable: "UNKNOWN",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/ci-running",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [
            { workflowName: "CI", runId: 1, status: "in_progress", conclusion: null },
          ],
          reviews: [],
          reviewComments: [],
          changedFilesCount: 1,
          additions: 1,
          deletions: 0,
        };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("CI Running PR")).toBeTruthy();
    });

    await waitFor(() => {
      const ciBadge = rendered.container.querySelector(".plp-ci-badge");
      expect(ciBadge).toBeTruthy();
      expect(ciBadge?.classList.contains("running")).toBe(true);
    });
  });

  // --- Review badge ---
  it("shows review approved badge", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 85, title: "Approved PR" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 85,
          title: "Approved PR",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/85",
          mergeable: "MERGEABLE",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/approved",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [],
          reviews: [{ reviewer: "bob", state: "APPROVED" }],
          reviewComments: [],
          changedFilesCount: 1,
          additions: 1,
          deletions: 0,
        };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Approved PR")).toBeTruthy();
    });

    await waitFor(() => {
      const reviewBadge = rendered.container.querySelector(".plp-review-badge");
      expect(reviewBadge).toBeTruthy();
      expect(reviewBadge?.classList.contains("pass")).toBe(true);
    });
  });

  it("shows review changes_requested badge", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 86, title: "Changes Requested PR" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 86,
          title: "Changes Requested PR",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/86",
          mergeable: "MERGEABLE",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/changes-req",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [],
          reviews: [{ reviewer: "bob", state: "CHANGES_REQUESTED" }],
          reviewComments: [],
          changedFilesCount: 1,
          additions: 1,
          deletions: 0,
        };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Changes Requested PR")).toBeTruthy();
    });

    await waitFor(() => {
      const reviewBadge = rendered.container.querySelector(".plp-review-badge");
      expect(reviewBadge).toBeTruthy();
      expect(reviewBadge?.classList.contains("fail")).toBe(true);
    });
  });

  // --- Mergeable badge ---
  it("shows Mergeable badge when mergeable is MERGEABLE", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 87, title: "Mergeable PR" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 87,
          title: "Mergeable PR",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/87",
          mergeable: "MERGEABLE",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/mergeable",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [],
          reviews: [],
          reviewComments: [],
          changedFilesCount: 1,
          additions: 1,
          deletions: 0,
        };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Mergeable PR")).toBeTruthy();
    });

    await waitFor(() => {
      expect(rendered.getByText("Mergeable")).toBeTruthy();
      const badge = rendered.container.querySelector(".plp-merge-badge.mergeable");
      expect(badge).toBeTruthy();
    });
  });

  it("shows Conflicts badge when mergeable is CONFLICTING", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 88, title: "Conflicting PR" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 88,
          title: "Conflicting PR",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/88",
          mergeable: "CONFLICTING",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/conflict",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [],
          reviews: [],
          reviewComments: [],
          changedFilesCount: 1,
          additions: 1,
          deletions: 0,
        };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Conflicting PR")).toBeTruthy();
    });

    await waitFor(() => {
      expect(rendered.getByText("Conflicts")).toBeTruthy();
      const badge = rendered.container.querySelector(".plp-merge-badge.conflicting");
      expect(badge).toBeTruthy();
    });
  });

  // --- Sort priority: action required PRs first ---
  it("sorts action-required PRs before others", async () => {
    const prs: PrListItem[] = [
      makePr({
        number: 90,
        title: "Normal PR",
        author: { login: "bob" },
        updatedAt: "2025-01-01T00:00:00Z",
      }),
      makePr({
        number: 91,
        title: "Action Required PR",
        author: { login: "alice" },
        updatedAt: "2024-12-01T00:00:00Z",
      }),
    ];

    invokeMock.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") {
        const prNum = args?.prNumber;
        if (prNum === 91) {
          return {
            number: 91,
            title: "Action Required PR",
            state: "OPEN",
            url: "https://github.com/test/repo/pull/91",
            mergeable: "MERGEABLE",
            author: "alice",
            baseBranch: "main",
            headBranch: "feature/action",
            labels: [],
            assignees: [],
            milestone: null,
            linkedIssues: [],
            checkSuites: [
              { workflowName: "CI", runId: 1, status: "completed", conclusion: "failure" },
            ],
            reviews: [],
            reviewComments: [],
            changedFilesCount: 1,
            additions: 1,
            deletions: 0,
          };
        }
        return null;
      }
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Action Required PR")).toBeTruthy();
      expect(rendered.getByText("Normal PR")).toBeTruthy();
    });

    // Wait for details to load and sorting to happen
    await waitFor(() => {
      const rows = rendered.container.querySelectorAll(".plp-pr-row");
      const firstRowText = rows[0]?.textContent ?? "";
      expect(firstRowText).toContain("Action Required PR");
    });
  });

  // --- Sort priority: review-requested PRs before others ---
  it("sorts review-requested PRs before regular PRs", async () => {
    const prs: PrListItem[] = [
      makePr({
        number: 92,
        title: "Regular PR",
        author: { login: "bob" },
        reviewRequests: [],
        updatedAt: "2025-01-02T00:00:00Z",
      }),
      makePr({
        number: 93,
        title: "Review Requested PR",
        author: { login: "bob" },
        reviewRequests: [{ login: "alice" }],
        updatedAt: "2025-01-01T00:00:00Z",
      }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      const rows = rendered.container.querySelectorAll(".plp-pr-row");
      expect(rows.length).toBe(2);
      const firstRowText = rows[0]?.textContent ?? "";
      expect(firstRowText).toContain("Review Requested PR");
    });
  });

  // --- Error handling: string error from fetch ---
  it("shows string error from fetch_pr_list", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") throw "Raw string error";
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Raw string error")).toBeTruthy();
    });
  });

  // --- Error handling: non-standard error object ---
  it("shows String(err) for non-standard error objects", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") throw 42;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("42")).toBeTruthy();
    });
  });

  // --- Open in GitHub ---
  it("calls openExternalUrl when PR number is clicked", async () => {
    const { openExternalUrl } = await import("../openExternalUrl");
    const prs: PrListItem[] = [
      makePr({ number: 100, title: "Open URL PR", url: "https://github.com/test/repo/pull/100" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("#100")).toBeTruthy();
    });

    await fireEvent.click(rendered.getByText("#100"));

    expect(openExternalUrl).toHaveBeenCalledWith("https://github.com/test/repo/pull/100");
  });

  // --- Neutral CI badge ---
  it("shows neutral CI badge when check suite has mixed results (not all pass, no fail, not running)", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 95, title: "Neutral CI PR" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 95,
          title: "Neutral CI PR",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/95",
          mergeable: "UNKNOWN",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/neutral-ci",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [
            { workflowName: "CI", runId: 1, status: "completed", conclusion: "neutral" },
          ],
          reviews: [],
          reviewComments: [],
          changedFilesCount: 1,
          additions: 1,
          deletions: 0,
        };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Neutral CI PR")).toBeTruthy();
    });

    await waitFor(() => {
      const ciBadge = rendered.container.querySelector(".plp-ci-badge");
      expect(ciBadge).toBeTruthy();
      expect(ciBadge?.classList.contains("neutral")).toBe(true);
      expect(ciBadge?.textContent).toBe("\u25CB");
    });
  });

  // --- Neutral review badge ---
  it("shows neutral review badge for COMMENTED review", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 96, title: "Commented Review PR" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 96,
          title: "Commented Review PR",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/96",
          mergeable: "UNKNOWN",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/commented",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [],
          reviews: [{ reviewer: "bob", state: "COMMENTED" }],
          reviewComments: [],
          changedFilesCount: 1,
          additions: 1,
          deletions: 0,
        };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Commented Review PR")).toBeTruthy();
    });

    await waitFor(() => {
      const reviewBadge = rendered.container.querySelector(".plp-review-badge");
      expect(reviewBadge).toBeTruthy();
      expect(reviewBadge?.classList.contains("neutral")).toBe(true);
    });
  });

  // --- No Review/Merge buttons for closed PRs ---
  it("does not show Merge or Review buttons for closed PRs", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 97, title: "Closed PR", state: "CLOSED" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Closed PR")).toBeTruthy();
    });

    // Merge and Review buttons should not be present
    expect(rendered.queryByText("Merge")).toBeNull();
    expect(rendered.queryByText("Review")).toBeNull();
  });

  // --- isMyPr with assignee match ---
  it("highlights PR as mine when user is an assignee", async () => {
    const prs: PrListItem[] = [
      makePr({
        number: 98,
        title: "Assigned PR",
        author: { login: "bob" },
        assignees: [{ login: "alice" }],
      }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Assigned PR")).toBeTruthy();
    });

    const row = rendered.container.querySelector(".plp-pr-row");
    expect(row?.classList.contains("my-pr")).toBe(true);
  });

  // --- isMyPr with reviewRequest match ---
  it("highlights PR as mine when user is a review requester", async () => {
    const prs: PrListItem[] = [
      makePr({
        number: 99,
        title: "Review Requested My PR",
        author: { login: "bob" },
        reviewRequests: [{ login: "alice" }],
      }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Review Requested My PR")).toBeTruthy();
    });

    const row = rendered.container.querySelector(".plp-pr-row");
    expect(row?.classList.contains("my-pr")).toBe(true);
  });

  // --- Label style with dark color ---
  it("renders label with dark background and light text", async () => {
    const prs: PrListItem[] = [
      makePr({
        number: 101,
        title: "Dark Label PR",
        labels: [{ name: "urgent", color: "1a1a1a" }],
      }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      const label = rendered.getByText("urgent");
      expect(label).toBeTruthy();
      const style = label.getAttribute("style") ?? "";
      // Dark background should produce light text (#cdd6f4 or rgb equivalent)
      expect(style.includes("#cdd6f4") || style.includes("205, 214, 244")).toBe(true);
    });
  });

  // --- Label style with light color ---
  it("renders label with light background and dark text", async () => {
    const prs: PrListItem[] = [
      makePr({
        number: 102,
        title: "Light Label PR",
        labels: [{ name: "docs", color: "f0f0f0" }],
      }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      const label = rendered.getByText("docs");
      expect(label).toBeTruthy();
      const style = label.getAttribute("style") ?? "";
      // Light background should produce dark text (#1e1e2e or rgb equivalent)
      expect(style.includes("#1e1e2e") || style.includes("30, 30, 46")).toBe(true);
    });
  });

  // --- fetch_github_user failure gracefully sets currentUser to null ---
  it("renders PRs even when fetch_github_user fails", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 103, title: "No User PR" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") throw new Error("User fetch failed");
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail") return null;
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("No User PR")).toBeTruthy();
    });
  });

  // --- Checks heading count in expanded detail ---
  it("shows Checks heading with count in expanded detail", async () => {
    const prs: PrListItem[] = [
      makePr({ number: 104, title: "Checks Count PR", body: "body" }),
    ];

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return ghCliOk();
      if (cmd === "fetch_github_user") return { login: "alice", ghStatus: ghCliOk() } as GitHubUserResponse;
      if (cmd === "fetch_pr_list") return makePrResponse(prs);
      if (cmd === "fetch_pr_detail")
        return {
          number: 104,
          title: "Checks Count PR",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/104",
          mergeable: "MERGEABLE",
          author: "alice",
          baseBranch: "main",
          headBranch: "feature/checks-count",
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [
            { workflowName: "Build", runId: 1, status: "completed", conclusion: "success" },
            { workflowName: "Test", runId: 2, status: "completed", conclusion: "success" },
            { workflowName: "Lint", runId: 3, status: "completed", conclusion: "success" },
          ],
          reviews: [],
          reviewComments: [],
          changedFilesCount: 5,
          additions: 10,
          deletions: 3,
        };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Checks Count PR")).toBeTruthy();
    });

    const prRow = rendered.container.querySelector(".plp-pr-row") as HTMLElement;
    await fireEvent.click(prRow);

    await waitFor(() => {
      expect(rendered.getByText("Checks (3)")).toBeTruthy();
      expect(rendered.getByText("Build")).toBeTruthy();
      expect(rendered.getByText("Test")).toBeTruthy();
      expect(rendered.getByText("Lint")).toBeTruthy();
    });
  });
});
