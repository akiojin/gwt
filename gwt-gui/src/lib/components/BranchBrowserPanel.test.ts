import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import BranchBrowserPanel from "./BranchBrowserPanel.svelte";
import type {
  BranchBrowserPanelConfig,
  BranchInfo,
  BranchInventoryDetail,
  BranchInventorySnapshotEntry,
  WorktreeInfo,
} from "../types";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const localBranch: BranchInfo = {
  name: "feature/local",
  display_name: null,
  commit: "abc1234",
  is_current: false,
  is_agent_running: false,
  agent_status: "unknown",
  ahead: 1,
  behind: 0,
  divergence_status: "Ahead",
  commit_timestamp: 1_700_000_000_000,
  last_tool_usage: "codex@latest",
};

const remoteBranch: BranchInfo = {
  name: "origin/feature/remote",
  display_name: null,
  commit: "def5678",
  is_current: false,
  is_agent_running: false,
  agent_status: "unknown",
  ahead: 0,
  behind: 0,
  divergence_status: "UpToDate",
  commit_timestamp: 1_700_000_000_500,
  last_tool_usage: null,
};

const worktree: WorktreeInfo = {
  path: "/tmp/project/.gwt/worktrees/feature-local",
  branch: "feature/local",
  commit: "abc1234",
  status: "active",
  is_main: false,
  has_changes: false,
  has_unpushed: true,
  is_current: false,
  is_protected: false,
  is_agent_running: false,
  agent_status: "unknown",
  ahead: 1,
  behind: 0,
  is_gone: false,
  last_tool_usage: "codex@latest",
  safety_level: "warning",
};

const localEntry: BranchInventorySnapshotEntry = {
  id: "feature/local",
  canonical_name: "feature/local",
  primary_branch: localBranch,
  local_branch: localBranch,
  remote_branch: null,
  has_local: true,
  has_remote: false,
  worktree_path: worktree.path,
  worktree_count: 1,
  resolution_action: "focusExisting",
};

const remoteEntry: BranchInventorySnapshotEntry = {
  id: "feature/remote",
  canonical_name: "feature/remote",
  primary_branch: remoteBranch,
  local_branch: null,
  remote_branch: remoteBranch,
  has_local: false,
  has_remote: true,
  worktree_path: null,
  worktree_count: 0,
  resolution_action: "createWorktree",
};

const localDetail: BranchInventoryDetail = {
  ...localEntry,
  primary_branch: {
    ...localBranch,
    display_name: "Local feature",
    last_tool_usage: "codex@latest",
  },
  local_branch: {
    ...localBranch,
    display_name: "Local feature",
    last_tool_usage: "codex@latest",
  },
  remote_branch: null,
  worktree_path: worktree.path,
};

const remoteDetail: BranchInventoryDetail = {
  ...remoteEntry,
  primary_branch: {
    ...remoteBranch,
    display_name: "Remote feature",
  },
  local_branch: null,
  remote_branch: {
    ...remoteBranch,
    display_name: "Remote feature",
  },
  worktree_path: null,
};

function createConfig(overrides: Partial<BranchBrowserPanelConfig> = {}): BranchBrowserPanelConfig {
  return {
    projectPath: "/tmp/project",
    refreshKey: 0,
    currentBranch: "feature/local",
    agentTabBranches: [],
    activeAgentTabBranch: null,
    appLanguage: "en",
    onBranchSelect: vi.fn(),
    ...overrides,
  };
}

