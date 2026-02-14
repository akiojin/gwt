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
const remoteBranchFixture = {
  ...branchFixture,
  name: "origin/feature/sidebar-size",
};

const quickStartEntryFixture = {
  branch: branchFixture.name,
  tool_id: "codex",
  tool_label: "Codex",
  session_id: "session-123",
  mode: "normal",
  model: "gpt-5",
  reasoning_level: "high",
  skip_permissions: true,
  tool_version: "0.33.0",
  docker_service: "workspace",
  docker_force_host: true,
  docker_recreate: false,
  docker_build: true,
  docker_keep: false,
  timestamp: 1_700_000_000,
};

const noSessionSummaryFixture = {
  status: "no-session",
  generating: false,
  bulletPoints: [],
};

const makeLocalStorageMock = () => {
  const store = new Map<string, string>();
  return {
    getItem: (key: string) => (store.has(key) ? store.get(key) : null),
    setItem: (key: string, value: string) => {
      store.set(key, String(value));
    },
    removeItem: (key: string) => {
      store.delete(key);
    },
    clear: () => {
      store.clear();
    },
    key: (index: number) => Array.from(store.keys())[index] ?? null,
    get length() {
      return store.size;
    },
  };
};

describe("Sidebar", () => {
  beforeEach(() => {
    cleanup();
    const mockLocalStorage = makeLocalStorageMock();
    Object.defineProperty(globalThis, "localStorage", {
      value: mockLocalStorage,
      configurable: true,
    });
    Object.defineProperty(globalThis, "__TAURI_INTERNALS__", {
      value: { invoke: invokeMock },
      configurable: true,
    });
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

  it("refreshes all filter caches on refreshKey change and reuses prefetched data on switch", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_remote_branches") return [remoteBranchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      refreshKey: 0,
    });

    await rendered.findByText(branchFixture.name);
    invokeMock.mockClear();

    await rendered.rerender({ refreshKey: 1 });

    await waitFor(() => {
      expect(countInvokeCalls("list_worktree_branches")).toBe(2);
      expect(countInvokeCalls("list_remote_branches")).toBe(2);
      expect(countInvokeCalls("list_worktrees")).toBe(2);
    });

    const remoteFetchCount = countInvokeCalls("list_remote_branches");
    await fireEvent.click(rendered.getByRole("button", { name: "Remote" }));
    await rendered.findByText(remoteBranchFixture.name);
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(countInvokeCalls("list_remote_branches")).toBe(remoteFetchCount);
  });

  it("reuses cached filter data when switching back within ttl", async () => {
    const nowSpy = vi.spyOn(Date, "now").mockImplementation(() => 1_700_000_000_000);
    try {
      invokeMock.mockImplementation(async (command: string) => {
        if (command === "list_worktree_branches") return [branchFixture];
        if (command === "list_remote_branches") return [remoteBranchFixture];
        if (command === "list_worktrees") return [];
        return [];
      });

      const rendered = await renderSidebar({
        projectPath: "/tmp/project",
        onBranchSelect: vi.fn(),
      });

      await rendered.findByText(branchFixture.name);
      expect(countInvokeCalls("list_worktree_branches")).toBe(1);

      await fireEvent.click(rendered.getByRole("button", { name: "Remote" }));
      await rendered.findByText(remoteBranchFixture.name);
      expect(countInvokeCalls("list_remote_branches")).toBe(1);

      await fireEvent.click(rendered.getByRole("button", { name: "Local" }));
      await rendered.findByText(branchFixture.name);
      await new Promise((resolve) => setTimeout(resolve, 30));

      expect(countInvokeCalls("list_worktree_branches")).toBe(1);
    } finally {
      nowSpy.mockRestore();
    }
  });

  it("keeps cached list visible and refreshes in background after ttl", async () => {
    let nowMs = 1_700_000_000_000;
    const nowSpy = vi.spyOn(Date, "now").mockImplementation(() => nowMs);
    let localFetchCount = 0;
    const localRefreshDeferred: { resolve?: (value: typeof branchFixture[]) => void } = {};

    try {
      invokeMock.mockImplementation((command: string) => {
        if (command === "list_worktree_branches") {
          localFetchCount += 1;
          if (localFetchCount === 1) return Promise.resolve([branchFixture]);
          return new Promise<typeof branchFixture[]>((resolve) => {
            localRefreshDeferred.resolve = resolve;
          });
        }
        if (command === "list_remote_branches") return Promise.resolve([remoteBranchFixture]);
        if (command === "list_worktrees") return Promise.resolve([]);
        return Promise.resolve([]);
      });

      const rendered = await renderSidebar({
        projectPath: "/tmp/project",
        onBranchSelect: vi.fn(),
      });

      await rendered.findByText(branchFixture.name);
      await fireEvent.click(rendered.getByRole("button", { name: "Remote" }));
      await rendered.findByText(remoteBranchFixture.name);

      nowMs += 11_000;
      await fireEvent.click(rendered.getByRole("button", { name: "Local" }));
      await rendered.findByText(branchFixture.name);

      expect(rendered.queryByText("Loading...")).toBeNull();
      await waitFor(() => {
        expect(countInvokeCalls("list_worktree_branches")).toBe(2);
      });

      localRefreshDeferred.resolve?.([branchFixture]);
      await waitFor(() => {
        expect(rendered.queryByText("Loading...")).toBeNull();
      });
    } finally {
      localRefreshDeferred.resolve?.([branchFixture]);
      nowSpy.mockRestore();
    }
  });

  it("does not re-run fetch_pr_status immediately when switching filters", async () => {
    vi.useFakeTimers();
    try {
      invokeMock.mockImplementation((command: string) => {
        if (command === "list_worktree_branches") return Promise.resolve([branchFixture]);
        if (command === "list_remote_branches") return Promise.resolve([remoteBranchFixture]);
        if (command === "list_worktrees") return Promise.resolve([]);
        if (command === "fetch_pr_status") {
          return Promise.resolve({
            statuses: {},
            ghStatus: { available: true, authenticated: true },
          });
        }
        return Promise.resolve([]);
      });

      const rendered = await renderSidebar({
        projectPath: "/tmp/project",
        onBranchSelect: vi.fn(),
      });

      await rendered.findByText(branchFixture.name);
      await vi.advanceTimersByTimeAsync(30_000);
      await waitFor(() => {
        expect(countInvokeCalls("fetch_pr_status")).toBeGreaterThan(0);
      });

      const beforeSwitchCount = countInvokeCalls("fetch_pr_status");

      await fireEvent.click(rendered.getByRole("button", { name: "Remote" }));
      await rendered.findByText(remoteBranchFixture.name);
      await fireEvent.click(rendered.getByRole("button", { name: "Local" }));
      await rendered.findByText(branchFixture.name);
      await vi.advanceTimersByTimeAsync(20);

      expect(countInvokeCalls("fetch_pr_status")).toBe(beforeSwitchCount);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not queue extra fetch_pr_status calls while previous polling is in flight", async () => {
    vi.useFakeTimers();
    type PrStatusResponse = {
      statuses: Record<string, null>;
      ghStatus: { available: boolean; authenticated: boolean };
    };
    let pendingResolve: ((value: PrStatusResponse) => void) | null = null;
    try {
      invokeMock.mockImplementation((command: string) => {
        if (command === "list_worktree_branches") return Promise.resolve([branchFixture]);
        if (command === "list_remote_branches") return Promise.resolve([remoteBranchFixture]);
        if (command === "list_worktrees") return Promise.resolve([]);
        if (command === "fetch_pr_status") {
          return new Promise<PrStatusResponse>((resolve) => {
            pendingResolve = resolve;
          });
        }
        return Promise.resolve([]);
      });

      const rendered = await renderSidebar({
        projectPath: "/tmp/project",
        onBranchSelect: vi.fn(),
      });

      await rendered.findByText(branchFixture.name);
      await vi.advanceTimersByTimeAsync(30_000);
      await waitFor(() => {
        expect(countInvokeCalls("fetch_pr_status")).toBeGreaterThan(0);
      });
      const inFlightCount = countInvokeCalls("fetch_pr_status");

      await fireEvent.click(rendered.getByRole("button", { name: "Remote" }));
      await rendered.findByText(remoteBranchFixture.name);
      await fireEvent.click(rendered.getByRole("button", { name: "Local" }));
      await rendered.findByText(branchFixture.name);
      await vi.advanceTimersByTimeAsync(20);

      expect(countInvokeCalls("fetch_pr_status")).toBe(inFlightCount);
    } finally {
      const resolvePending = pendingResolve as ((value: PrStatusResponse) => void) | null;
      if (resolvePending) {
        resolvePending({
          statuses: {},
          ghStatus: { available: true, authenticated: true },
        });
        pendingResolve = null;
      }
      vi.useRealTimers();
    }
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

  it("opens Launch Agent from summary panel button", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      if (command === "get_branch_quick_start") return [];
      if (command === "get_branch_session_summary") return noSessionSummaryFixture;
      return [];
    });

    const onLaunchAgent = vi.fn();
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      onLaunchAgent,
      selectedBranch: branchFixture,
    });

    const launchButton = await rendered.findByRole("button", {
      name: "Launch Agent...",
    });
    await fireEvent.click(launchButton);

    expect(onLaunchAgent).toHaveBeenCalledTimes(1);
  });

  it("invokes onQuickLaunch with continue mode from Quick Start", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      if (command === "get_branch_quick_start") return [quickStartEntryFixture];
      if (command === "get_branch_session_summary") return noSessionSummaryFixture;
      return [];
    });

    const onQuickLaunch = vi.fn().mockResolvedValue(undefined);
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      onQuickLaunch,
      selectedBranch: branchFixture,
    });

    const continueButton = await rendered.findByRole("button", { name: "Continue" });
    await fireEvent.click(continueButton);

    await waitFor(() => expect(onQuickLaunch).toHaveBeenCalledTimes(1));
    expect(onQuickLaunch).toHaveBeenCalledWith(
      expect.objectContaining({
        agentId: "codex",
        branch: branchFixture.name,
        mode: "continue",
        resumeSessionId: quickStartEntryFixture.session_id,
        model: quickStartEntryFixture.model,
        agentVersion: quickStartEntryFixture.tool_version,
        skipPermissions: quickStartEntryFixture.skip_permissions,
        reasoningLevel: quickStartEntryFixture.reasoning_level,
        dockerService: quickStartEntryFixture.docker_service,
        dockerForceHost: quickStartEntryFixture.docker_force_host,
        dockerRecreate: quickStartEntryFixture.docker_recreate,
        dockerBuild: quickStartEntryFixture.docker_build,
        dockerKeep: quickStartEntryFixture.docker_keep,
      })
    );
  });

  it("invokes onQuickLaunch with normal mode from Quick Start New", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      if (command === "get_branch_quick_start") return [quickStartEntryFixture];
      if (command === "get_branch_session_summary") return noSessionSummaryFixture;
      return [];
    });

    const onQuickLaunch = vi.fn().mockResolvedValue(undefined);
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      onQuickLaunch,
      selectedBranch: branchFixture,
    });

    const newButton = await rendered.findByRole("button", { name: "New" });
    await fireEvent.click(newButton);

    await waitFor(() => expect(onQuickLaunch).toHaveBeenCalledTimes(1));
    expect(onQuickLaunch).toHaveBeenCalledWith(
      expect.objectContaining({
        agentId: "codex",
        branch: branchFixture.name,
        mode: "normal",
      })
    );
    expect(onQuickLaunch.mock.calls[0][0].resumeSessionId).toBeUndefined();
  });

  it("ignores Quick Start button clicks when no onQuickLaunch handler is provided", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      if (command === "get_branch_quick_start") return [quickStartEntryFixture];
      if (command === "get_branch_session_summary") return noSessionSummaryFixture;
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      selectedBranch: branchFixture,
    });

    const continueButton = await rendered.findByRole("button", { name: "Continue" });
    await fireEvent.click(continueButton);

    expect(rendered.queryByText(/^Failed to launch:/)).toBeNull();
  });

  it("disables capitalization and completion helpers for the branch filter input", async () => {
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    const searchInput = rendered.getByPlaceholderText("Filter branches...") as HTMLInputElement;
    expect(searchInput.getAttribute("autocapitalize")).toBe("off");
    expect(searchInput.getAttribute("autocorrect")).toBe("off");
    expect(searchInput.getAttribute("autocomplete")).toBe("off");
    expect(searchInput.getAttribute("spellcheck")).toBe("false");
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

  it("uses default summary panel height when no persisted value exists", async () => {
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    const summaryWrap = rendered.container.querySelector(".worktree-summary-wrap");
    expect(summaryWrap).toBeTruthy();
    expect(summaryWrap?.getAttribute("style")).toContain("height: 360px");
  });

  it("restores summary panel height from localStorage", async () => {
    window.localStorage.setItem("gwt.sidebar.worktreeSummaryHeight", "420");

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    const summaryWrap = rendered.container.querySelector(".worktree-summary-wrap");
    expect(summaryWrap).toBeTruthy();
    expect(summaryWrap?.getAttribute("style")).toContain("height: 420px");
  });

  it("falls back to default height when persisted value is invalid", async () => {
    window.localStorage.setItem("gwt.sidebar.worktreeSummaryHeight", "invalid");

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    const summaryWrap = rendered.container.querySelector(".worktree-summary-wrap");
    expect(summaryWrap).toBeTruthy();
    expect(summaryWrap?.getAttribute("style")).toContain("height: 360px");
  });

  it("renders summary resize handle in branch mode", async () => {
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    const resizeHandle = rendered.container.querySelector(".summary-resize-handle");
    expect(resizeHandle).toBeTruthy();
    expect(resizeHandle?.getAttribute("aria-label")).toBe("Resize session summary");
  });

  it("shows spinner indicator for branches with open agent tabs", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") {
        return [branchFixture];
      }
      if (command === "list_worktrees") {
        return [
          {
            path: "/tmp/worktrees/feature-sidebar-size",
            branch: branchFixture.name,
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
          },
        ];
      }
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      agentTabBranches: [branchFixture.name],
    });

    await rendered.findByText(branchFixture.name);
    expect(rendered.getByTitle("Agent tab is open for this branch")).toBeTruthy();
  });

  it("shows PR badge when prStatuses contains data for a branch", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const prStatuses: Record<string, any> = {
      [branchFixture.name]: {
        number: 42,
        title: "Test PR",
        state: "OPEN",
        url: "https://github.com/test/pr/42",
        mergeable: "MERGEABLE",
        author: "test",
        baseBranch: "main",
        headBranch: branchFixture.name,
        labels: [],
        assignees: [],
        milestone: null,
        linkedIssues: [],
        checkSuites: [],
        reviews: [],
        reviewComments: [],
        changedFilesCount: 1,
        additions: 10,
        deletions: 5,
      },
    };

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      prStatuses,
      ghCliStatus: { available: true, authenticated: true },
    });

    await rendered.findByText(branchFixture.name);
    const badge = rendered.getByText(/#42 Open/);
    expect(badge).toBeTruthy();
    expect(badge.classList.contains("pr-badge")).toBe(true);
  });

  it("shows tree toggle for branches with PR and expands workflow runs", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const prStatuses: Record<string, any> = {
      [branchFixture.name]: {
        number: 42,
        title: "Test PR",
        state: "OPEN",
        url: "https://github.com/test/pr/42",
        mergeable: "MERGEABLE",
        author: "test",
        baseBranch: "main",
        headBranch: branchFixture.name,
        labels: [],
        assignees: [],
        milestone: null,
        linkedIssues: [],
        checkSuites: [
          { workflowName: "CI Build", runId: 100, status: "completed", conclusion: "success" },
          { workflowName: "Lint", runId: 101, status: "in_progress", conclusion: null },
        ],
        reviews: [],
        reviewComments: [],
        changedFilesCount: 1,
        additions: 10,
        deletions: 5,
      },
    };

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      prStatuses,
      ghCliStatus: { available: true, authenticated: true },
    });

    await rendered.findByText(branchFixture.name);

    // Tree toggle should be present
    const toggleBtn = rendered.getByTitle("Expand");
    expect(toggleBtn).toBeTruthy();

    // Click to expand
    await fireEvent.click(toggleBtn);

    // Workflow names should appear
    expect(rendered.getByText("CI Build")).toBeTruthy();
    expect(rendered.getByText("Lint")).toBeTruthy();
  });

  it("shows 'No PR' badge when ghCli is authenticated but no PR exists", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      prStatuses: {},
      ghCliStatus: { available: true, authenticated: true },
    });

    await rendered.findByText(branchFixture.name);
    const badge = rendered.getByText("No PR");
    expect(badge).toBeTruthy();
    expect(badge.classList.contains("no-pr")).toBe(true);
  });

  it("shows disconnected badge when ghCli is not authenticated", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      prStatuses: {},
      ghCliStatus: { available: true, authenticated: false },
    });

    await rendered.findByText(branchFixture.name);
    const badge = rendered.getByText("GitHub not connected");
    expect(badge).toBeTruthy();
    expect(badge.classList.contains("disconnected")).toBe(true);
  });

  it("calls onOpenCiLog when clicking a workflow run item", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const prStatuses: Record<string, any> = {
      [branchFixture.name]: {
        number: 42,
        title: "Test PR",
        state: "OPEN",
        url: "https://github.com/test/pr/42",
        mergeable: "MERGEABLE",
        author: "test",
        baseBranch: "main",
        headBranch: branchFixture.name,
        labels: [],
        assignees: [],
        milestone: null,
        linkedIssues: [],
        checkSuites: [
          { workflowName: "CI Build", runId: 100, status: "completed", conclusion: "success" },
        ],
        reviews: [],
        reviewComments: [],
        changedFilesCount: 1,
        additions: 10,
        deletions: 5,
      },
    };

    const onOpenCiLog = vi.fn();
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      prStatuses,
      ghCliStatus: { available: true, authenticated: true },
      onOpenCiLog,
    });

    await rendered.findByText(branchFixture.name);

    // Expand tree
    const toggleBtn = rendered.getByTitle("Expand");
    await fireEvent.click(toggleBtn);

    // Click workflow run
    const ciItem = rendered.getByText("CI Build");
    await fireEvent.click(ciItem.closest("button")!);

    expect(onOpenCiLog).toHaveBeenCalledWith(100);
  });
});
