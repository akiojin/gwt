import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

import type { CommitEntry } from "../types";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
}));

const makeCommit = (index: number): CommitEntry => ({
  sha: `abcde${String(index).padStart(4, "0")}ff`,
  message: `Commit message ${index + 1}`,
  timestamp: Math.floor((Date.now() - index * 90_000) / 1000),
  author: "tester",
});

async function renderTab(overrides: Record<string, unknown> = {}) {
  const { default: GitCommitsTab } = await import("./GitCommitsTab.svelte");
  return render(GitCommitsTab, {
    props: {
      projectPath: "/tmp/project",
      branch: "feature/commits",
      baseBranch: "main",
      refreshToken: 0,
      ...overrides,
    },
  });
}

describe("GitCommitsTab", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
    Object.defineProperty(globalThis, "__TAURI_INTERNALS__", {
      value: { invoke: invokeMock },
      configurable: true,
    });
  });

  afterEach(() => {
    delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it("renders commit list and loads more pages", async () => {
    invokeMock.mockImplementation(async (command: string, args: { offset?: number }) => {
      if (command !== "get_branch_commits") return [];
      if ((args?.offset ?? 0) === 0) {
        return Array.from({ length: 20 }, (_, i) => makeCommit(i));
      }
      if (args?.offset === 20) {
        return [makeCommit(20)];
      }
      return [];
    });

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".commit-row")).toHaveLength(20);
    });
    expect(rendered.getByText("Commit message 1")).toBeTruthy();

    const showMore = rendered.getByRole("button", { name: "Show more" });
    await fireEvent.click(showMore);

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".commit-row")).toHaveLength(21);
    });
    expect(rendered.queryByRole("button", { name: "Show more" })).toBeNull();
  });

  it("shows a loading message while fetching", async () => {
    invokeMock.mockImplementation(async () => {
      await new Promise((resolve) => setTimeout(resolve, 30));
      return [];
    });

    const rendered = await renderTab();

    expect(rendered.getByText("Loading...")).toBeTruthy();
    await waitFor(() => {
      expect(rendered.getByText("No commits")).toBeTruthy();
    });
  });

  it("shows empty state when no commits exist", async () => {
    invokeMock.mockResolvedValue([]);
    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("No commits")).toBeTruthy();
    });
    expect(rendered.queryByRole("button", { name: "Show more" })).toBeNull();
  });

  it("shows error text when invoke fails", async () => {
    invokeMock.mockRejectedValue(new Error("Failed to load commits"));

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("Failed to load commits")).toBeTruthy();
    });
  });

  it("shows error from loadMore when invoke fails", async () => {
    let loadMoreCallCount = 0;
    invokeMock.mockImplementation(async (command: string, args: { offset?: number }) => {
      if (command !== "get_branch_commits") return [];
      if ((args?.offset ?? 0) === 0) {
        return Array.from({ length: 20 }, (_, i) => makeCommit(i));
      }
      loadMoreCallCount++;
      throw new Error("load more failed");
    });

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".commit-row")).toHaveLength(20);
    });

    const showMore = rendered.getByRole("button", { name: "Show more" });
    await fireEvent.click(showMore);

    await waitFor(() => {
      expect(rendered.getByText("load more failed")).toBeTruthy();
    });
  });

  it("handles string error in toErrorMessage", async () => {
    invokeMock.mockImplementation(async () => {
      throw "plain string error";
    });

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("plain string error")).toBeTruthy();
    });
  });

  it("handles non-standard error object in toErrorMessage", async () => {
    invokeMock.mockImplementation(async () => {
      throw 42;
    });

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("42")).toBeTruthy();
    });
  });

  it("re-fetches commits when refreshToken changes", async () => {
    invokeMock.mockResolvedValue([makeCommit(0)]);

    const rendered = await renderTab({ refreshToken: 0 });

    await waitFor(() => {
      expect(rendered.getByText("Commit message 1")).toBeTruthy();
    });

    const callsBefore = invokeMock.mock.calls.filter((c) => c[0] === "get_branch_commits").length;

    await rendered.rerender({
      projectPath: "/tmp/project",
      branch: "feature/commits",
      baseBranch: "main",
      refreshToken: 1,
    });

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter((c) => c[0] === "get_branch_commits").length
      ).toBeGreaterThan(callsBefore);
    });
  });

  it("handles null result from invoke gracefully", async () => {
    invokeMock.mockResolvedValue(null);

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("No commits")).toBeTruthy();
    });
  });

  it("displays singular time units (1 minute, 1 hour, 1 day, 1 week)", async () => {
    const now = Math.floor(Date.now() / 1000);
    const commits: CommitEntry[] = [
      { sha: "aaa0001aaa", message: "One minute ago", timestamp: now - 60, author: "tester" },
      { sha: "bbb0001bbb", message: "One hour ago", timestamp: now - 3600, author: "tester" },
      { sha: "ccc0001ccc", message: "One day ago", timestamp: now - 86400, author: "tester" },
      { sha: "ddd0001ddd", message: "One week ago", timestamp: now - 604800, author: "tester" },
    ];
    invokeMock.mockResolvedValue(commits);

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("One minute ago")).toBeTruthy();
    });

    // Check singular forms are displayed
    const timeTexts = Array.from(rendered.container.querySelectorAll(".commit-time"))
      .map((el) => el.textContent?.trim());

    expect(timeTexts).toContain("1 minute ago");
    expect(timeTexts).toContain("1 hour ago");
    expect(timeTexts).toContain("1 day ago");
    expect(timeTexts).toContain("1 week ago");
  });

  it("displays plural time units (2 minutes, 3 hours, 4 days, 2 weeks)", async () => {
    const now = Math.floor(Date.now() / 1000);
    const commits: CommitEntry[] = [
      { sha: "eee0001eee", message: "Two min ago", timestamp: now - 120, author: "tester" },
      { sha: "fff0001fff", message: "Three hr ago", timestamp: now - 10800, author: "tester" },
      { sha: "ggg0001ggg", message: "Four days ago", timestamp: now - 345600, author: "tester" },
      { sha: "hhh0001hhh", message: "Two weeks ago", timestamp: now - 1209600, author: "tester" },
    ];
    invokeMock.mockResolvedValue(commits);

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("Two min ago")).toBeTruthy();
    });

    const timeTexts = Array.from(rendered.container.querySelectorAll(".commit-time"))
      .map((el) => el.textContent?.trim());

    expect(timeTexts).toContain("2 minutes ago");
    expect(timeTexts).toContain("3 hours ago");
    expect(timeTexts).toContain("4 days ago");
    expect(timeTexts).toContain("2 weeks ago");
  });

  it("displays month-level relative time (singular and plural)", async () => {
    const now = Math.floor(Date.now() / 1000);
    const commits: CommitEntry[] = [
      { sha: "iii0001iii", message: "One month ago", timestamp: now - (35 * 86400), author: "tester" },
      { sha: "jjj0001jjj", message: "Three months ago", timestamp: now - (95 * 86400), author: "tester" },
    ];
    invokeMock.mockResolvedValue(commits);

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("One month ago")).toBeTruthy();
    });

    const timeTexts = Array.from(rendered.container.querySelectorAll(".commit-time"))
      .map((el) => el.textContent?.trim());

    expect(timeTexts).toContain("1 month ago");
    expect(timeTexts).toContain("3 months ago");
  });

  it("displays 'just now' for very recent commits", async () => {
    const now = Math.floor(Date.now() / 1000);
    const commits: CommitEntry[] = [
      { sha: "kkk0001kkk", message: "Just happened", timestamp: now - 10, author: "tester" },
    ];
    invokeMock.mockResolvedValue(commits);

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("Just happened")).toBeTruthy();
    });

    const timeTexts = Array.from(rendered.container.querySelectorAll(".commit-time"))
      .map((el) => el.textContent?.trim());

    expect(timeTexts).toContain("just now");
  });

  it("ignores stale commit response after base branch changes again", async () => {
    let resolveDevelop: ((value: CommitEntry[]) => void) | null = null;

    invokeMock.mockImplementation((command: string, args?: { baseBranch?: string }) => {
      if (command !== "get_branch_commits") return Promise.resolve([]);
      if (args?.baseBranch === "develop") {
        return new Promise<CommitEntry[]>((resolve) => {
          resolveDevelop = resolve;
        });
      }
      if (args?.baseBranch === "main") {
        return Promise.resolve([
          {
            sha: "main0001",
            message: "Main latest commit",
            timestamp: Math.floor(Date.now() / 1000),
            author: "tester",
          },
        ]);
      }
      return Promise.resolve([]);
    });

    const rendered = await renderTab({ baseBranch: "develop" });
    await waitFor(() => {
      expect(resolveDevelop).toBeTruthy();
    });

    await rendered.rerender({
      projectPath: "/tmp/project",
      branch: "feature/commits",
      baseBranch: "main",
      refreshToken: 0,
    });

    await waitFor(() => {
      expect(rendered.getByText("Main latest commit")).toBeTruthy();
    });

    const resolveDevelopNow = resolveDevelop as ((value: CommitEntry[]) => void) | null;
    if (!resolveDevelopNow) {
      throw new Error("Expected pending develop response");
    }
    resolveDevelopNow([
      {
        sha: "dev00001",
        message: "Develop stale commit",
        timestamp: Math.floor(Date.now() / 1000),
        author: "tester",
      },
    ]);
    await Promise.resolve();

    await waitFor(() => {
      expect(rendered.queryByText("Develop stale commit")).toBeNull();
      expect(rendered.getByText("Main latest commit")).toBeTruthy();
    });
  });
});
