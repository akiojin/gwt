import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();
const listenMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

async function renderCleanupModal(props: any) {
  const { default: CleanupModal } = await import("./CleanupModal.svelte");
  return render(CleanupModal, { props });
}

const worktreeFixture = {
  path: "/tmp/worktrees/feature/active",
  branch: "feature/active",
  commit: "1234567",
  status: "active",
  is_main: false,
  has_changes: false,
  has_unpushed: false,
  is_current: false,
  is_protected: false,
  is_agent_running: false,
  ahead: 0,
  behind: 0,
  is_gone: false,
  last_tool_usage: null,
  safety_level: "safe",
};

describe("CleanupModal", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
    listenMock.mockReset();
    listenMock.mockResolvedValue(() => {});
  });

  it("shows ACTIVE badge for worktrees with open agent tabs", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [worktreeFixture.branch],
    });

    await rendered.findByText(worktreeFixture.branch);
    expect(rendered.getByText("ACTIVE")).toBeTruthy();
  });

  it("warns before cleanup when selected worktrees have open agent tabs", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "cleanup_worktrees") return [];
      return [];
    });

    const onClose = vi.fn();
    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose,
      agentTabBranches: [worktreeFixture.branch],
    });

    await rendered.findByText(worktreeFixture.branch);

    const checkbox = rendered.container.querySelector(
      "tbody input[type=\"checkbox\"]"
    ) as HTMLInputElement;
    expect(checkbox).toBeTruthy();
    await fireEvent.click(checkbox);

    const cleanupButton = await rendered.findByRole("button", {
      name: "Cleanup (1)",
    });
    await fireEvent.click(cleanupButton);

    await rendered.findByText("Active Agent Tabs Detected");

    const continueButton = await rendered.findByRole("button", { name: "Continue" });
    await fireEvent.click(continueButton);

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledTimes(1);
      expect(invokeMock).toHaveBeenCalledWith(
        "cleanup_worktrees",
        expect.objectContaining({
          projectPath: "/tmp/project",
          branches: [worktreeFixture.branch],
          force: false,
        })
      );
    });
  });

  it("uses force cleanup when both unsafe and active-tab worktrees are selected", async () => {
    const unsafeWorktree = { ...worktreeFixture, safety_level: "warning" };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [unsafeWorktree];
      if (command === "cleanup_worktrees") return [];
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [unsafeWorktree.branch],
    });

    await rendered.findByText(unsafeWorktree.branch);

    const checkbox = rendered.container.querySelector(
      "tbody input[type=\"checkbox\"]"
    ) as HTMLInputElement;
    await fireEvent.click(checkbox);

    const cleanupButton = await rendered.findByRole("button", {
      name: "Cleanup (1)",
    });
    await fireEvent.click(cleanupButton);

    await rendered.findByText("Active Tabs and Unsafe Worktrees Selected");

    const forceButton = await rendered.findByRole("button", { name: "Force Cleanup" });
    await fireEvent.click(forceButton);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "cleanup_worktrees",
        expect.objectContaining({
          force: true,
        })
      );
    });
  });
});

