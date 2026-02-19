import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

import type { CommitEntry } from "../types";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
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
