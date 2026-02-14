import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, cleanup } from "@testing-library/svelte";

import type { StashEntry } from "../types";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

const stashFixture: StashEntry[] = [
  { index: 0, message: "WIP on feature branch", file_count: 1 },
  { index: 1, message: "fix: bug", file_count: 3 },
];

async function renderTab(overrides: Record<string, unknown> = {}) {
  const { default: GitStashTab } = await import("./GitStashTab.svelte");
  return render(GitStashTab, {
    props: {
      projectPath: "/tmp/project",
      branch: "feature/stash",
      refreshToken: 0,
      ...overrides,
    },
  });
}

describe("GitStashTab", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
  });

  it("renders stash entries", async () => {
    invokeMock.mockResolvedValue(stashFixture);

    const rendered = await renderTab();
    await waitFor(() => {
      expect(rendered.getByText("stash@{0}:")).toBeTruthy();
    });
    expect(rendered.getByText("WIP on feature branch")).toBeTruthy();
    expect(rendered.getByText("stash@{1}:")).toBeTruthy();
    expect(rendered.getByText("(3 files)")).toBeTruthy();
  });

  it("shows an empty state when no stash exists", async () => {
    invokeMock.mockResolvedValue([]);
    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("No stash entries")).toBeTruthy();
    });
  });

  it("shows error text when invoke fails", async () => {
    invokeMock.mockRejectedValue(new Error("Failed to load stash"));

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("Failed to load stash")).toBeTruthy();
    });
  });
});
