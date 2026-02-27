import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

import type { CommitEntry, GitChangeSummary, StashEntry } from "../types";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
}));

async function renderSection() {
  const { default: GitSection } = await import("./GitSection.svelte");
  return render(GitSection, {
    props: {
      projectPath: "/tmp/project",
      branch: "feature/git",
      collapsible: false,
    },
  });
}

describe("GitSection", () => {
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

  it("loads and renders git summary with base branch selector", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_git_change_summary") {
        return {
          file_count: 2,
          commit_count: 1,
          stash_count: 0,
          base_branch: "main",
        } as GitChangeSummary;
      }
      if (command === "get_base_branch_candidates") return ["main", "develop"];
      if (command === "get_branch_diff_files") return [];
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByText("2 files, 1 commit")).toBeTruthy();
      expect(rendered.getByText("Base:")).toBeTruthy();
    });

    const select = rendered.container.querySelector("#base-branch-select") as HTMLSelectElement;
    expect(select.value).toBe("main");
  });

  it("switches to commits tab and renders commit rows", async () => {
    const commits: CommitEntry[] = [
      {
        sha: "abc1234",
        message: "feat: update UI",
        timestamp: Math.floor(Date.now() / 1000),
        author: "alice",
      },
    ];

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_git_change_summary") {
        return {
          file_count: 1,
          commit_count: 1,
          stash_count: 0,
          base_branch: "main",
        } as GitChangeSummary;
      }
      if (command === "get_base_branch_candidates") return ["main"];
      if (command === "get_branch_diff_files") return [];
      if (command === "get_branch_commits") return commits;
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByRole("button", { name: "Commits" })).toBeTruthy();
    });
    await fireEvent.click(rendered.getByRole("button", { name: "Commits" }));
    await waitFor(() => {
      expect(rendered.getByText("feat: update UI")).toBeTruthy();
    });
  });

  it("shows stash tab when stash entries exist", async () => {
    const stashEntries: StashEntry[] = [{ index: 0, message: "WIP stash", file_count: 2 }];

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_git_change_summary") {
        return {
          file_count: 1,
          commit_count: 1,
          stash_count: 1,
          base_branch: "main",
        } as GitChangeSummary;
      }
      if (command === "get_base_branch_candidates") return ["main"];
      if (command === "get_branch_diff_files") return [];
      if (command === "get_stash_list") return stashEntries;
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByRole("button", { name: "Stash" })).toBeTruthy();
    });
    await fireEvent.click(rendered.getByRole("button", { name: "Stash" }));

    await waitFor(() => {
      expect(rendered.getByText("stash@{0}:")).toBeTruthy();
      expect(rendered.getByText("WIP stash")).toBeTruthy();
    });
  });

  it("renders error state when summary fetch fails", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_git_change_summary") throw new Error("git summary failed");
      if (command === "get_base_branch_candidates") return ["main"];
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByText("git summary failed")).toBeTruthy();
    });
  });

  it("refresh button triggers reload", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_git_change_summary") {
        return {
          file_count: 0,
          commit_count: 0,
          stash_count: 0,
          base_branch: "main",
        } as GitChangeSummary;
      }
      if (command === "get_base_branch_candidates") return ["main"];
      if (command === "get_branch_diff_files") return [];
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter((c) => c[0] === "get_git_change_summary").length
      ).toBe(1);
    });

    await fireEvent.click(rendered.getByTitle("Refresh"));
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter((c) => c[0] === "get_git_change_summary").length
      ).toBe(2);
    });
  });

  it("re-fetches summary when base branch changes", async () => {
    invokeMock.mockImplementation(async (command: string, args?: { baseBranch?: string }) => {
      if (command === "get_base_branch_candidates") return ["main", "develop"];
      if (command === "get_branch_diff_files") return [];
      if (command === "get_git_change_summary") {
        if (args?.baseBranch === "develop") {
          return {
            file_count: 4,
            commit_count: 2,
            stash_count: 0,
            base_branch: "develop",
          } as GitChangeSummary;
        }
        return {
          file_count: 1,
          commit_count: 1,
          stash_count: 0,
          base_branch: "main",
        } as GitChangeSummary;
      }
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByText("1 file, 1 commit")).toBeTruthy();
    });

    const select = rendered.container.querySelector("#base-branch-select") as HTMLSelectElement;
    select.value = "develop";
    await fireEvent.change(select);

    await waitFor(() => {
      expect(rendered.getByText("4 files, 2 commits")).toBeTruthy();
    });
  });

  it("refresh button invokes summary fetch again and updates data", async () => {
    let callCount = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_base_branch_candidates") return ["main"];
      if (command === "get_branch_diff_files") return [];
      if (command === "get_git_change_summary") {
        callCount++;
        if (callCount > 1) {
          return {
            file_count: 3,
            commit_count: 2,
            stash_count: 0,
            base_branch: "main",
          } as GitChangeSummary;
        }
        return {
          file_count: 1,
          commit_count: 1,
          stash_count: 0,
          base_branch: "main",
        } as GitChangeSummary;
      }
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByText("1 file, 1 commit")).toBeTruthy();
    });

    // Trigger refresh
    await fireEvent.click(rendered.getByTitle("Refresh"));

    await waitFor(() => {
      expect(rendered.getByText("3 files, 2 commits")).toBeTruthy();
    });
  });

  it("shows stash count in summary text", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_git_change_summary") {
        return {
          file_count: 5,
          commit_count: 3,
          stash_count: 2,
          base_branch: "main",
        } as GitChangeSummary;
      }
      if (command === "get_base_branch_candidates") return ["main"];
      if (command === "get_branch_diff_files") return [];
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByText("5 files, 3 commits, 2 stash")).toBeTruthy();
    });
  });

  it("handles base branch change to invalid candidate gracefully", async () => {
    invokeMock.mockImplementation(async (command: string, args?: { baseBranch?: string }) => {
      if (command === "get_base_branch_candidates") return ["main", "develop"];
      if (command === "get_branch_diff_files") return [];
      if (command === "get_git_change_summary") {
        return {
          file_count: 1,
          commit_count: 1,
          stash_count: 0,
          base_branch: args?.baseBranch || "main",
        } as GitChangeSummary;
      }
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByText("1 file, 1 commit")).toBeTruthy();
    });

    // Verify selecting same base branch doesn't re-fetch
    const select = rendered.container.querySelector("#base-branch-select") as HTMLSelectElement;
    const callsBefore = invokeMock.mock.calls.filter((c) => c[0] === "get_git_change_summary").length;
    select.value = "main";
    await fireEvent.change(select);

    // Should not increase call count since the value didn't change
    expect(
      invokeMock.mock.calls.filter((c) => c[0] === "get_git_change_summary").length
    ).toBe(callsBefore);
  });

  it("handles collapsible mode with header click", async () => {
    const { default: GitSection } = await import("./GitSection.svelte");
    const rendered = render(GitSection, {
      props: {
        projectPath: "/tmp/project",
        branch: "feature/git",
        collapsible: true,
        defaultCollapsed: true,
      },
    });

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_git_change_summary") {
        return {
          file_count: 2,
          commit_count: 1,
          stash_count: 0,
          base_branch: "main",
        } as GitChangeSummary;
      }
      if (command === "get_base_branch_candidates") return ["main"];
      if (command === "get_branch_diff_files") return [];
      return [];
    });

    // Initially collapsed - no body
    expect(rendered.container.querySelector(".git-body")).toBeNull();
    expect(rendered.container.querySelector(".collapse-icon")).toBeTruthy();

    // Click header to expand
    const header = rendered.container.querySelector(".git-header") as HTMLElement;
    await fireEvent.click(header);

    await waitFor(() => {
      expect(rendered.container.querySelector(".git-body")).toBeTruthy();
    });

    // Click again to collapse
    await fireEvent.click(header);
    expect(rendered.container.querySelector(".git-body")).toBeNull();
  });

  it("uses summaryBase when candidates array is empty", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_git_change_summary") {
        return {
          file_count: 2,
          commit_count: 1,
          stash_count: 0,
          base_branch: "main",
        } as GitChangeSummary;
      }
      if (command === "get_base_branch_candidates") return [];
      if (command === "get_branch_diff_files") return [];
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByText("2 files, 1 commit")).toBeTruthy();
    });

    // No base-branch-select should be shown when candidates are empty
    expect(rendered.container.querySelector("#base-branch-select")).toBeNull();
  });

  it("falls back to summaryBase when baseBranch is not in candidates", async () => {
    let callCount = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_git_change_summary") {
        callCount++;
        // First call returns summary with base_branch "develop"
        // Second call (after baseBranch updated) still returns develop
        return {
          file_count: callCount,
          commit_count: 1,
          stash_count: 0,
          base_branch: "develop",
        } as GitChangeSummary;
      }
      if (command === "get_base_branch_candidates") return ["develop", "staging"];
      if (command === "get_branch_diff_files") return [];
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByText("1 file, 1 commit")).toBeTruthy();
    });

    // baseBranch should be set to "develop" (from summaryBase since it's in candidates)
    const select = rendered.container.querySelector("#base-branch-select") as HTMLSelectElement;
    expect(select.value).toBe("develop");
  });

  it("falls back to first candidate when summaryBase is not in candidates", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_git_change_summary") {
        return {
          file_count: 3,
          commit_count: 2,
          stash_count: 0,
          base_branch: "obsolete-branch",
        } as GitChangeSummary;
      }
      if (command === "get_base_branch_candidates") return ["main", "develop"];
      if (command === "get_branch_diff_files") return [];
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByText("3 files, 2 commits")).toBeTruthy();
    });

    // baseBranch should fall back to first candidate "main" since summaryBase isn't in candidates
    const select = rendered.container.querySelector("#base-branch-select") as HTMLSelectElement;
    expect(select.value).toBe("main");
  });

  it("handles base branch change to invalid candidate with fallback", async () => {
    invokeMock.mockImplementation(async (command: string, args?: { baseBranch?: string }) => {
      if (command === "get_base_branch_candidates") return ["main", "develop"];
      if (command === "get_branch_diff_files") return [];
      if (command === "get_git_change_summary") {
        return {
          file_count: 1,
          commit_count: 1,
          stash_count: 0,
          base_branch: args?.baseBranch || "main",
        } as GitChangeSummary;
      }
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByText("1 file, 1 commit")).toBeTruthy();
    });

    // Add a temporary option to let the DOM accept the invalid value
    const select = rendered.container.querySelector("#base-branch-select") as HTMLSelectElement;
    const tempOption = document.createElement("option");
    tempOption.value = "nonexistent";
    tempOption.textContent = "nonexistent";
    select.appendChild(tempOption);
    select.value = "nonexistent";
    await fireEvent.change(select);

    // baseBranch should fall back to current baseBranch (main) since nonexistent is not in candidates
    // The summary call count should not increase because the change was rejected
    const callsAfter = invokeMock.mock.calls.filter((c) => c[0] === "get_git_change_summary").length;
    expect(callsAfter).toBe(1); // No additional call
  });

  it("handles empty base branch change event", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_base_branch_candidates") return ["main"];
      if (command === "get_branch_diff_files") return [];
      if (command === "get_git_change_summary") {
        return {
          file_count: 1,
          commit_count: 1,
          stash_count: 0,
          base_branch: "main",
        } as GitChangeSummary;
      }
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByText("1 file, 1 commit")).toBeTruthy();
    });

    const select = rendered.container.querySelector("#base-branch-select") as HTMLSelectElement;
    const callsBefore = invokeMock.mock.calls.filter((c) => c[0] === "get_git_change_summary").length;

    // Set to empty value and trigger change
    select.value = "";
    await fireEvent.change(select);

    // Should not re-fetch when nextBase is empty
    expect(
      invokeMock.mock.calls.filter((c) => c[0] === "get_git_change_summary").length
    ).toBe(callsBefore);
  });

  it("handles string error in toErrorMessage", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_git_change_summary") throw "plain string error";
      if (command === "get_base_branch_candidates") throw "plain string error";
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      expect(rendered.getByText("plain string error")).toBeTruthy();
    });
  });

  it("handles non-standard error in toErrorMessage", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_git_change_summary") throw { code: 500 };
      if (command === "get_base_branch_candidates") throw { code: 500 };
      return [];
    });

    const rendered = await renderSection();

    await waitFor(() => {
      const errorEl = rendered.container.querySelector(".git-error");
      expect(errorEl).toBeTruthy();
    });
  });

  it("ignores stale summary failures after base branch changes again", async () => {
    let rejectDevelopSummary: ((reason?: unknown) => void) | null = null;

    invokeMock.mockImplementation((command: string, args?: { baseBranch?: string }) => {
      if (command === "get_base_branch_candidates") return Promise.resolve(["main", "develop"]);
      if (command === "get_branch_diff_files") return Promise.resolve([]);
      if (command === "get_git_change_summary") {
        if (!args?.baseBranch) {
          return Promise.resolve({
            file_count: 1,
            commit_count: 1,
            stash_count: 0,
            base_branch: "main",
          } as GitChangeSummary);
        }
        if (args.baseBranch === "develop") {
          return new Promise<GitChangeSummary>((_resolve, reject) => {
            rejectDevelopSummary = reject;
          });
        }
        return Promise.resolve({
          file_count: 9,
          commit_count: 3,
          stash_count: 0,
          base_branch: "main",
        } as GitChangeSummary);
      }
      return Promise.resolve([]);
    });

    const rendered = await renderSection();
    await waitFor(() => {
      expect(rendered.getByText("1 file, 1 commit")).toBeTruthy();
    });

    const select = rendered.container.querySelector("#base-branch-select") as HTMLSelectElement;
    select.value = "develop";
    await fireEvent.change(select);
    select.value = "main";
    await fireEvent.change(select);

    await waitFor(() => {
      expect(rendered.getByText("9 files, 3 commits")).toBeTruthy();
    });

    const rejectDevelopSummaryNow = rejectDevelopSummary as ((reason?: unknown) => void) | null;
    expect(rejectDevelopSummaryNow).toBeTruthy();
    rejectDevelopSummaryNow?.(new Error("stale develop failure"));
    await Promise.resolve();

    await waitFor(() => {
      expect(rendered.queryByText("stale develop failure")).toBeNull();
      expect(rendered.getByText("9 files, 3 commits")).toBeTruthy();
    });
  });
});
