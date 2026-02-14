import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

import type { CommitEntry, GitChangeSummary, StashEntry } from "../types";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  default: {
    invoke: invokeMock,
  },
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
});
