import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

async function renderSidebar(props: any) {
  const { default: Sidebar } = await import("./Sidebar.svelte");
  return render(Sidebar, { props });
}

function countInvokeCalls(name: string): number {
  return invokeMock.mock.calls.filter((c) => c[0] === name).length;
}

const branchFixture = {
  name: "feature/sidebar-size",
  commit: "1234567",
  is_current: false,
  ahead: 0,
  behind: 0,
  divergence_status: "UpToDate",
  last_tool_usage: null,
};

describe("Sidebar", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
    invokeMock.mockResolvedValue([]);
  });

  it("does not re-fetch local branches when refreshKey is unchanged", async () => {
    const onBranchSelect = vi.fn();

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect,
      refreshKey: 0,
    });

    await waitFor(() => {
      expect(invokeMock.mock.calls.length).toBeGreaterThan(0);
    });

    const firstLocalBranchFetchCount = countInvokeCalls("list_worktree_branches");

    // Rerender with the same key should not trigger a re-fetch.
    await rendered.rerender({ refreshKey: 0 });

    await new Promise((r) => setTimeout(r, 50));
    expect(countInvokeCalls("list_worktree_branches")).toBe(firstLocalBranchFetchCount);
  });

  it("re-fetches local branches when refreshKey changes", async () => {
    const onBranchSelect = vi.fn();

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect,
      refreshKey: 0,
    });

    await waitFor(() => {
      expect(invokeMock.mock.calls.length).toBeGreaterThan(0);
    });

    const firstLocalBranchFetchCount = countInvokeCalls("list_worktree_branches");

    // Changing refreshKey should trigger a re-fetch.
    await rendered.rerender({
      refreshKey: 1,
    });

    await waitFor(() => {
      expect(countInvokeCalls("list_worktree_branches")).toBe(
        firstLocalBranchFetchCount + 1
      );
    });
  });

  it("applies sidebar width from props", async () => {
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      widthPx: 333,
    });

    const sidebar = rendered.container.querySelector(".sidebar");
    expect(sidebar).toBeTruthy();
    expect((sidebar as HTMLElement).style.width).toBe("333px");
    expect((sidebar as HTMLElement).style.minWidth).toBe("333px");
  });

  it("opens Launch Agent from context menu", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const onBranchActivate = vi.fn();
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      onBranchActivate,
    });

    const branchLabel = await rendered.findByText(branchFixture.name);
    const branchButton = branchLabel.closest("button");
    expect(branchButton).toBeTruthy();

    await fireEvent.contextMenu(branchButton as HTMLElement);

    const launchMenuButton = await rendered.findByRole("button", {
      name: "Launch Agent...",
    });
    expect(launchMenuButton).toBeTruthy();

    await fireEvent.click(launchMenuButton);

    expect(onBranchActivate).toHaveBeenCalledTimes(1);
    expect(onBranchActivate).toHaveBeenCalledWith(
      expect.objectContaining({ name: branchFixture.name })
    );
  });

  it("sets autocapitalize off for the branch filter input", async () => {
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    const searchInput = rendered.getByPlaceholderText("Filter branches...") as HTMLInputElement;
    expect(searchInput.getAttribute("autocapitalize")).toBe("off");
  });

  it("disables Launch Agent menu item when no activation handler is provided", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    const branchLabel = await rendered.findByText(branchFixture.name);
    const branchButton = branchLabel.closest("button");
    expect(branchButton).toBeTruthy();

    await fireEvent.contextMenu(branchButton as HTMLElement);

    const launchMenuButton = await rendered.findByRole("button", {
      name: "Launch Agent...",
    });
    expect((launchMenuButton as HTMLButtonElement).disabled).toBe(true);
  });
});
