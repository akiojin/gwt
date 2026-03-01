import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, cleanup } from "@testing-library/svelte";

import type { StashEntry } from "../types";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
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

  it("shows singular 'file' label when stash has exactly 1 file", async () => {
    // Exercises the ternary true branch: file_count === 1 ? "" : "s"
    invokeMock.mockResolvedValue([
      { index: 0, message: "single file stash", file_count: 1 },
    ]);

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("(1 file)")).toBeTruthy();
    });
  });

  it("shows plural 'files' label when stash has multiple files", async () => {
    // Exercises the ternary false branch: file_count !== 1 → "s"
    invokeMock.mockResolvedValue([
      { index: 0, message: "multi file stash", file_count: 2 },
    ]);

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("(2 files)")).toBeTruthy();
    });
  });

  it("shows error using String(err) when error is a non-Error-object without string message", async () => {
    // Exercises toErrorMessage line 24: return String(err)
    // An object that has a message property but its value is not a string
    invokeMock.mockRejectedValue({ message: 42, code: "ERR_UNKNOWN" });

    const rendered = await renderTab();

    await waitFor(() => {
      const errEl = rendered.container.querySelector(".stash-error");
      expect(errEl).toBeTruthy();
      expect(errEl?.textContent).toContain("[object Object]");
    });
  });

  it("shows error using String(err) when error is a number", async () => {
    // Exercises toErrorMessage line 24: return String(err) for non-object, non-string
    invokeMock.mockRejectedValue(404);

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("404")).toBeTruthy();
    });
  });

  it("shows error text when error is a plain string", async () => {
    // Exercises toErrorMessage line 19: typeof err === "string" → return err
    invokeMock.mockRejectedValue("plain string error");

    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("plain string error")).toBeTruthy();
    });
  });
});
