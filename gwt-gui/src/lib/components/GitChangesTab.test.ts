import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

import type { FileChange, FileDiff, WorkingTreeEntry } from "../types";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
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

  it("renders Renamed kind color as cyan", async () => {
    const renamedFiles: FileChange[] = [
      {
        path: "src/old-name.ts",
        kind: "Renamed",
        additions: 0,
        deletions: 0,
        is_binary: false,
      },
    ];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") return renamedFiles;
      return [];
    });

    const rendered = await renderTab();
    await waitFor(() => {
      const kindSpan = rendered.container.querySelector(".file-kind");
      expect(kindSpan).toBeTruthy();
      expect(kindSpan!.textContent).toBe("R");
      expect((kindSpan as HTMLElement).style.color).toBe("var(--cyan)");
    });
  });

  it("renders Deleted kind color as red", async () => {
    const deletedFiles: FileChange[] = [
      {
        path: "src/removed.ts",
        kind: "Deleted",
        additions: 0,
        deletions: 10,
        is_binary: false,
      },
    ];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") return deletedFiles;
      return [];
    });

    const rendered = await renderTab();
    await waitFor(() => {
      const kindSpan = rendered.container.querySelector(".file-kind");
      expect(kindSpan).toBeTruthy();
      expect(kindSpan!.textContent).toBe("D");
      expect((kindSpan as HTMLElement).style.color).toBe("var(--red)");
    });
  });

  it("shows empty staged and unstaged sections separately", async () => {
    const onlyStagedTree: WorkingTreeEntry[] = [
      { path: "src/staged-only.ts", status: "Added", is_staged: true },
    ];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") return [];
      if (command === "get_working_tree_status") return onlyStagedTree;
      return [];
    });

    const rendered = await renderTab();
    await fireEvent.click(rendered.getByRole("button", { name: "Uncommitted" }));

    await waitFor(() => {
      expect(rendered.getByText("Staged")).toBeTruthy();
      expect(rendered.getByText("src/staged-only.ts")).toBeTruthy();
      expect(rendered.getByText("No unstaged changes")).toBeTruthy();
    });
  });

  it("shows empty staged section when only unstaged changes exist", async () => {
    const onlyUnstagedTree: WorkingTreeEntry[] = [
      { path: "src/unstaged-only.ts", status: "Modified", is_staged: false },
    ];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") return [];
      if (command === "get_working_tree_status") return onlyUnstagedTree;
      return [];
    });

    const rendered = await renderTab();
    await fireEvent.click(rendered.getByRole("button", { name: "Uncommitted" }));

    await waitFor(() => {
      expect(rendered.getByText("No staged changes")).toBeTruthy();
      expect(rendered.getByText("src/unstaged-only.ts")).toBeTruthy();
    });
  });

  it("handles diff fetch error gracefully", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") return fileChanges;
      if (command === "get_file_diff") throw new Error("diff fetch failed");
      return [];
    });

    const rendered = await renderTab();
    await waitFor(() => {
      expect(rendered.getByText("index.ts")).toBeTruthy();
    });

    const fileRow = rendered.getByText("index.ts").closest(".file-row") as HTMLElement;
    await fireEvent.click(fileRow);

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Error: diff fetch failed");
    });
  });

  it("handles non-Error object in toErrorMessage", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") throw 42;
      return [];
    });

    const rendered = await renderTab();
    await waitFor(() => {
      expect(rendered.container.textContent).toContain("42");
    });
  });

  it("handles result null from get_branch_diff_files", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") return null;
      return [];
    });

    const rendered = await renderTab();
    await waitFor(() => {
      expect(rendered.getByText("No changes")).toBeTruthy();
    });
  });

  it("handles result null from get_working_tree_status", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") return [];
      if (command === "get_working_tree_status") return null;
      return [];
    });

    const rendered = await renderTab();
    await fireEvent.click(rendered.getByRole("button", { name: "Uncommitted" }));

    await waitFor(() => {
      expect(rendered.getByText("No uncommitted changes")).toBeTruthy();
    });
  });

  it("does not expand diff if already cached", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_branch_diff_files") return fileChanges;
      if (command === "get_file_diff") return diffFixture;
      return [];
    });

    const rendered = await renderTab();
    await waitFor(() => {
      expect(rendered.getByText("index.ts")).toBeTruthy();
    });

    // First click: expand and load diff
    const fileRow = rendered.getByText("index.ts").closest(".file-row") as HTMLElement;
    await fireEvent.click(fileRow);
    await waitFor(() => {
      expect(rendered.getByText("+added")).toBeTruthy();
    });

    // Collapse
    await fireEvent.click(fileRow);
    await waitFor(() => {
      expect(rendered.queryByText("+added")).toBeNull();
    });

    // Clear call count after initial loads
    const callCountBefore = invokeMock.mock.calls.filter((c: string[]) => c[0] === "get_file_diff").length;

    // Re-expand: should use cached diff, not re-fetch
    await fireEvent.click(fileRow);
    await waitFor(() => {
      expect(rendered.getByText("+added")).toBeTruthy();
    });

    const callCountAfter = invokeMock.mock.calls.filter((c: string[]) => c[0] === "get_file_diff").length;
    expect(callCountAfter).toBe(callCountBefore);
  });

  it("ignores stale committed response after base branch changes again", async () => {
    let resolveDevelop: ((value: FileChange[]) => void) | null = null;

    invokeMock.mockImplementation((command: string, args?: { baseBranch?: string }) => {
      if (command === "get_branch_diff_files") {
        if (args?.baseBranch === "develop") {
          return new Promise<FileChange[]>((resolve) => {
            resolveDevelop = resolve;
          });
        }
        if (args?.baseBranch === "main") {
          return Promise.resolve([
            {
              path: "src/main-final.ts",
              kind: "Modified",
              additions: 2,
              deletions: 1,
              is_binary: false,
            },
          ]);
        }
      }
      if (command === "get_working_tree_status") return Promise.resolve([]);
      return Promise.resolve([]);
    });

    const rendered = await renderTab({ baseBranch: "develop" });
    await waitFor(() => {
      expect(resolveDevelop).toBeTruthy();
    });

    await rendered.rerender({
      projectPath: "/tmp/project",
      branch: "feature/changes",
      baseBranch: "main",
      refreshToken: 0,
    });

    await waitFor(() => {
      expect(rendered.getByText("main-final.ts")).toBeTruthy();
    });

    const resolveDevelopNow = resolveDevelop as ((value: FileChange[]) => void) | null;
    if (!resolveDevelopNow) {
      throw new Error("Expected pending develop response");
    }
    resolveDevelopNow([
      {
        path: "src/develop-stale.ts",
        kind: "Added",
        additions: 5,
        deletions: 0,
        is_binary: false,
      },
    ]);
    await Promise.resolve();

    await waitFor(() => {
      expect(rendered.queryByText("develop-stale.ts")).toBeNull();
      expect(rendered.getByText("main-final.ts")).toBeTruthy();
    });
  });
});
