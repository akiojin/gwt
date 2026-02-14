import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();
type TauriEventHandler = (event: { payload: any }) => void;
const eventListeners = new Map<string, Set<TauriEventHandler>>();
const listenMock = vi.fn(async (eventName: string, handler: TauriEventHandler) => {
  let bucket = eventListeners.get(eventName);
  if (!bucket) {
    bucket = new Set();
    eventListeners.set(eventName, bucket);
  }
  bucket.add(handler);
  return () => {
    bucket?.delete(handler);
    if (bucket && bucket.size === 0) eventListeners.delete(eventName);
  };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

async function emitTauriEvent(eventName: string, payload: any) {
  const handlers = Array.from(eventListeners.get(eventName) ?? []);
  for (const handler of handlers) {
    await handler({ payload });
  }
}

async function renderSidebar(props: any) {
  const { default: Sidebar } = await import("./Sidebar.svelte");
  return render(Sidebar, { props });
}

function countInvokeCalls(name: string): number {
  return invokeMock.mock.calls.filter((c) => c[0] === name).length;
}

function getRenderedBranchNames(rendered: { container: HTMLElement }): string[] {
  return Array.from(
    rendered.container.querySelectorAll("button.branch-item .branch-name")
  ).map((node) => node.textContent?.trim() ?? "");
}

function fetchPrStatusProjectPaths(): string[] {
  return invokeMock.mock.calls
    .filter((c) => c[0] === "fetch_pr_status")
    .map((c) => (c[1] as { projectPath?: string } | undefined)?.projectPath ?? "");
}

const branchFixture = {
  name: "feature/sidebar-size",
  commit: "1234567",
  is_current: false,
  is_agent_running: false,
  ahead: 0,
  behind: 0,
  divergence_status: "UpToDate",
  last_tool_usage: null,
};
const mainBranchFixture = {
  ...branchFixture,
  name: "main",
  commit_timestamp: 1_700_000_100,
};
const developBranchFixture = {
  ...branchFixture,
  name: "develop",
  commit_timestamp: 1_700_000_050,
};
const featureAlphaBranchFixture = {
  ...branchFixture,
  name: "feature/alpha",
  commit_timestamp: 1_700_000_090,
};
const featureBetaBranchFixture = {
  ...branchFixture,
  name: "feature/beta",
  commit_timestamp: 1_700_000_080,
};
const featureNoTimestampBranchFixture = {
  ...branchFixture,
  name: "feature/no-timestamp",
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
    eventListeners.clear();
    listenMock.mockClear();
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
      expect(countInvokeCalls("fetch_pr_status")).toBeGreaterThan(0);

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

  it("scopes in-flight polling by projectPath and refreshes immediately after project switch", async () => {
    vi.useFakeTimers();
    type PrStatusResponse = {
      statuses: Record<string, null>;
      ghStatus: { available: boolean; authenticated: boolean };
    };
    let resolveProjectA: ((value: PrStatusResponse) => void) | null = null;
    const projectBBranch = {
      ...branchFixture,
      name: "feature/project-b",
    };
    try {
      invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
        const path = typeof args?.projectPath === "string" ? args.projectPath : "";
        if (command === "list_worktree_branches") {
          if (path === "/tmp/project-b") return Promise.resolve([projectBBranch]);
          return Promise.resolve([branchFixture]);
        }
        if (command === "list_worktrees") return Promise.resolve([]);
        if (command === "fetch_pr_status") {
          if (path === "/tmp/project-b") {
            return Promise.resolve({
              statuses: {},
              ghStatus: { available: true, authenticated: true },
            });
          }
          return new Promise<PrStatusResponse>((resolve) => {
            resolveProjectA = resolve;
          });
        }
        return Promise.resolve([]);
      });

      const rendered = await renderSidebar({
        projectPath: "/tmp/project-a",
        onBranchSelect: vi.fn(),
      });

      await rendered.findByText(branchFixture.name);
      await vi.advanceTimersByTimeAsync(30_000);
      await waitFor(() => {
        expect(fetchPrStatusProjectPaths()).toContain("/tmp/project-a");
      });

      await rendered.rerender({ projectPath: "/tmp/project-b" });
      await rendered.findByText(projectBBranch.name);
      await waitFor(() => {
        expect(fetchPrStatusProjectPaths()).toContain("/tmp/project-b");
      });
    } finally {
      const finalizeProjectA = resolveProjectA as unknown as
        | ((value: PrStatusResponse) => void)
        | null;
      if (typeof finalizeProjectA === "function") {
        finalizeProjectA({
          statuses: {},
          ghStatus: { available: true, authenticated: true },
        });
      }
      vi.useRealTimers();
    }
  });

  it("clears stale polling statuses immediately when projectPath changes", async () => {
    type PrStatusResponse = {
      statuses: Record<string, unknown>;
      ghStatus: { available: boolean; authenticated: boolean };
    };
    let resolveProjectB: ((value: PrStatusResponse) => void) | null = null;
    try {
      invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
        const path = typeof args?.projectPath === "string" ? args.projectPath : "";
        if (command === "list_worktree_branches") return Promise.resolve([branchFixture]);
        if (command === "list_worktrees") return Promise.resolve([]);
        if (command === "fetch_pr_status") {
          if (path === "/tmp/project-a") {
            return Promise.resolve({
              statuses: {
                [branchFixture.name]: {
                  number: 111,
                  state: "OPEN",
                  title: "A",
                  url: "https://example.invalid/pr/111",
                  mergeable: "MERGEABLE",
                  draft: false,
                  author: "owner",
                  baseBranch: "main",
                  headBranch: branchFixture.name,
                  labels: [],
                  assignees: [],
                  milestone: null,
                  linkedIssues: [],
                  mergedAt: null,
                  updatedAt: "2026-02-14T00:00:00Z",
                  reviews: [],
                  reviewComments: [],
                  checkSuites: [],
                  changedFilesCount: 0,
                  additions: 0,
                  deletions: 0,
                },
              },
              ghStatus: { available: true, authenticated: true },
            });
          }
          return new Promise<PrStatusResponse>((resolve) => {
            resolveProjectB = resolve;
          });
        }
        if (command === "fetch_pr_detail") {
          return Promise.resolve({
            number: 111,
            state: "OPEN",
            title: "A",
            url: "https://example.invalid/pr/111",
            mergeable: "MERGEABLE",
            draft: false,
            author: "owner",
            baseBranch: "main",
            headBranch: branchFixture.name,
            labels: [],
            assignees: [],
            milestone: null,
            linkedIssues: [],
            mergedAt: null,
            updatedAt: "2026-02-14T00:00:00Z",
            reviews: [],
            reviewComments: [],
            checkSuites: [],
            changedFilesCount: 0,
            additions: 0,
            deletions: 0,
          });
        }
        return Promise.resolve([]);
      });

      const rendered = await renderSidebar({
        projectPath: "/tmp/project-a",
        onBranchSelect: vi.fn(),
        selectedBranch: branchFixture,
      });

      await waitFor(() => {
        expect(countInvokeCalls("fetch_pr_status")).toBeGreaterThan(0);
      });
      expect(rendered.container.querySelector(".branch-list .branch-name")).toBeTruthy();
      const tabs = rendered.container.querySelectorAll(".summary-tab");
      const prTab = tabs[2] as HTMLElement;
      await fireEvent.click(prTab);
      await rendered.findByText("#111 A");

      await rendered.rerender({ projectPath: "/tmp/project-b" });
      await rendered.findByText(branchFixture.name);
      await waitFor(() => {
        expect(rendered.queryByText("#111 A")).toBeNull();
      });
    } finally {
      const finalizeProjectB = resolveProjectB as unknown as
        | ((value: PrStatusResponse) => void)
        | null;
      if (typeof finalizeProjectB === "function") {
        finalizeProjectB({
          statuses: {},
          ghStatus: { available: true, authenticated: true },
        });
      }
    }
  });

  it("bootstraps fetch_pr_status as soon as branches load", async () => {
    let resolveBranches: ((value: typeof branchFixture[]) => void) | null = null;
    try {
      invokeMock.mockImplementation((command: string) => {
        if (command === "list_worktree_branches") {
          return new Promise<typeof branchFixture[]>((resolve) => {
            resolveBranches = resolve;
          });
        }
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
      expect(countInvokeCalls("fetch_pr_status")).toBe(0);

      await waitFor(() => {
        expect(resolveBranches).toBeTruthy();
      });
      const finalizeBranches = resolveBranches as unknown as
        | ((value: typeof branchFixture[]) => void)
        | null;
      if (typeof finalizeBranches === "function") {
        finalizeBranches([branchFixture]);
      }
      await rendered.findByText(branchFixture.name);
      await waitFor(() => {
        expect(countInvokeCalls("fetch_pr_status")).toBeGreaterThan(0);
      });
    } finally {
      const finalizeBranches = resolveBranches as unknown as
        | ((value: typeof branchFixture[]) => void)
        | null;
      if (typeof finalizeBranches === "function") {
        finalizeBranches([branchFixture]);
      }
    }
  });

  it("skips 30s fetch_pr_status polling while search input is focused", async () => {
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

      await waitFor(() => {
        expect(countInvokeCalls("list_worktree_branches")).toBeGreaterThan(0);
      });
      const beforePollCallCount = countInvokeCalls("fetch_pr_status");
      await vi.advanceTimersByTimeAsync(30_000);
      await waitFor(() => {
        expect(countInvokeCalls("fetch_pr_status")).toBeGreaterThan(beforePollCallCount);
      });
      await waitFor(() => {
        expect(
          rendered.container.querySelectorAll(".branch-list .branch-item").length
        ).toBe(1);
      });
      const searchInput = rendered.getByPlaceholderText("Filter branches...");
      (searchInput as HTMLInputElement).focus();
      expect(document.activeElement).toBe(searchInput);

      invokeMock.mockClear();
      await vi.advanceTimersByTimeAsync(30_000);
      expect(countInvokeCalls("fetch_pr_status")).toBe(0);

      // Move focus away from search input so polling can resume.
      (searchInput as HTMLInputElement).blur();
      expect(document.activeElement).not.toBe(searchInput);
      await vi.advanceTimersByTimeAsync(30_000);
      expect(countInvokeCalls("fetch_pr_status")).toBeGreaterThan(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("applies branch filtering after debounce delay", async () => {
    vi.useFakeTimers();
    try {
      invokeMock.mockImplementation(async (command: string) => {
        if (command === "list_worktree_branches") {
          return [
            branchFixture,
            {
              ...branchFixture,
              name: "feature/another-branch",
            },
          ];
        }
        if (command === "list_worktrees") return [];
        return [];
      });

      const rendered = await renderSidebar({
        projectPath: "/tmp/project",
        onBranchSelect: vi.fn(),
      });

      await rendered.findByText("feature/sidebar-size");
      await rendered.findByText("feature/another-branch");

      const searchInput = rendered.getByPlaceholderText("Filter branches...");
      await fireEvent.input(searchInput, { target: { value: "another" } });

      expect(rendered.queryByText("feature/sidebar-size")).toBeTruthy();

      await vi.advanceTimersByTimeAsync(150);
      await waitFor(() => {
        expect(rendered.queryByText("feature/sidebar-size")).toBeNull();
      });
      expect(rendered.queryByText("feature/another-branch")).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });

  it("sorts branch list by name by default with main/develop prioritized", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches")
        return [
          { ...featureBetaBranchFixture },
          { ...mainBranchFixture },
          { ...developBranchFixture },
          { ...featureAlphaBranchFixture },
        ];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText(mainBranchFixture.name);
    await waitFor(() => {
      expect(getRenderedBranchNames(rendered)).toEqual([
        "main",
        "develop",
        "feature/alpha",
        "feature/beta",
      ]);
    });
  });

  it("sorts branch list by latest commit timestamp in updated mode", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches")
        return [
          { ...featureBetaBranchFixture },
          { ...mainBranchFixture },
          { ...developBranchFixture },
          { ...featureAlphaBranchFixture },
        ];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    const sortButton = rendered.getByRole("button", { name: "Sort mode" });
    await fireEvent.click(sortButton);

    await waitFor(() => {
      expect(getRenderedBranchNames(rendered)).toEqual([
        "main",
        "develop",
        "feature/alpha",
        "feature/beta",
      ]);
    });
  });

  it("cycles branch sort mode when pressing the sort button", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches")
        return [
          { ...branchFixture, name: "feature/zulu", commit_timestamp: 200 },
          { ...mainBranchFixture },
          { ...developBranchFixture },
          { ...branchFixture, name: "feature/beta", commit_timestamp: 500 },
          { ...branchFixture, name: "feature/alpha", commit_timestamp: 300 },
        ];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText(mainBranchFixture.name);
    expect(getRenderedBranchNames(rendered)).toEqual([
      "main",
      "develop",
      "feature/alpha",
      "feature/beta",
      "feature/zulu",
    ]);

    const sortButton = rendered.getByRole("button", { name: "Sort mode" });
    const sortModeText = sortButton.querySelector(".sort-mode-text");
    expect(sortModeText?.textContent).toBe("Name");

    await fireEvent.click(sortButton);
    await waitFor(() => {
      expect(sortModeText?.textContent).toBe("Updated");
      expect(getRenderedBranchNames(rendered)).toEqual([
        "main",
        "develop",
        "feature/beta",
        "feature/alpha",
        "feature/zulu",
      ]);
    });

    await fireEvent.click(sortButton);
    await waitFor(() => {
      expect(sortModeText?.textContent).toBe("Name");
      expect(getRenderedBranchNames(rendered)).toEqual([
        "main",
        "develop",
        "feature/alpha",
        "feature/beta",
        "feature/zulu",
      ]);
    });
  });

  it("puts branches with missing commit timestamp at the end in updated mode", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches")
        return [
          { ...developBranchFixture },
          { ...featureNoTimestampBranchFixture },
          { ...featureAlphaBranchFixture },
          { ...mainBranchFixture },
        ];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    const sortButton = rendered.getByRole("button", { name: "Sort mode" });
    await fireEvent.click(sortButton);

    await waitFor(() => {
      expect(getRenderedBranchNames(rendered)).toEqual([
        "main",
        "develop",
        "feature/alpha",
        "feature/no-timestamp",
      ]);
    });
  });

  it("sorts All mode with Local-side first and sort mode applied per side", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") {
        return [
          { ...featureBetaBranchFixture },
          { ...mainBranchFixture },
          { ...developBranchFixture },
        ];
      }
      if (command === "list_remote_branches") {
        return [
          { ...remoteBranchFixture, name: "origin/main", commit_timestamp: 1_700_000_200 },
          { ...remoteBranchFixture, name: "origin/feature/beta", commit_timestamp: 1_700_000_150 },
          { ...remoteBranchFixture, name: "origin/feature/alpha", commit_timestamp: 1_700_000_180 },
        ];
      }
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await fireEvent.click(rendered.getByRole("button", { name: "All" }));
    await rendered.findByText("origin/main");
    await rendered.findByText("origin/feature/alpha");

    expect(getRenderedBranchNames(rendered)).toEqual([
      "main",
      "develop",
      "feature/beta",
      "origin/main",
      "origin/feature/alpha",
      "origin/feature/beta",
    ]);
  });

  it("moves selected worktree with ArrowUp/ArrowDown keys", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches")
        return [
          { ...featureBetaBranchFixture },
          { ...mainBranchFixture },
          { ...developBranchFixture },
          { ...featureAlphaBranchFixture },
        ];
      if (command === "list_worktrees") return [];
      return [];
    });

    let selectedBranch = { ...mainBranchFixture, is_current: true };
    const onBranchSelect = vi.fn((next) => {
      selectedBranch = next;
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect,
      selectedBranch,
    });

    await rendered.findByText(mainBranchFixture.name);
    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".branch-list .branch-item").length).toBe(4);
    });

    const firstItem = rendered.container.querySelector<HTMLButtonElement>(
      'button[data-branch-name="main"]'
    );
    expect(firstItem).toBeTruthy();
    firstItem?.focus();

    await fireEvent.keyDown(firstItem as HTMLElement, { key: "ArrowDown" });
    await waitFor(() => expect(onBranchSelect).toHaveBeenCalledTimes(1));
    expect(onBranchSelect).toHaveBeenCalledWith(expect.objectContaining({ name: developBranchFixture.name }));
    await rendered.rerender({ selectedBranch });

    onBranchSelect.mockClear();
    const secondItem = rendered.container.querySelector<HTMLButtonElement>(
      'button[data-branch-name="develop"]'
    );
    expect(secondItem).toBeTruthy();
    secondItem?.focus();
    await fireEvent.keyDown(secondItem as HTMLElement, { key: "ArrowUp" });
    await waitFor(() => expect(onBranchSelect).toHaveBeenCalledTimes(1));
    expect(onBranchSelect).toHaveBeenCalledWith(expect.objectContaining({ name: mainBranchFixture.name }));

    onBranchSelect.mockClear();
    await rendered.rerender({ selectedBranch: featureBetaBranchFixture });
    const lastItem = rendered.container.querySelector<HTMLButtonElement>(
      'button[data-branch-name="feature/beta"]'
    );
    expect(lastItem).toBeTruthy();
    lastItem?.focus();
    await fireEvent.keyDown(lastItem as HTMLElement, { key: "ArrowDown" });
    await waitFor(() => expect(onBranchSelect).toHaveBeenCalledTimes(0));
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

  it("does not show PR/CI indicators in branch rows", async () => {
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
    expect(rendered.queryByText(/#42 Open/)).toBeNull();
    expect(rendered.queryByText("No PR")).toBeNull();
    expect(rendered.queryByText("GitHub not connected")).toBeNull();
    expect(rendered.queryByText("CI Build")).toBeNull();
    expect(rendered.queryByText("Lint")).toBeNull();
    expect(rendered.queryByTitle("Expand")).toBeNull();
    expect(onOpenCiLog).not.toHaveBeenCalled();
  });

  it("shows an animated indicator for branches with open agent tabs", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      agentTabBranches: [branchFixture.name],
    });

    const branchButton = (await rendered.findByText(branchFixture.name)).closest("button");
    expect(branchButton).toBeTruthy();
    expect(branchButton?.classList.contains("agent-active")).toBe(true);
    expect(rendered.queryByTitle("Agent tab is open for this branch")).toBeTruthy();
  });

  it("highlights selected branch in Worktree list", async () => {
    const currentBranch = {
      ...mainBranchFixture,
      is_current: true,
    };
    const selectedBranch = {
      ...developBranchFixture,
      is_current: false,
    };
    const onBranchSelect = vi.fn();

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") {
        return [currentBranch, selectedBranch];
      }
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect,
      selectedBranch,
    });

    await waitFor(() => {
      expect(
        rendered.container.querySelectorAll(".branch-list .branch-item").length
      ).toBe(2);
    });

    const branchButtons = Array.from(
      rendered.container.querySelectorAll<HTMLButtonElement>(".branch-list .branch-item")
    );
    const selectedButton = branchButtons.find(
      (button) => button.getAttribute("data-branch-name") === selectedBranch.name
    );
    const currentButton = branchButtons.find(
      (button) => button.getAttribute("data-branch-name") === currentBranch.name
    );

    expect(selectedButton).toBeTruthy();
    expect(selectedButton?.classList.contains("active")).toBe(true);
    expect(currentButton).toBeTruthy();
    expect(currentButton?.classList.contains("active")).toBe(false);
  });

  it("handles sidebar resize by pointer and keyboard", async () => {
    const onResize = vi.fn();
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      onResize,
      widthPx: 260,
      minWidthPx: 220,
      maxWidthPx: 520,
    });

    const resizeHandle = rendered.container.querySelector<HTMLButtonElement>(".resize-handle");
    expect(resizeHandle).toBeTruthy();

    await fireEvent.pointerDown(resizeHandle as HTMLElement, {
      button: 1,
      pointerId: 10,
      clientX: 100,
    });
    await fireEvent.pointerMove(window, { pointerId: 10, clientX: 180 });
    expect(onResize).not.toHaveBeenCalled();

    await fireEvent.pointerDown(resizeHandle as HTMLElement, {
      button: 0,
      pointerId: 1,
      clientX: 100,
    });
    expect(document.body.style.cursor).toBe("col-resize");
    expect(document.body.style.userSelect).toBe("none");

    await fireEvent.pointerMove(window, { pointerId: 2, clientX: 180 });
    expect(onResize).not.toHaveBeenCalled();

    await fireEvent.pointerMove(window, { pointerId: 1, clientX: 140 });
    expect(onResize).toHaveBeenCalledWith(300);

    await fireEvent.pointerUp(window, { pointerId: 2 });
    expect(document.body.style.cursor).toBe("col-resize");

    await fireEvent.pointerUp(window, { pointerId: 1 });
    expect(document.body.style.cursor).toBe("");
    expect(document.body.style.userSelect).toBe("");

    await fireEvent.keyDown(resizeHandle as HTMLElement, { key: "ArrowRight", shiftKey: true });
    await fireEvent.keyDown(resizeHandle as HTMLElement, { key: "ArrowLeft" });
    await fireEvent.keyDown(resizeHandle as HTMLElement, { key: "Home" });
    await fireEvent.keyDown(resizeHandle as HTMLElement, { key: "End" });
    await fireEvent.keyDown(resizeHandle as HTMLElement, { key: "Enter" });

    expect(onResize).toHaveBeenCalledWith(284);
    expect(onResize).toHaveBeenCalledWith(248);
    expect(onResize).toHaveBeenCalledWith(220);
    expect(onResize).toHaveBeenCalledWith(520);
  });

  it("handles summary resize by pointer and keyboard and persists height", async () => {
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    const summaryWrap = rendered.container.querySelector<HTMLElement>(".worktree-summary-wrap");
    const summaryResizeHandle = rendered.container.querySelector<HTMLButtonElement>(
      ".summary-resize-handle"
    );
    expect(summaryWrap).toBeTruthy();
    expect(summaryResizeHandle).toBeTruthy();
    expect(summaryWrap?.style.height).toBe("360px");

    await fireEvent.keyDown(summaryResizeHandle as HTMLElement, { key: "Enter" });
    expect(summaryWrap?.style.height).toBe("360px");

    await fireEvent.keyDown(summaryResizeHandle as HTMLElement, { key: "ArrowDown" });
    expect(summaryWrap?.style.height).toBe("344px");

    await fireEvent.keyDown(summaryResizeHandle as HTMLElement, { key: "ArrowUp", shiftKey: true });
    expect(summaryWrap?.style.height).toBe("376px");
    expect(window.localStorage.getItem("gwt.sidebar.worktreeSummaryHeight")).toBe("376");

    await fireEvent.pointerDown(summaryResizeHandle as HTMLElement, {
      button: 0,
      pointerId: 44,
      clientY: 200,
    });
    expect(document.body.style.cursor).toBe("row-resize");
    expect(document.body.style.userSelect).toBe("none");

    await fireEvent.pointerMove(window, { pointerId: 45, clientY: 220 });
    expect(summaryWrap?.style.height).toBe("376px");

    await fireEvent.pointerMove(window, { pointerId: 44, clientY: 232 });
    expect(summaryWrap?.style.height).toBe("344px");

    await fireEvent.pointerUp(window, { pointerId: 45 });
    expect(document.body.style.cursor).toBe("row-resize");

    await fireEvent.pointerUp(window, { pointerId: 44 });
    expect(document.body.style.cursor).toBe("");
    expect(document.body.style.userSelect).toBe("");
    expect(window.localStorage.getItem("gwt.sidebar.worktreeSummaryHeight")).toBe("344");
  });

  it("tracks cleanup-progress events and keeps deleting rows unselectable", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const onBranchSelect = vi.fn();
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect,
      selectedBranch: null,
    });

    const branchLabel = await rendered.findByText(branchFixture.name);
    const branchButton = branchLabel.closest("button");
    expect(branchButton).toBeTruthy();

    await waitFor(() =>
      expect(
        listenMock.mock.calls.some((call) => call[0] === "cleanup-progress")
      ).toBe(true)
    );

    await emitTauriEvent("cleanup-progress", {
      branch: branchFixture.name,
      status: "deleting",
    });
    await waitFor(() => {
      expect(branchButton?.classList.contains("deleting")).toBe(true);
      expect(rendered.container.querySelector(".safety-spinner")).toBeTruthy();
    });

    await fireEvent.click(branchButton as HTMLElement);
    expect(onBranchSelect).not.toHaveBeenCalled();

    await emitTauriEvent("cleanup-progress", {
      branch: branchFixture.name,
      status: "done",
    });
    await waitFor(() => {
      expect(branchButton?.classList.contains("deleting")).toBe(false);
      expect(rendered.container.querySelector(".safety-spinner")).toBeNull();
    });

  });

  it("opens cleanup confirmation from context menu and handles failure dialog", async () => {
    const warningBranch = {
      ...branchFixture,
      name: "feature/warning",
      ahead: 1,
      behind: 0,
      divergence_status: "Ahead",
      last_tool_usage: "claude@1.0.0",
    };

    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "list_worktree_branches") return [warningBranch];
      if (command === "list_worktrees") {
        return [
          {
            branch: warningBranch.name,
            has_changes: true,
            has_unpushed: false,
            safety_level: "warning",
          },
        ];
      }
      if (command === "cleanup_single_worktree") {
        const branch = (args?.branch as string) ?? "";
        throw new Error(`failed to cleanup ${branch}`);
      }
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    const branchLabel = await rendered.findByText(warningBranch.name);
    const branchButton = branchLabel.closest("button");
    expect(branchButton).toBeTruthy();
    expect(rendered.queryByText("claude@1.0.0")).toBeTruthy();
    expect(rendered.queryByText("+1")).toBeTruthy();

    await fireEvent.contextMenu(branchButton as HTMLElement);
    await fireEvent.click(await rendered.findByRole("button", { name: "Cleanup this branch" }));

    expect(rendered.queryByText("Delete Worktree")).toBeTruthy();
    expect(rendered.queryByText(/has uncommitted changes/)).toBeTruthy();

    await fireEvent.click(rendered.getByRole("button", { name: "Delete" }));
    await rendered.findByText("Delete Failed");
    expect(rendered.queryByText(/failed to cleanup/)).toBeTruthy();

    await fireEvent.click(rendered.getByRole("button", { name: "Close" }));
    await waitFor(() => {
      expect(rendered.queryByText("Delete Failed")).toBeNull();
    });
  });

  it("handles cleanup entry points and mode switching", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const onCleanupRequest = vi.fn();
    const onModeChange = vi.fn();
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      onCleanupRequest,
      onModeChange,
      mode: "branch",
      selectedBranch: branchFixture,
      currentBranch: "main",
    });

    const cleanupButton = rendered.getByRole("button", { name: "Cleanup" });
    await fireEvent.click(cleanupButton);
    expect(onCleanupRequest).toHaveBeenCalledWith();

    await waitFor(() => {
      expect(
        rendered.container.querySelector(`button[data-branch-name="${branchFixture.name}"]`)
      ).toBeTruthy();
    });
    const branchButton = rendered.container.querySelector(
      `button[data-branch-name="${branchFixture.name}"]`
    );
    expect(branchButton).toBeTruthy();
    await fireEvent.contextMenu(branchButton as HTMLElement);

    await new Promise((resolve) => setTimeout(resolve, 0));
    await fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => {
      expect(rendered.queryByRole("button", { name: "Cleanup Worktrees..." })).toBeNull();
    });

    await fireEvent.contextMenu(branchButton as HTMLElement);
    await fireEvent.click(await rendered.findByRole("button", { name: "Cleanup Worktrees..." }));
    expect(onCleanupRequest).toHaveBeenCalledWith(branchFixture.name);

    const modeButtons = rendered.container.querySelectorAll<HTMLButtonElement>(".mode-btn");
    expect(modeButtons.length).toBe(2);
    await fireEvent.click(modeButtons[1] as HTMLButtonElement);
    expect(onModeChange).toHaveBeenCalledWith("agent");

    await rendered.rerender({ mode: "agent" });
    await fireEvent.click(modeButtons[1] as HTMLButtonElement);
    expect(onModeChange).toHaveBeenCalledTimes(1);

    await fireEvent.click(modeButtons[0] as HTMLButtonElement);
    expect(onModeChange).toHaveBeenCalledWith("branch");
  });
});