describe("BranchBrowserPanel", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_branch_inventory") {
        return Promise.resolve([localEntry, remoteEntry]);
      }
      if (command === "get_branch_inventory_detail") {
        return Promise.resolve(localDetail);
      }
      return Promise.resolve([]);
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("loads a minimal snapshot first, then hydrates selected branch detail", async () => {
    const onBranchSelect = vi.fn();
    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({
          selectedBranchName: localEntry.canonical_name,
          onBranchSelect,
        }),
      },
    });

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("list_branch_inventory", {
        projectPath: "/tmp/project",
        refreshKey: 0,
      }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("get_branch_inventory_detail", {
        projectPath: "/tmp/project",
        canonicalName: localEntry.canonical_name,
        forceRefresh: false,
      }),
    );
    await waitFor(() =>
      expect(rendered.container.querySelector(".detail-title")?.textContent).toBe(
        "Local feature",
      ),
    );
    expect(invokeMock).toHaveBeenCalledWith("list_branch_inventory", {
      projectPath: "/tmp/project",
      refreshKey: 0,
    });
    expect(rendered.getByTestId("branch-browser-detail").textContent).toContain(
      "/tmp/project/.gwt/worktrees/feature-local",
    );
  });

  it("switches to Remote mode and renders remote refs", async () => {
    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig(),
      },
    });

    await waitFor(() =>
      expect(rendered.container.querySelector(".branch-row")).toBeTruthy(),
    );
    await fireEvent.click(rendered.getByText("Remote"));

    await waitFor(() =>
      expect(rendered.container.querySelector(".branch-row")?.textContent).toContain(
        "origin/feature/remote",
      ),
    );
    expect(rendered.getByTestId("branch-browser-detail").textContent).not.toContain(
        "origin/feature/remote",
    );
  });

  it("merges local and remote refs into one canonical entry in All mode", async () => {
    const mergedEntry: BranchInventorySnapshotEntry = {
      ...localEntry,
      has_remote: true,
      remote_branch: {
        ...remoteBranch,
        name: "origin/feature/local",
      },
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_branch_inventory") return Promise.resolve([mergedEntry]);
      return Promise.resolve([]);
    });

    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({
          selectedBranchName: mergedEntry.canonical_name,
        }),
      },
    });

    await waitFor(() =>
      expect(rendered.container.querySelector(".branch-row")).toBeTruthy(),
    );
    await fireEvent.click(rendered.getByText("All"));

    await waitFor(() => expect(rendered.getByText("Local + Remote")).toBeTruthy());
    expect(rendered.container.querySelectorAll(".branch-row")).toHaveLength(1);
  });

  it("forwards branch selection to the host shell", async () => {
    const onBranchSelect = vi.fn();
    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({ onBranchSelect }),
      },
    });

    await waitFor(() =>
      expect(rendered.container.querySelector(".branch-row")).toBeTruthy(),
    );
    await fireEvent.click(
      rendered.container.querySelector(".branch-row") as HTMLButtonElement,
    );

    expect(onBranchSelect).toHaveBeenCalledWith(localBranch);
  });

  it("does not refetch when non-refresh config props change", async () => {
    const countListCalls = () =>
      invokeMock.mock.calls.filter(
        ([command]) => command === "list_branch_inventory",
      ).length;
    invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "list_branch_inventory") {
        return Promise.resolve([localEntry, remoteEntry]);
      }
      if (command === "get_branch_inventory_detail") {
        return Promise.resolve(
          args?.canonicalName === remoteEntry.canonical_name ? remoteDetail : localDetail,
        );
      }
      return Promise.resolve([]);
    });

    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({
          selectedBranch: localBranch,
        }),
      },
    });

    await waitFor(() =>
      expect(rendered.container.querySelector(".detail-title")?.textContent).toBe(
        "Local feature",
      ),
    );
    expect(countListCalls()).toBe(1);

    await rendered.rerender({
      config: createConfig({
        selectedBranch: remoteBranch,
        refreshKey: 0,
      }),
    });

    await waitFor(() =>
      expect(rendered.getByTestId("branch-browser-detail").textContent).toContain(
        "origin/feature/remote",
      ),
    );
    expect(countListCalls()).toBe(1);
  });

  it("clears stale selected detail immediately when switching selected branch", async () => {
    invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "list_branch_inventory") {
        return Promise.resolve([localEntry, remoteEntry]);
      }
      if (command === "get_branch_inventory_detail") {
        return Promise.resolve(
          args?.canonicalName === remoteEntry.canonical_name ? remoteDetail : localDetail,
        );
      }
      return Promise.resolve([]);
    });

    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({
          selectedBranch: localBranch,
        }),
      },
    });

    await waitFor(() =>
      expect(rendered.container.querySelector(".detail-title")?.textContent).toBe(
        "Local feature",
      ),
    );

    await rendered.rerender({
      config: createConfig({
        selectedBranch: remoteBranch,
        refreshKey: 0,
      }),
    });

    expect(rendered.getByTestId("branch-browser-detail").textContent).not.toContain(
      "Local feature",
    );
  });

  it("forwards open/focus worktree action for the selected branch", async () => {
    const onBranchActivate = vi.fn();
    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({
          selectedBranchName: localEntry.canonical_name,
          onBranchActivate,
        }),
      },
    });

    await waitFor(() =>
      expect(rendered.getByRole("button", { name: "Focus Worktree" })).toBeTruthy(),
    );
    await fireEvent.click(rendered.getByRole("button", { name: "Focus Worktree" }));

    expect(onBranchActivate).toHaveBeenCalledWith(localDetail.primary_branch);
  });

  it("shows create worktree when the selected branch has no materialized worktree", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_branch_inventory") return Promise.resolve([remoteEntry]);
      if (command === "get_branch_inventory_detail") return Promise.resolve(remoteDetail);
      return Promise.resolve([]);
    });

    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({
          selectedBranchName: remoteEntry.canonical_name,
        }),
      },
    });

    await waitFor(() =>
      expect(rendered.getByRole("button", { name: "Create Worktree" })).toBeTruthy(),
    );
  });

  it("disables activation when multiple worktrees map to one ref", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_branch_inventory") {
        return Promise.resolve([
          {
            ...localEntry,
            worktree_path: null,
            worktree_count: 2,
            resolution_action: "resolveAmbiguity" as const,
          },
        ]);
      }
      return Promise.resolve([]);
    });

    const onBranchActivate = vi.fn();
    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({
          selectedBranchName: localEntry.canonical_name,
          onBranchActivate,
        }),
      },
    });

    const button = await waitFor(() =>
      rendered.getByRole("button", { name: "Resolve Ambiguity" }),
    );
    expect((button as HTMLButtonElement).disabled).toBe(true);
    expect(onBranchActivate).not.toHaveBeenCalled();
  });

  it("hydrates and reports filter/query state for window-local persistence", async () => {
    const onStateChange = vi.fn();
    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({
          initialFilter: "Remote",
          initialQuery: "remote",
          selectedBranchName: "origin/feature/remote",
          onStateChange,
        }),
      },
    });

    await waitFor(() =>
      expect(rendered.getByDisplayValue("remote")).toBeTruthy(),
    );
    expect(rendered.getByRole("button", { name: "Remote" }).className).toContain("active");
    await fireEvent.click(rendered.getByRole("button", { name: "All" }));
    await fireEvent.input(rendered.getByPlaceholderText("Filter branches..."), {
      target: { value: "feature" },
    });

    expect(onStateChange).toHaveBeenLastCalledWith({
      filter: "All",
      query: "feature",
      selectedBranchName: "origin/feature/remote",
    });
  });

  it("re-fetches snapshot with refreshKey and rehydrates detail", async () => {
    const refreshedDetail: BranchInventoryDetail = {
      ...localDetail,
      primary_branch: {
        ...localDetail.primary_branch,
        display_name: "Local feature refreshed",
      },
      local_branch: {
        ...localDetail.local_branch!,
        display_name: "Local feature refreshed",
      },
      worktree_path: "/tmp/project/.gwt/worktrees/feature-local-refreshed",
    };
    invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "list_branch_inventory") return Promise.resolve([localEntry, remoteEntry]);
      if (command === "get_branch_inventory_detail") {
        return Promise.resolve(args?.forceRefresh ? refreshedDetail : localDetail);
      }
      return Promise.resolve([]);
    });

    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({
          refreshKey: 0,
          selectedBranchName: localEntry.canonical_name,
        }),
      },
    });

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("list_branch_inventory", {
        projectPath: "/tmp/project",
        refreshKey: 0,
      }),
    );

    await rendered.rerender({
      config: createConfig({
        refreshKey: 1,
        selectedBranchName: localEntry.canonical_name,
      }),
    });

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("get_branch_inventory_detail", {
        projectPath: "/tmp/project",
        canonicalName: localEntry.canonical_name,
        forceRefresh: true,
      }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("list_branch_inventory", {
        projectPath: "/tmp/project",
        refreshKey: 1,
      }),
    );
    await waitFor(() =>
      expect(rendered.container.querySelector(".detail-title")?.textContent).toBe(
        "Local feature refreshed",
      ),
    );
    expect(rendered.getByTestId("branch-browser-detail").textContent).toContain(
      "/tmp/project/.gwt/worktrees/feature-local-refreshed",
    );
  });
});
