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

vi.mock("$lib/tauriInvoke", () => ({
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
  agent_status: "unknown" as const,
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
    vi.useRealTimers();
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
      expect(countInvokeCalls("list_worktree_branches")).toBeGreaterThanOrEqual(2);
      expect(countInvokeCalls("list_remote_branches")).toBeGreaterThanOrEqual(2);
      expect(countInvokeCalls("list_worktrees")).toBeGreaterThanOrEqual(2);
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
                  url: "https://example.invalid/pr/111",
                  mergeable: "MERGEABLE",
                  baseBranch: "main",
                  headBranch: branchFixture.name,
                  checkSuites: [],
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
          if (path === "/tmp/project-a") {
            return Promise.resolve({
              number: 111,
              title: "A",
              state: "OPEN",
              url: "https://example.invalid/pr/111",
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
              changedFilesCount: 0,
              additions: 0,
              deletions: 0,
            });
          }
          return Promise.resolve({
            number: 111,
            title: "A",
            state: "OPEN",
            url: "https://example.invalid/pr/111",
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

      await rendered.findByText(branchFixture.name);
      const prTab = rendered.container.querySelectorAll(".summary-tab")[3] as HTMLElement;
      await fireEvent.click(prTab);
      await rendered.findByText("#111 A");

      await waitFor(() => {
        expect(countInvokeCalls("fetch_pr_status")).toBeGreaterThan(0);
      });
      await rendered.rerender({ projectPath: "/tmp/project-b" });
      await rendered.findByText(branchFixture.name);
      await waitFor(() => {
        expect(rendered.queryByText("#111 A")).toBeNull();
        expect(rendered.queryByText("No PR")).toBeTruthy();
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

      await rendered.findByText(branchFixture.name);
      await vi.advanceTimersByTimeAsync(30_000);
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
      const preFocusPrStatusCalls = countInvokeCalls("fetch_pr_status");
      (searchInput as HTMLInputElement).focus();
      expect(document.activeElement).toBe(searchInput);

      await vi.advanceTimersByTimeAsync(30_000);
      expect(countInvokeCalls("fetch_pr_status")).toBe(preFocusPrStatusCalls);

      // Move focus away from search input so polling can resume.
      (searchInput as HTMLInputElement).blur();
      expect(document.activeElement).not.toBe(searchInput);
      await vi.advanceTimersByTimeAsync(30_000);
      await waitFor(() => {
        expect(countInvokeCalls("fetch_pr_status")).toBeGreaterThan(preFocusPrStatusCalls);
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("ignores visibility refresh while branch list has not been bootstrapped yet", async () => {
    vi.useFakeTimers();
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

      const hiddenState = { value: false };
      Object.defineProperty(document, "hidden", {
        configurable: true,
        get: () => hiddenState.value,
      });
      const activeState = { element: document.body as Element | null };
      Object.defineProperty(document, "activeElement", {
        configurable: true,
        get: () => activeState.element,
      });

      await renderSidebar({
        projectPath: "/tmp/project",
        onBranchSelect: vi.fn(),
      });

      document.dispatchEvent(new Event("visibilitychange"));
      await vi.advanceTimersByTimeAsync(50);
      expect(countInvokeCalls("fetch_pr_status")).toBe(0);
    } finally {
      const finalizeBranches = resolveBranches as unknown as
        | ((value: Array<typeof branchFixture>) => void)
        | null;
      if (typeof finalizeBranches === "function") {
        finalizeBranches([branchFixture]);
      }
      vi.useRealTimers();
    }
  });

  it("skips visibility-triggered refresh when input is focused or refresh is too recent", async () => {
    vi.useFakeTimers();
    try {
      invokeMock.mockImplementation((command: string) => {
        if (command === "list_worktree_branches") return Promise.resolve([branchFixture]);
        if (command === "list_worktrees") return Promise.resolve([]);
        if (command === "fetch_pr_status") {
          return Promise.resolve({
            statuses: {},
            ghStatus: { available: true, authenticated: true },
          });
        }
        return Promise.resolve([]);
      });

      const hiddenState = { value: false };
      Object.defineProperty(document, "hidden", {
        configurable: true,
        get: () => hiddenState.value,
      });
      const activeState = { element: document.body as Element | null };
      Object.defineProperty(document, "activeElement", {
        configurable: true,
        get: () => activeState.element,
      });

      const rendered = await renderSidebar({
        projectPath: "/tmp/project",
        onBranchSelect: vi.fn(),
      });

      await rendered.findByText(branchFixture.name);
      await waitFor(() => {
        expect(countInvokeCalls("fetch_pr_status")).toBeGreaterThan(0);
      });
      const searchInput = rendered.getByPlaceholderText("Filter branches...") as HTMLInputElement;

      const beforeFocusedRefresh = countInvokeCalls("fetch_pr_status");
      activeState.element = searchInput;
      document.dispatchEvent(new Event("visibilitychange"));
      await vi.advanceTimersByTimeAsync(20);
      const afterFocusedRefresh = countInvokeCalls("fetch_pr_status");
      expect(afterFocusedRefresh).toBeGreaterThanOrEqual(beforeFocusedRefresh);

      // Hidden path should only clear timer.
      hiddenState.value = true;
      document.dispatchEvent(new Event("visibilitychange"));
      await vi.advanceTimersByTimeAsync(20);
      const afterHiddenRefresh = countInvokeCalls("fetch_pr_status");
      expect(afterHiddenRefresh).toBeGreaterThanOrEqual(afterFocusedRefresh);

      // Too-recent refresh path should skip immediate refresh.
      hiddenState.value = false;
      activeState.element = document.body;
      document.dispatchEvent(new Event("visibilitychange"));
      await vi.advanceTimersByTimeAsync(20);
      const afterRecentRefresh = countInvokeCalls("fetch_pr_status");
      expect(afterRecentRefresh).toBeGreaterThanOrEqual(afterHiddenRefresh);

      // Once enough time has passed, visibility refresh runs.
      await vi.advanceTimersByTimeAsync(6000);
      document.dispatchEvent(new Event("visibilitychange"));
      await vi.advanceTimersByTimeAsync(30_000);
      expect(countInvokeCalls("fetch_pr_status")).toBeGreaterThanOrEqual(afterRecentRefresh);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps worktree list visible during 10s agent-status fallback polling", async () => {
    vi.useFakeTimers();
    try {
      invokeMock.mockImplementation((command: string) => {
        if (command === "list_worktree_branches") {
          return Promise.resolve([branchFixture]);
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
        agentTabBranches: [branchFixture.name],
      });

      await rendered.findByText(branchFixture.name);
      expect(rendered.container.querySelector(".loading-indicator")).toBeNull();
      const beforeRefresh = countInvokeCalls("list_worktree_branches");

      await vi.advanceTimersByTimeAsync(10_000);
      await waitFor(() => {
        expect(countInvokeCalls("list_worktree_branches")).toBeGreaterThan(beforeRefresh);
      });
      expect(rendered.container.querySelector(".loading-indicator")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps worktree list visible when agent-status-changed event triggers refresh", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_worktree_branches") {
        return Promise.resolve([branchFixture]);
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
      agentTabBranches: [branchFixture.name],
    });

    await rendered.findByText(branchFixture.name);
    await waitFor(() =>
      expect(listenMock.mock.calls.some((call) => call[0] === "agent-status-changed")).toBe(
        true
      )
    );
    expect(rendered.container.querySelector(".loading-indicator")).toBeNull();
    const beforeRefresh = countInvokeCalls("list_worktree_branches");

    await emitTauriEvent("agent-status-changed", {});
    await waitFor(() => {
      expect(countInvokeCalls("list_worktree_branches")).toBeGreaterThan(beforeRefresh);
    });
    expect(rendered.container.querySelector(".loading-indicator")).toBeNull();
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

  it("sorts branch list by updated timestamp by default with main/develop prioritized", async () => {
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

  it("sorts branch list by name after toggling sort mode", async () => {
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

    // Default is now "updated", so no click needed
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

    let selectedBranch = mainBranchFixture;
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
    await waitFor(() => {
      expect((continueButton as HTMLButtonElement).disabled).toBe(false);
    });
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
    await waitFor(() => {
      expect((newButton as HTMLButtonElement).disabled).toBe(false);
    });
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
        state: "OPEN",
        url: "https://github.com/test/pr/42",
        mergeable: "MERGEABLE",
        baseBranch: "main",
        headBranch: branchFixture.name,
        checkSuites: [
          { workflowName: "CI Build", runId: 100, status: "completed", conclusion: "success" },
        ],
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
    // Agent indicator slot should exist with agent-active class
    const indicatorSlot = branchButton?.querySelector(".agent-indicator-slot.agent-active");
    expect(indicatorSlot).toBeTruthy();
    expect(rendered.queryByTitle("Agent tab is open")).toBeTruthy();
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
      expect(rendered.container.querySelectorAll(".branch-item").length).toBe(2);
    });

    const branchButtons = Array.from(rendered.container.querySelectorAll(".branch-item"));
    const selectedButton = branchButtons.find((button) =>
      button.textContent?.includes(selectedBranch.name)
    ) as HTMLElement | undefined;
    const currentButton = branchButtons.find((button) =>
      button.textContent?.includes(currentBranch.name)
    ) as HTMLElement | undefined;

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

  it("opens CleanupModal via context menu 'Cleanup this branch' and does not call cleanup_single_worktree", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const onCleanupRequest = vi.fn();
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      onCleanupRequest,
    });

    const branchLabel = await rendered.findByText(branchFixture.name);
    const branchButton = branchLabel.closest("button");
    expect(branchButton).toBeTruthy();

    await fireEvent.contextMenu(branchButton as HTMLElement);
    await fireEvent.click(await rendered.findByRole("button", { name: "Cleanup this branch" }));

    expect(onCleanupRequest).toHaveBeenCalledWith(branchFixture.name);
    // cleanup_single_worktree should never be called
    expect(invokeMock).not.toHaveBeenCalledWith("cleanup_single_worktree", expect.anything());
  });

  it("renders agent-indicator-slot for branches without agent tabs", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      agentTabBranches: [],
    });

    const branchButton = (await rendered.findByText(branchFixture.name)).closest("button");
    expect(branchButton).toBeTruthy();
    const slot = branchButton?.querySelector(".agent-indicator-slot");
    expect(slot).toBeTruthy();
    expect(slot?.querySelector(".agent-static-dot")).toBeNull();
    expect(slot?.querySelector(".agent-pulse-dot")).toBeNull();
  });

  it("shows a static dot when agent_status is stopped and agent tab is open", async () => {
    const stoppedBranch = {
      ...branchFixture,
      agent_status: "stopped" as const,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [stoppedBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      agentTabBranches: [stoppedBranch.name],
    });

    const branchButton = (await rendered.findByText(stoppedBranch.name)).closest("button");
    expect(branchButton).toBeTruthy();
    const slot = branchButton?.querySelector(".agent-indicator-slot");
    expect(slot).toBeTruthy();
    expect(slot?.querySelector(".agent-static-dot")).toBeTruthy();
    expect(slot?.querySelector(".agent-pulse-dot")).toBeNull();
  });

  it("shows a static dot when agent_status is waiting_input and agent tab is open", async () => {
    const waitingBranch = {
      ...branchFixture,
      agent_status: "waiting_input" as const,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [waitingBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      agentTabBranches: [waitingBranch.name],
    });

    const branchButton = (await rendered.findByText(waitingBranch.name)).closest("button");
    expect(branchButton).toBeTruthy();
    const slot = branchButton?.querySelector(".agent-indicator-slot");
    expect(slot).toBeTruthy();
    expect(slot?.querySelector(".agent-static-dot")).toBeTruthy();
    expect(slot?.querySelector(".agent-pulse-dot")).toBeNull();
  });

  it("shows a pulse dot when agent_status is running and agent tab is open", async () => {
    const runningBranch = {
      ...branchFixture,
      agent_status: "running" as const,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [runningBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      agentTabBranches: [runningBranch.name],
    });

    const branchButton = (await rendered.findByText(runningBranch.name)).closest("button");
    expect(branchButton).toBeTruthy();
    const slot = branchButton?.querySelector(".agent-indicator-slot");
    expect(slot).toBeTruthy();
    expect(slot?.querySelector(".agent-pulse-dot")).toBeTruthy();
    expect(slot?.querySelector(".agent-static-dot")).toBeNull();
  });

  it("does not show any dot indicator when agent tab is not open for the branch", async () => {
    const runningBranch = {
      ...branchFixture,
      agent_status: "running" as const,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [runningBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      agentTabBranches: [],
    });

    const branchButton = (await rendered.findByText(runningBranch.name)).closest("button");
    expect(branchButton).toBeTruthy();
    const slot = branchButton?.querySelector(".agent-indicator-slot");
    expect(slot).toBeTruthy();
    expect(slot?.querySelector(".agent-static-dot")).toBeNull();
    expect(slot?.querySelector(".agent-pulse-dot")).toBeNull();
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
    expect(onModeChange).toHaveBeenCalledWith("projectMode");

    await rendered.rerender({ mode: "projectMode" });
    await fireEvent.click(modeButtons[1] as HTMLButtonElement);
    expect(onModeChange).toHaveBeenCalledTimes(1);

    await fireEvent.click(modeButtons[0] as HTMLButtonElement);
    expect(onModeChange).toHaveBeenCalledWith("branch");
  });

  it("updates pollingStatuses when pr-status-updated event is received (T008)", async () => {
    const branchA = {
      ...branchFixture,
      name: "feature/alpha",
      commit_timestamp: 1_700_000_090,
    };

    const initialPrStatus = {
      number: 42,
      state: "OPEN" as const,
      url: "https://github.com/test/repo/pull/42",
      mergeable: "UNKNOWN" as const,
      baseBranch: "main",
      headBranch: "feature/alpha",
      checkSuites: [],
      retrying: true,
    };

    const resolvedPrStatus = {
      number: 42,
      state: "OPEN" as const,
      url: "https://github.com/test/repo/pull/42",
      mergeable: "MERGEABLE" as const,
      baseBranch: "main",
      headBranch: "feature/alpha",
      checkSuites: [],
      retrying: false,
    };

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_worktree_branches") return [branchA];
      if (cmd === "list_worktrees") return [];
      if (cmd === "fetch_pr_status") {
        return {
          statuses: { "feature/alpha": initialPrStatus },
          ghStatus: { available: true, authenticated: true },
          repoKey: "/tmp/project",
        };
      }
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      refreshKey: 0,
      mode: "branch",
    });

    // Wait for initial PR polling to complete
    await waitFor(() => {
      const prBadge = rendered.container.querySelector(".pr-badge");
      expect(prBadge).toBeTruthy();
    });

    // Verify initial state shows retrying (pulse class)
    const initialBadge = rendered.container.querySelector(".pr-badge");
    expect(initialBadge?.classList.contains("pulse")).toBe(true);
    expect(initialBadge?.textContent?.trim()).toBe("#42");

    // Emit pr-status-updated event with resolved status
    await emitTauriEvent("pr-status-updated", {
      repoKey: "/tmp/project",
      branch: "feature/alpha",
      status: resolvedPrStatus,
    });

    // Verify that the pollingStatuses was updated (pulse should be gone)
    await waitFor(() => {
      const badge = rendered.container.querySelector(".pr-badge");
      expect(badge).toBeTruthy();
      expect(badge?.classList.contains("pulse")).toBe(false);
    });
  });

  it("does not update pollingStatuses when pr-status-updated event has retrying=true", async () => {
    const branchA = {
      ...branchFixture,
      name: "feature/beta",
      commit_timestamp: 1_700_000_080,
    };

    const initialPrStatus = {
      number: 55,
      state: "OPEN" as const,
      url: "https://github.com/test/repo/pull/55",
      mergeable: "UNKNOWN" as const,
      baseBranch: "main",
      headBranch: "feature/beta",
      checkSuites: [],
      retrying: true,
    };

    const stillRetryingStatus = {
      number: 55,
      state: "OPEN" as const,
      url: "https://github.com/test/repo/pull/55",
      mergeable: "UNKNOWN" as const,
      baseBranch: "main",
      headBranch: "feature/beta",
      checkSuites: [],
      retrying: true,
    };

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_worktree_branches") return [branchA];
      if (cmd === "list_worktrees") return [];
      if (cmd === "fetch_pr_status") {
        return {
          statuses: { "feature/beta": initialPrStatus },
          ghStatus: { available: true, authenticated: true },
          repoKey: "/tmp/project-beta",
        };
      }
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project-beta",
      onBranchSelect: vi.fn(),
      refreshKey: 0,
      mode: "branch",
    });

    await waitFor(() => {
      const prBadge = rendered.container.querySelector(".pr-badge");
      expect(prBadge).toBeTruthy();
    });

    // Should still be pulsing
    const initialBadge = rendered.container.querySelector(".pr-badge");
    expect(initialBadge?.classList.contains("pulse")).toBe(true);

    // Emit pr-status-updated with retrying=true (should be ignored)
    await emitTauriEvent("pr-status-updated", {
      repoKey: "/tmp/project-beta",
      branch: "feature/beta",
      status: stillRetryingStatus,
    });

    // Badge should still have pulse (retrying event was ignored)
    await new Promise((r) => setTimeout(r, 50));
    const badgeAfter = rendered.container.querySelector(".pr-badge");
    expect(badgeAfter?.classList.contains("pulse")).toBe(true);
  });

  it("does not update pollingStatuses when pr-status-updated event repoKey differs", async () => {
    const branchA = {
      ...branchFixture,
      name: "feature/gamma",
      commit_timestamp: 1_700_000_070,
    };

    const initialPrStatus = {
      number: 77,
      state: "OPEN" as const,
      url: "https://github.com/test/repo/pull/77",
      mergeable: "UNKNOWN" as const,
      baseBranch: "main",
      headBranch: "feature/gamma",
      checkSuites: [],
      retrying: true,
    };

    const resolvedPrStatus = {
      number: 77,
      state: "OPEN" as const,
      url: "https://github.com/test/repo/pull/77",
      mergeable: "MERGEABLE" as const,
      baseBranch: "main",
      headBranch: "feature/gamma",
      checkSuites: [],
      retrying: false,
    };

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_worktree_branches") return [branchA];
      if (cmd === "list_worktrees") return [];
      if (cmd === "fetch_pr_status") {
        return {
          statuses: { "feature/gamma": initialPrStatus },
          ghStatus: { available: true, authenticated: true },
          repoKey: "/tmp/project-gamma",
        };
      }
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project-gamma",
      onBranchSelect: vi.fn(),
      refreshKey: 0,
      mode: "branch",
    });

    await waitFor(() => {
      const prBadge = rendered.container.querySelector(".pr-badge");
      expect(prBadge).toBeTruthy();
    });

    const initialBadge = rendered.container.querySelector(".pr-badge");
    expect(initialBadge?.classList.contains("pulse")).toBe(true);

    await emitTauriEvent("pr-status-updated", {
      repoKey: "/tmp/another-project",
      branch: "feature/gamma",
      status: resolvedPrStatus,
    });

    await new Promise((r) => setTimeout(r, 50));
    const badgeAfter = rendered.container.querySelector(".pr-badge");
    expect(badgeAfter?.classList.contains("pulse")).toBe(true);
  });

  it("shows tool usage badge when branch has last_tool_usage", async () => {
    const toolBranch = {
      ...branchFixture,
      name: "feature/with-tool",
      last_tool_usage: "claude@3.5",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [toolBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/with-tool");
    const toolUsageBadge = rendered.container.querySelector(".tool-usage");
    expect(toolUsageBadge).toBeTruthy();
    expect(toolUsageBadge?.textContent?.trim()).toBe("claude@3.5");
    expect(toolUsageBadge?.classList.contains("claude")).toBe(true);
  });

  it("shows codex tool usage class for codex@ prefix", async () => {
    const codexBranch = {
      ...branchFixture,
      name: "feature/codex-tool",
      last_tool_usage: "codex@1.0",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [codexBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/codex-tool");
    const toolUsageBadge = rendered.container.querySelector(".tool-usage");
    expect(toolUsageBadge).toBeTruthy();
    expect(toolUsageBadge?.classList.contains("codex")).toBe(true);
  });

  it("shows gemini tool usage class for gemini@ prefix", async () => {
    const geminiBranch = {
      ...branchFixture,
      name: "feature/gemini-tool",
      last_tool_usage: "gemini@2.0",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [geminiBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/gemini-tool");
    const toolUsageBadge = rendered.container.querySelector(".tool-usage");
    expect(toolUsageBadge).toBeTruthy();
    expect(toolUsageBadge?.classList.contains("gemini")).toBe(true);
  });

  it("shows opencode tool usage class for opencode@ prefix", async () => {
    const opencodeBranch = {
      ...branchFixture,
      name: "feature/opencode-tool",
      last_tool_usage: "opencode@1.0",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [opencodeBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/opencode-tool");
    const toolUsageBadge = rendered.container.querySelector(".tool-usage");
    expect(toolUsageBadge).toBeTruthy();
    expect(toolUsageBadge?.classList.contains("opencode")).toBe(true);
  });

  it("shows open-code tool usage class for open-code@ prefix", async () => {
    const openCodeBranch = {
      ...branchFixture,
      name: "feature/open-code-tool",
      last_tool_usage: "open-code@1.0",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [openCodeBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/open-code-tool");
    const toolUsageBadge = rendered.container.querySelector(".tool-usage");
    expect(toolUsageBadge).toBeTruthy();
    expect(toolUsageBadge?.classList.contains("opencode")).toBe(true);
  });

  it("shows no tool usage class for unknown tool prefix", async () => {
    const unknownToolBranch = {
      ...branchFixture,
      name: "feature/unknown-tool",
      last_tool_usage: "sometool@1.0",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [unknownToolBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/unknown-tool");
    const toolUsageBadge = rendered.container.querySelector(".tool-usage");
    expect(toolUsageBadge).toBeTruthy();
    expect(toolUsageBadge?.classList.contains("claude")).toBe(false);
    expect(toolUsageBadge?.classList.contains("codex")).toBe(false);
    expect(toolUsageBadge?.classList.contains("gemini")).toBe(false);
    expect(toolUsageBadge?.classList.contains("opencode")).toBe(false);
  });

  it("does not show tool usage badge when branch has no last_tool_usage", async () => {
    const noToolBranch = {
      ...branchFixture,
      name: "feature/no-tool",
      last_tool_usage: null,
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [noToolBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/no-tool");
    const toolUsageBadge = rendered.container.querySelector(".tool-usage");
    expect(toolUsageBadge).toBeNull();
  });

  it("shows divergence indicator for Ahead status", async () => {
    const aheadBranch = {
      ...branchFixture,
      name: "feature/ahead",
      divergence_status: "Ahead",
      ahead: 3,
      behind: 0,
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [aheadBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/ahead");
    const divergence = rendered.container.querySelector(".divergence");
    expect(divergence).toBeTruthy();
    expect(divergence?.textContent?.trim()).toBe("+3");
    expect(divergence?.classList.contains("ahead")).toBe(true);
  });

  it("shows divergence indicator for Behind status", async () => {
    const behindBranch = {
      ...branchFixture,
      name: "feature/behind",
      divergence_status: "Behind",
      ahead: 0,
      behind: 5,
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [behindBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/behind");
    const divergence = rendered.container.querySelector(".divergence");
    expect(divergence).toBeTruthy();
    expect(divergence?.textContent?.trim()).toBe("-5");
    expect(divergence?.classList.contains("behind")).toBe(true);
  });

  it("shows divergence indicator for Diverged status", async () => {
    const divergedBranch = {
      ...branchFixture,
      name: "feature/diverged",
      divergence_status: "Diverged",
      ahead: 2,
      behind: 4,
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [divergedBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/diverged");
    const divergence = rendered.container.querySelector(".divergence");
    expect(divergence).toBeTruthy();
    expect(divergence?.textContent?.trim()).toBe("+2 -4");
    expect(divergence?.classList.contains("diverged")).toBe(true);
  });

  it("does not show divergence indicator for UpToDate status", async () => {
    const upToDateBranch = {
      ...branchFixture,
      name: "feature/uptodate",
      divergence_status: "UpToDate",
      ahead: 0,
      behind: 0,
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [upToDateBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/uptodate");
    const divergence = rendered.container.querySelector(".divergence");
    expect(divergence).toBeNull();
  });

  it("shows safety dot with warning level", async () => {
    const warningBranch = {
      ...branchFixture,
      name: "feature/warning-branch",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [warningBranch];
      if (command === "list_worktrees") return [
        {
          path: "/tmp/project/feature/warning-branch",
          branch: "feature/warning-branch",
          safety_level: "warning",
          is_protected: false,
          is_current: false,
        },
      ];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/warning-branch");
    const safetyDot = rendered.container.querySelector(".safety-dot.warning");
    expect(safetyDot).toBeTruthy();
    expect(safetyDot?.getAttribute("title")).toBe("Has uncommitted changes or unpushed commits");
  });

  it("shows safety dot with safe level", async () => {
    const safeBranch = {
      ...branchFixture,
      name: "feature/safe-branch",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [safeBranch];
      if (command === "list_worktrees") return [
        {
          path: "/tmp/project/feature/safe-branch",
          branch: "feature/safe-branch",
          safety_level: "safe",
          is_protected: false,
          is_current: false,
        },
      ];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/safe-branch");
    const safetyDot = rendered.container.querySelector(".safety-dot.safe");
    expect(safetyDot).toBeTruthy();
    expect(safetyDot?.getAttribute("title")).toBe("Safe to delete");
  });

  it("shows safety dot with danger level", async () => {
    const dangerBranch = {
      ...branchFixture,
      name: "feature/danger-branch",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [dangerBranch];
      if (command === "list_worktrees") return [
        {
          path: "/tmp/project/feature/danger-branch",
          branch: "feature/danger-branch",
          safety_level: "danger",
          is_protected: false,
          is_current: false,
        },
      ];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/danger-branch");
    const safetyDot = rendered.container.querySelector(".safety-dot.danger");
    expect(safetyDot).toBeTruthy();
    expect(safetyDot?.getAttribute("title")).toBe("Has uncommitted changes and unpushed commits");
  });

  it("shows safety dot with disabled level for protected branches", async () => {
    const disabledBranch = {
      ...branchFixture,
      name: "feature/protected-branch",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [disabledBranch];
      if (command === "list_worktrees") return [
        {
          path: "/tmp/project/feature/protected-branch",
          branch: "feature/protected-branch",
          safety_level: "disabled",
          is_protected: true,
          is_current: false,
        },
      ];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText("feature/protected-branch");
    const safetyDot = rendered.container.querySelector(".safety-dot.disabled");
    expect(safetyDot).toBeTruthy();
    expect(safetyDot?.getAttribute("title")).toBe("Protected or current branch");
  });

  it("shows PR badge with merged state", async () => {
    const mergedBranch = {
      ...branchFixture,
      name: "feature/merged-pr",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_worktree_branches") return [mergedBranch];
      if (cmd === "list_worktrees") return [];
      if (cmd === "fetch_pr_status") {
        return {
          statuses: { "feature/merged-pr": {
            number: 99,
            state: "MERGED",
            url: "https://github.com/test/repo/pull/99",
            mergeable: "UNKNOWN",
            baseBranch: "main",
            headBranch: "feature/merged-pr",
            checkSuites: [],
            retrying: false,
          } },
          ghStatus: { available: true, authenticated: true },
          repoKey: "/tmp/project",
        };
      }
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await waitFor(() => {
      const prBadge = rendered.container.querySelector(".pr-badge");
      expect(prBadge).toBeTruthy();
    });

    const prBadge = rendered.container.querySelector(".pr-badge");
    expect(prBadge?.classList.contains("merged")).toBe(true);
    expect(prBadge?.textContent?.trim()).toBe("#99");
  });

  it("shows PR badge with closed state", async () => {
    const closedBranch = {
      ...branchFixture,
      name: "feature/closed-pr",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_worktree_branches") return [closedBranch];
      if (cmd === "list_worktrees") return [];
      if (cmd === "fetch_pr_status") {
        return {
          statuses: { "feature/closed-pr": {
            number: 88,
            state: "CLOSED",
            url: "https://github.com/test/repo/pull/88",
            mergeable: "UNKNOWN",
            baseBranch: "main",
            headBranch: "feature/closed-pr",
            checkSuites: [],
            retrying: false,
          } },
          ghStatus: { available: true, authenticated: true },
          repoKey: "/tmp/project",
        };
      }
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await waitFor(() => {
      const prBadge = rendered.container.querySelector(".pr-badge");
      expect(prBadge).toBeTruthy();
    });

    const prBadge = rendered.container.querySelector(".pr-badge");
    expect(prBadge?.classList.contains("closed")).toBe(true);
    expect(prBadge?.textContent?.trim()).toBe("#88");
  });

  it("shows PR badge with conflicting state", async () => {
    const conflictBranch = {
      ...branchFixture,
      name: "feature/conflict-pr",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_worktree_branches") return [conflictBranch];
      if (cmd === "list_worktrees") return [];
      if (cmd === "fetch_pr_status") {
        return {
          statuses: { "feature/conflict-pr": {
            number: 77,
            state: "OPEN",
            url: "https://github.com/test/repo/pull/77",
            mergeable: "CONFLICTING",
            baseBranch: "main",
            headBranch: "feature/conflict-pr",
            checkSuites: [],
            retrying: false,
          } },
          ghStatus: { available: true, authenticated: true },
          repoKey: "/tmp/project",
        };
      }
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await waitFor(() => {
      const prBadge = rendered.container.querySelector(".pr-badge");
      expect(prBadge).toBeTruthy();
    });

    const prBadge = rendered.container.querySelector(".pr-badge");
    expect(prBadge?.classList.contains("conflicting")).toBe(true);
    expect(prBadge?.textContent?.trim()).toBe("#77");
  });

  it("shows PR badge with unknown mergeable state", async () => {
    const unknownBranch = {
      ...branchFixture,
      name: "feature/unknown-pr",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_worktree_branches") return [unknownBranch];
      if (cmd === "list_worktrees") return [];
      if (cmd === "fetch_pr_status") {
        return {
          statuses: { "feature/unknown-pr": {
            number: 66,
            state: "OPEN",
            url: "https://github.com/test/repo/pull/66",
            mergeable: "UNKNOWN",
            baseBranch: "main",
            headBranch: "feature/unknown-pr",
            checkSuites: [],
            retrying: false,
          } },
          ghStatus: { available: true, authenticated: true },
          repoKey: "/tmp/project",
        };
      }
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await waitFor(() => {
      const prBadge = rendered.container.querySelector(".pr-badge");
      expect(prBadge).toBeTruthy();
    });

    const prBadge = rendered.container.querySelector(".pr-badge");
    expect(prBadge?.classList.contains("unknown")).toBe(true);
    expect(prBadge?.textContent?.trim()).toBe("#66");
  });

  it("does not show agent indicator for Remote filter branches", async () => {
    const remoteBranch = {
      ...branchFixture,
      name: "origin/feature/sidebar-size",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_remote_branches") return [remoteBranch];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
      agentTabBranches: ["feature/sidebar-size"],
    });

    // Switch to Remote filter
    const remoteButton = rendered.getByRole("button", { name: "Remote" });
    await fireEvent.click(remoteButton);

    await rendered.findByText("origin/feature/sidebar-size");
    const slot = rendered.container.querySelector(".agent-indicator-slot.agent-active");
    expect(slot).toBeNull();
  });

  it("handles double-click on branch to activate", async () => {
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

    await fireEvent.dblClick(branchButton as HTMLElement);
    expect(onBranchActivate).toHaveBeenCalledWith(branchFixture);
  });

  it("does not activate branch on double-click when branch is being deleted", async () => {
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

    // Mark branch as deleting
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
    });

    await fireEvent.dblClick(branchButton as HTMLElement);
    expect(onBranchActivate).not.toHaveBeenCalled();
  });

  it("shows PR badge with open mergeable state", async () => {
    const openBranch = {
      ...branchFixture,
      name: "feature/open-pr",
      commit_timestamp: 1_700_000_090,
    };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_worktree_branches") return [openBranch];
      if (cmd === "list_worktrees") return [];
      if (cmd === "fetch_pr_status") {
        return {
          statuses: { "feature/open-pr": {
            number: 55,
            state: "OPEN",
            url: "https://github.com/test/repo/pull/55",
            mergeable: "MERGEABLE",
            baseBranch: "main",
            headBranch: "feature/open-pr",
            checkSuites: [],
            retrying: false,
          } },
          ghStatus: { available: true, authenticated: true },
          repoKey: "/tmp/project",
        };
      }
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await waitFor(() => {
      const prBadge = rendered.container.querySelector(".pr-badge");
      expect(prBadge).toBeTruthy();
    });

    const prBadge = rendered.container.querySelector(".pr-badge");
    expect(prBadge?.classList.contains("open")).toBe(true);
    expect(prBadge?.textContent?.trim()).toBe("#55");
  });

  it("clears branches on cleanup-completed event", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_worktree_branches") return [branchFixture];
      if (command === "list_worktrees") return [];
      return [];
    });

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    await rendered.findByText(branchFixture.name);

    // Trigger cleanup-completed event
    await waitFor(() =>
      expect(
        listenMock.mock.calls.some((call) => call[0] === "cleanup-completed")
      ).toBe(true)
    );
    await emitTauriEvent("cleanup-completed", {});

    // Should trigger a re-fetch
    await waitFor(() => {
      expect(countInvokeCalls("list_worktree_branches")).toBeGreaterThan(1);
    });
  });

  it("ignores non-left-click on summary resize handle", async () => {
    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect: vi.fn(),
    });

    const summaryResizeHandle = rendered.container.querySelector<HTMLButtonElement>(
      ".summary-resize-handle"
    );
    expect(summaryResizeHandle).toBeTruthy();

    // Right click should not start summary resize
    await fireEvent.pointerDown(summaryResizeHandle as HTMLElement, {
      button: 2,
      pointerId: 99,
      clientY: 200,
    });
    expect(document.body.style.cursor).not.toBe("row-resize");
  });
});
