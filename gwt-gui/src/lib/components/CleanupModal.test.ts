import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();
const listenMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  default: {
    invoke: invokeMock,
  },
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

  it("shows spinner indicator for worktrees with open agent tabs", async () => {
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
    expect(rendered.getByTitle("Agent tab is open for this worktree")).toBeTruthy();
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

  it("sorts worktrees with open agent tabs to the top", async () => {
    const otherWorktree = { ...worktreeFixture, path: "/tmp/worktrees/other", branch: "feature/other" };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [otherWorktree, worktreeFixture];
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [worktreeFixture.branch],
    });

    await rendered.findByText(worktreeFixture.branch);

    const rows = rendered.container.querySelectorAll("tbody tr");
    expect(rows.length).toBe(2);
    expect(rows[0].textContent).toContain(worktreeFixture.branch);
  });

  it("shows list error when fetching worktrees fails", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") throw new Error("list failed");
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await waitFor(() => {
      expect(
        rendered.getByText("Failed to list worktrees: list failed")
      ).toBeTruthy();
    });
  });

  it("selects only safe worktrees with Select All Safe", async () => {
    const warningWorktree = {
      ...worktreeFixture,
      path: "/tmp/worktrees/feature/warning",
      branch: "feature/warning",
      safety_level: "warning",
    };
    const disabledWorktree = {
      ...worktreeFixture,
      path: "/tmp/worktrees/feature/disabled",
      branch: "feature/disabled",
      safety_level: "disabled",
    };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") {
        return [worktreeFixture, warningWorktree, disabledWorktree];
      }
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(worktreeFixture.branch);
    await fireEvent.click(rendered.getByRole("button", { name: "Select All Safe" }));

    await waitFor(() => {
      expect(rendered.getByRole("button", { name: "Cleanup (1)" })).toBeTruthy();
    });
  });

  it("preselects branch when preselectedBranch is provided", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      preselectedBranch: worktreeFixture.branch,
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(worktreeFixture.branch);
    await waitFor(() => {
      expect(rendered.getByRole("button", { name: "Cleanup (1)" })).toBeTruthy();
    });
  });

  it("shows failure dialog when cleanup-completed includes failed entries", async () => {
    let cleanupCompletedHandler: ((event: { payload: { results: any[] } }) => void) | null = null;
    listenMock.mockImplementation(async (_eventName: string, handler: any) => {
      cleanupCompletedHandler = handler;
      return () => {};
    });

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "cleanup_worktrees") return [];
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(worktreeFixture.branch);
    const checkbox = rendered.container.querySelector(
      "tbody input[type=\"checkbox\"]"
    ) as HTMLInputElement;
    await fireEvent.click(checkbox);
    await fireEvent.click(rendered.getByRole("button", { name: "Cleanup (1)" }));

    await waitFor(() => {
      expect(cleanupCompletedHandler).toBeTruthy();
    });
    cleanupCompletedHandler?.({
      payload: {
        results: [{ branch: worktreeFixture.branch, success: false, error: "in use" }],
      },
    });

    await waitFor(() => {
      expect(rendered.getByText("Cleanup Failed")).toBeTruthy();
      expect(rendered.getByText("in use")).toBeTruthy();
    });
  });

  it("preserves checked branches across refreshKey updates and drops unavailable ones", async () => {
    const anotherSafe = {
      ...worktreeFixture,
      path: "/tmp/worktrees/feature/another",
      branch: "feature/another",
    };
    const disabledAfterRefresh = {
      ...anotherSafe,
      safety_level: "disabled",
    };

    let listCalls = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") {
        listCalls += 1;
        if (listCalls === 1) return [worktreeFixture, anotherSafe];
        return [worktreeFixture, disabledAfterRefresh];
      }
      return [];
    });

    const onClose = vi.fn();
    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      refreshKey: 1,
      onClose,
      agentTabBranches: [],
    });

    await rendered.findByText(worktreeFixture.branch);
    const checks = rendered.container.querySelectorAll(
      "tbody input[type=\"checkbox\"]"
    ) as NodeListOf<HTMLInputElement>;
    await fireEvent.click(checks[0]);
    await fireEvent.click(checks[1]);
    await waitFor(() => {
      expect(rendered.getByRole("button", { name: "Cleanup (2)" })).toBeTruthy();
    });

    await rendered.rerender({
      open: true,
      projectPath: "/tmp/project",
      refreshKey: 2,
      onClose,
      agentTabBranches: [],
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("list_worktrees", {
        projectPath: "/tmp/project",
      });
      expect(rendered.getByRole("button", { name: "Cleanup (1)" })).toBeTruthy();
    });
  });

  it("shows unsafe confirmation and allows cancel via Escape", async () => {
    const warningWorktree = {
      ...worktreeFixture,
      path: "/tmp/worktrees/feature/warning-only",
      branch: "feature/warning-only",
      safety_level: "warning",
    };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [warningWorktree];
      if (command === "cleanup_worktrees") return [];
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(warningWorktree.branch);
    const checkbox = rendered.container.querySelector(
      "tbody input[type=\"checkbox\"]"
    ) as HTMLInputElement;
    await fireEvent.click(checkbox);
    await fireEvent.click(rendered.getByRole("button", { name: "Cleanup (1)" }));

    await rendered.findByText("Unsafe Worktrees Selected");

    const overlay = rendered.container.querySelector(".overlay") as HTMLDivElement;
    await fireEvent.keyDown(overlay, { key: "Escape" });
    await waitFor(() => {
      expect(rendered.queryByText("Unsafe Worktrees Selected")).toBeNull();
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Cleanup (1)" }));
    await fireEvent.click(rendered.getByRole("button", { name: "Force Cleanup" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "cleanup_worktrees",
        expect.objectContaining({
          force: true,
        })
      );
    });
  });

  it("shows cleanup invoke failure details and allows closing failure dialog", async () => {
    const richWorktree = {
      ...worktreeFixture,
      safety_level: "danger",
      has_changes: true,
      has_unpushed: true,
      ahead: 2,
      behind: 1,
      is_gone: true,
      last_tool_usage: "codex",
    };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [richWorktree];
      if (command === "cleanup_worktrees") throw { code: "E_BUSY" };
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(richWorktree.branch);
    expect(rendered.getByTitle("Uncommitted changes")).toBeTruthy();
    expect(rendered.getByTitle("Unpushed commits")).toBeTruthy();
    expect(rendered.getByText("+2")).toBeTruthy();
    expect(rendered.getByText("-1")).toBeTruthy();
    expect(rendered.getByText("gone")).toBeTruthy();
    expect(rendered.getByText("codex")).toBeTruthy();

    const checkbox = rendered.container.querySelector(
      "tbody input[type=\"checkbox\"]"
    ) as HTMLInputElement;
    await fireEvent.click(checkbox);
    await fireEvent.click(rendered.getByRole("button", { name: "Cleanup (1)" }));
    await fireEvent.click(rendered.getByRole("button", { name: "Force Cleanup" }));

    await waitFor(() => {
      expect(rendered.getByText("Cleanup Failed")).toBeTruthy();
      expect(rendered.getByText("[object Object]")).toBeTruthy();
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Close" }));
    await waitFor(() => {
      expect(rendered.queryByText("Cleanup Failed")).toBeNull();
    });
  });

  it("formats list errors when backend throws a string", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") throw "plain string error";
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await waitFor(() => {
      expect(rendered.getByText("Failed to list worktrees: plain string error")).toBeTruthy();
    });
  });
});
