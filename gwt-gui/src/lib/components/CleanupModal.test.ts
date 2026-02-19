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

function invokeArgsFor(command: string): Record<string, unknown>[] {
  return invokeMock.mock.calls
    .filter((call) => call[0] === command)
    .map((call) => (call[1] ?? {}) as Record<string, unknown>);
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
  agent_status: "unknown",
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

  it("shows indicator for worktrees with open agent tabs", async () => {
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
    // Agent indicator slot should contain a static or pulse dot
    const indicatorSlot = rendered.container.querySelector(".agent-indicator-slot");
    expect(indicatorSlot).toBeTruthy();
    const dot = indicatorSlot?.querySelector(".agent-static-dot, .agent-pulse-dot");
    expect(dot).toBeTruthy();
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
    listenMock.mockResolvedValue(() => {});

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
      expect(
        listenMock.mock.calls.some(
          (call) => call[0] === "cleanup-completed" && typeof call[1] === "function"
        )
      ).toBe(true);
    });
    const cleanupHandler = listenMock.mock.calls.find(
      (call) => call[0] === "cleanup-completed"
    )?.[1] as ((event: { payload: { results: any[] } }) => void) | undefined;
    if (!cleanupHandler) {
      throw new Error("cleanupCompletedHandler is not registered");
    }
    cleanupHandler({
      payload: {
        results: [{ branch: worktreeFixture.branch, success: false, error: "in use", remote_success: null, remote_error: null }],
      },
    });

    await waitFor(() => {
      expect(rendered.getByText("Cleanup Results")).toBeTruthy();
      const resultText = rendered.container.querySelector(".result-list")?.textContent ?? "";
      expect(resultText).toContain("in use");
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

  // --- T020: Toggle visibility based on gh_available ---

  it("shows remote toggle when gh is available", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: false };
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(worktreeFixture.branch);
    await waitFor(() => {
      expect(rendered.container.querySelector("[data-testid='remote-toggle']")).toBeTruthy();
    });
  });

  it("uses projectPath when loading cleanup settings and PR statuses", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: true };
      if (command === "get_cleanup_pr_statuses") return {};
      return [];
    });

    await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project-args",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await waitFor(() => {
      const settingsArgs = invokeArgsFor("get_cleanup_settings")[0];
      const statusesArgs = invokeArgsFor("get_cleanup_pr_statuses")[0];
      expect(settingsArgs).toEqual({ projectPath: "/tmp/project-args" });
      expect(statusesArgs).toEqual({ projectPath: "/tmp/project-args" });
    });
  });

  it("saves remote toggle settings with projectPath", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: false };
      if (command === "get_cleanup_pr_statuses") return {};
      if (command === "set_cleanup_settings") return null;
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project-toggle",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(worktreeFixture.branch);
    let remoteToggle: Element | null = null;
    await waitFor(() => {
      remoteToggle = rendered.container.querySelector("[data-testid='remote-toggle']");
      expect(remoteToggle).toBeTruthy();
    });
    if (!remoteToggle) {
      throw new Error("remote toggle not found");
    }
    await fireEvent.click(remoteToggle);

    await waitFor(() => {
      const args = invokeArgsFor("set_cleanup_settings")[0];
      expect(args).toEqual({
        projectPath: "/tmp/project-toggle",
        settings: { delete_remote_branches: true },
      });
    });
  });

  it("hides remote toggle when gh is not available", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "check_gh_available") return false;
      if (command === "get_cleanup_settings") return { delete_remote_branches: false };
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(worktreeFixture.branch);
    await waitFor(() => {
      expect(rendered.container.querySelector("[data-testid='remote-toggle']")).toBeNull();
    });
  });

  // --- T022: Toggle ON/OFF changes safety dot ---

  it("downgrades safe to warning when toggle ON and PR is open", async () => {
    const safeWorktree = { ...worktreeFixture, safety_level: "safe" };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [safeWorktree];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: true };
      if (command === "get_cleanup_pr_statuses") return { [safeWorktree.branch!]: "open" };
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(safeWorktree.branch);
    // Wait for PR statuses to load and safety to recalculate
    await waitFor(() => {
      const dot = rendered.container.querySelector(".safety-dot");
      expect(dot?.classList.contains("dot-warning")).toBe(true);
    });
  });

  it("downgrades safe to warning when toggle ON and PR status is missing", async () => {
    const safeWorktree = { ...worktreeFixture, safety_level: "safe" };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [safeWorktree];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: true };
      if (command === "get_cleanup_pr_statuses") return {};
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(safeWorktree.branch);
    await waitFor(() => {
      const dot = rendered.container.querySelector(".safety-dot");
      expect(dot?.classList.contains("dot-warning")).toBe(true);
    });
  });

  it("keeps safe dot when toggle OFF regardless of PR status", async () => {
    const safeWorktree = { ...worktreeFixture, safety_level: "safe" };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [safeWorktree];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: false };
      if (command === "get_cleanup_pr_statuses") return { [safeWorktree.branch!]: "open" };
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(safeWorktree.branch);
    await waitFor(() => {
      const dot = rendered.container.querySelector(".safety-dot");
      expect(dot?.classList.contains("dot-safe")).toBe(true);
    });
  });

  // --- T025: PR badges ---

  it("shows green PR merged badge", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: false };
      if (command === "get_cleanup_pr_statuses") return { [worktreeFixture.branch!]: "merged" };
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(worktreeFixture.branch);
    await waitFor(() => {
      const badge = rendered.container.querySelector(".pr-badge-merged");
      expect(badge).toBeTruthy();
      expect(badge?.textContent).toContain("PR: merged");
    });
  });

  it("shows green PR closed badge", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: false };
      if (command === "get_cleanup_pr_statuses") return { [worktreeFixture.branch!]: "closed" };
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(worktreeFixture.branch);
    await waitFor(() => {
      const badge = rendered.container.querySelector(".pr-badge-closed");
      expect(badge).toBeTruthy();
      expect(badge?.textContent).toContain("PR: closed");
    });
  });

  it("shows orange PR open badge", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: false };
      if (command === "get_cleanup_pr_statuses") return { [worktreeFixture.branch!]: "open" };
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(worktreeFixture.branch);
    await waitFor(() => {
      const badge = rendered.container.querySelector(".pr-badge-open");
      expect(badge).toBeTruthy();
      expect(badge?.textContent).toContain("PR: open");
    });
  });

  it("hides PR badge when status is none", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: false };
      if (command === "get_cleanup_pr_statuses") return { [worktreeFixture.branch!]: "none" };
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(worktreeFixture.branch);
    await waitFor(() => {
      expect(rendered.container.querySelector(".pr-badge-merged")).toBeNull();
      expect(rendered.container.querySelector(".pr-badge-open")).toBeNull();
      expect(rendered.container.querySelector(".pr-badge-closed")).toBeNull();
    });
  });

  it("shows spinner while PR statuses are loading", async () => {
    let resolvePrStatuses: ((value: Record<string, string>) => void) | undefined;

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: false };
      if (command === "get_cleanup_pr_statuses") {
        return new Promise<Record<string, string>>((resolve) => {
          resolvePrStatuses = resolve;
        });
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
    await waitFor(() => {
      expect(rendered.container.querySelector(".pr-spinner")).toBeTruthy();
    });

    resolvePrStatuses?.({ [worktreeFixture.branch!]: "merged" });
    await waitFor(() => {
      expect(rendered.container.querySelector(".pr-spinner")).toBeNull();
    });
  });

  it("hides PR badges when gh is not available", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "check_gh_available") return false;
      if (command === "get_cleanup_settings") return { delete_remote_branches: false };
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(worktreeFixture.branch);
    // No PR column at all
    expect(rendered.container.querySelector(".col-pr")).toBeNull();
  });

  // --- T031: Gone badge emphasis ---

  it("emphasizes gone badge when toggle is ON", async () => {
    const goneWorktree = { ...worktreeFixture, is_gone: true };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [goneWorktree];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: true };
      if (command === "get_cleanup_pr_statuses") return {};
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(goneWorktree.branch);
    await waitFor(() => {
      const badge = rendered.container.querySelector(".gone-badge-emphasized");
      expect(badge).toBeTruthy();
    });
  });

  it("shows normal gone badge when toggle is OFF", async () => {
    const goneWorktree = { ...worktreeFixture, is_gone: true };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [goneWorktree];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: false };
      if (command === "get_cleanup_pr_statuses") return {};
      return [];
    });

    const rendered = await renderCleanupModal({
      open: true,
      projectPath: "/tmp/project",
      onClose: vi.fn(),
      agentTabBranches: [],
    });

    await rendered.findByText(goneWorktree.branch);
    await waitFor(() => {
      const badge = rendered.container.querySelector(".gone-badge");
      expect(badge).toBeTruthy();
      expect(rendered.container.querySelector(".gone-badge-emphasized")).toBeNull();
    });
  });

  // --- T033: Result dialog ---

  it("shows local and remote success in result dialog when toggle ON", async () => {
    listenMock.mockResolvedValue(() => {});

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: true };
      if (command === "get_cleanup_pr_statuses") return { [worktreeFixture.branch!]: "merged" };
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
      expect(
        listenMock.mock.calls.some(
          (call) => call[0] === "cleanup-completed" && typeof call[1] === "function"
        )
      ).toBe(true);
    });

    const handler = listenMock.mock.calls.find(
      (call) => call[0] === "cleanup-completed"
    )?.[1] as ((event: { payload: { results: any[] } }) => void);

    handler({
      payload: {
        results: [{
          branch: worktreeFixture.branch,
          success: true,
          error: null,
          remote_success: true,
          remote_error: null,
        }],
      },
    });

    await waitFor(() => {
      expect(rendered.getByText("Cleanup Results")).toBeTruthy();
      const resultText = rendered.container.querySelector(".result-list")?.textContent ?? "";
      expect(resultText).toContain("Local:");
      expect(resultText).toContain("Remote:");
    });
  });

  it("shows remote failure in result dialog when remote fails", async () => {
    listenMock.mockResolvedValue(() => {});

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: true };
      if (command === "get_cleanup_pr_statuses") return { [worktreeFixture.branch!]: "merged" };
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
      expect(
        listenMock.mock.calls.some(
          (call) => call[0] === "cleanup-completed" && typeof call[1] === "function"
        )
      ).toBe(true);
    });

    const handler = listenMock.mock.calls.find(
      (call) => call[0] === "cleanup-completed"
    )?.[1] as ((event: { payload: { results: any[] } }) => void);

    handler({
      payload: {
        results: [{
          branch: worktreeFixture.branch,
          success: true,
          error: null,
          remote_success: false,
          remote_error: "permission denied",
        }],
      },
    });

    await waitFor(() => {
      expect(rendered.getByText("Cleanup Results")).toBeTruthy();
      const resultText = rendered.container.querySelector(".result-list")?.textContent ?? "";
      expect(resultText).toContain("permission denied");
    });
  });

  it("shows only local result when toggle is OFF", async () => {
    listenMock.mockResolvedValue(() => {});

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [worktreeFixture];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: false };
      if (command === "get_cleanup_pr_statuses") return {};
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
      expect(
        listenMock.mock.calls.some(
          (call) => call[0] === "cleanup-completed" && typeof call[1] === "function"
        )
      ).toBe(true);
    });

    const handler = listenMock.mock.calls.find(
      (call) => call[0] === "cleanup-completed"
    )?.[1] as ((event: { payload: { results: any[] } }) => void);

    handler({
      payload: {
        results: [{
          branch: worktreeFixture.branch,
          success: true,
          error: null,
          remote_success: null,
          remote_error: null,
        }],
      },
    });

    await waitFor(() => {
      expect(rendered.getByText("Cleanup Results")).toBeTruthy();
      const resultText = rendered.container.querySelector(".result-list")?.textContent ?? "";
      expect(resultText).toContain("Local:");
      expect(resultText).not.toContain("Remote:");
    });
  });

  // --- T035: Confirm dialog remote warning ---

  it("shows remote warning in confirm dialog when toggle is ON", async () => {
    const warningWorktree = {
      ...worktreeFixture,
      path: "/tmp/worktrees/feature/warn",
      branch: "feature/warn",
      safety_level: "warning",
    };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [warningWorktree];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: true };
      if (command === "get_cleanup_pr_statuses") return {};
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
    expect(rendered.queryByText("Remote branches will also be deleted.")).toBeTruthy();
  });

  it("hides remote warning in confirm dialog when toggle is OFF", async () => {
    const warningWorktree = {
      ...worktreeFixture,
      path: "/tmp/worktrees/feature/warn2",
      branch: "feature/warn2",
      safety_level: "warning",
    };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktrees") return [warningWorktree];
      if (command === "check_gh_available") return true;
      if (command === "get_cleanup_settings") return { delete_remote_branches: false };
      if (command === "get_cleanup_pr_statuses") return {};
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
    expect(rendered.queryByText("Remote branches will also be deleted.")).toBeNull();
  });
});
