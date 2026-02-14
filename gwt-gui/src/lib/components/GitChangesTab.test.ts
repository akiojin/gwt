import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

import type { FileChange, FileDiff, WorkingTreeEntry } from "../types";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  default: {
    invoke: invokeMock,
  },
}));

const fileChanges: FileChange[] = [
  {
    path: "src/index.ts",
    kind: "Added",
    additions: 12,
    deletions: 0,
    is_binary: false,
  },
  {
    path: "src/utils/helper.ts",
    kind: "Modified",
    additions: 3,
    deletions: 2,
    is_binary: false,
  },
  {
    path: "assets/icon.bin",
    kind: "Modified",
    additions: 0,
    deletions: 0,
    is_binary: true,
  },
];

const workingTree: WorkingTreeEntry[] = [
  { path: "src/staged.ts", status: "Added", is_staged: true },
  { path: "src/uncommitted.ts", status: "Deleted", is_staged: false },
];

const diffFixture: FileDiff = {
  content: "+added\n-context",
  truncated: true,
};

async function renderTab(overrides: Record<string, unknown> = {}) {
  const { default: GitChangesTab } = await import("./GitChangesTab.svelte");
  return render(GitChangesTab, {
    props: {
      projectPath: "/tmp/project",
      branch: "feature/changes",
      baseBranch: "main",
      refreshToken: 0,
      ...overrides,
    },
  });
}

describe("GitChangesTab", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
    Object.defineProperty(globalThis, "__TAURI_INTERNALS__", {
      value: { invoke: invokeMock },
      configurable: true,
    });
  });

  it("renders directory tree and shows loading diff on row expand", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") return fileChanges;
      if (command === "get_file_diff") return diffFixture;
      return [];
    });

    const rendered = await renderTab();
    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".dir-row").length).toBeGreaterThanOrEqual(3);
      expect(rendered.getByText("index.ts")).toBeTruthy();
      expect(rendered.getByText("helper.ts")).toBeTruthy();
    });

    const fileRow = rendered.getByText("index.ts").closest(".file-row") as HTMLElement;
    await fireEvent.click(fileRow);

    await waitFor(() => {
      expect(rendered.getByText("Loading diff...")).toBeTruthy();
    });
    await waitFor(() => {
      expect(rendered.getByText("+added")).toBeTruthy();
      expect(rendered.getByText("-context")).toBeTruthy();
      expect(rendered.getByText("Too large to display")).toBeTruthy();
    });

    await fireEvent.click(fileRow);
    await waitFor(() => {
      expect(rendered.queryByText("Loading diff...")).toBeNull();
    });
  });

  it("does not fetch file diff for binary files", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") return fileChanges;
      if (command === "get_file_diff") return diffFixture;
      return [];
    });

    const rendered = await renderTab();
    await waitFor(() => {
      expect(rendered.getByText("Binary file changed")).toBeTruthy();
    });

    const binaryRow = rendered.getByText("icon.bin").closest(".file-row") as HTMLElement;
    await fireEvent.click(binaryRow);

    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(rendered.getByText("Binary file changed")).toBeTruthy();
  });

  it("shows committed empty state", async () => {
    invokeMock.mockResolvedValue([]);
    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("No changes")).toBeTruthy();
    });
  });

  it("shows committed error state", async () => {
    invokeMock.mockRejectedValue(new Error("Failed to load changes"));
    const rendered = await renderTab();

    await waitFor(() => {
      expect(rendered.getByText("Failed to load changes")).toBeTruthy();
    });
  });

  it("switches to uncommitted and shows staged/unstaged files", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") return [];
      if (command === "get_working_tree_status") return workingTree;
      return [];
    });

    const rendered = await renderTab();
    await fireEvent.click(rendered.getByRole("button", { name: "Uncommitted" }));

    await waitFor(() => {
      expect(rendered.getByText("Staged")).toBeTruthy();
      expect(rendered.getByText("Unstaged")).toBeTruthy();
      expect(rendered.getByText("src/staged.ts")).toBeTruthy();
      expect(rendered.getByText("src/uncommitted.ts")).toBeTruthy();
    });
  });

  it("shows uncommitted empty state", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") return [];
      if (command === "get_working_tree_status") return [];
      return [];
    });

    const rendered = await renderTab();
    await fireEvent.click(rendered.getByRole("button", { name: "Uncommitted" }));

    await waitFor(() => {
      expect(rendered.getByText("No uncommitted changes")).toBeTruthy();
    });
  });

  it("shows uncommitted error state", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") return [];
      if (command === "get_working_tree_status") throw new Error("Failed to read working tree");
      return [];
    });

    const rendered = await renderTab();
    await fireEvent.click(rendered.getByRole("button", { name: "Uncommitted" }));

    await waitFor(() => {
      expect(rendered.getByText("Failed to read working tree")).toBeTruthy();
    });
  });
});
