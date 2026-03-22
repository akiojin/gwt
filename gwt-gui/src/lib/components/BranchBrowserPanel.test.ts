import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import BranchBrowserPanel from "./BranchBrowserPanel.svelte";
import type {
  BranchBrowserPanelConfig,
  BranchInfo,
  BranchInventoryEntry,
  WorktreeInfo,
} from "../types";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const localBranch: BranchInfo = {
  name: "feature/local",
  display_name: "Local feature",
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

const localEntry: BranchInventoryEntry = {
  id: "feature/local",
  canonical_name: "feature/local",
  primary_branch: localBranch,
  local_branch: localBranch,
  remote_branch: null,
  has_local: true,
  has_remote: false,
  worktree,
  worktree_count: 1,
  resolution_action: "focusExisting",
};

const remoteEntry: BranchInventoryEntry = {
  id: "feature/remote",
  canonical_name: "feature/remote",
  primary_branch: remoteBranch,
  local_branch: null,
  remote_branch: remoteBranch,
  has_local: false,
  has_remote: true,
  worktree: null,
  worktree_count: 0,
  resolution_action: "createWorktree",
};

function createConfig(overrides: Partial<BranchBrowserPanelConfig> = {}): BranchBrowserPanelConfig {
  return {
    projectPath: "/tmp/project",
    refreshKey: 0,
    widthPx: 260,
    minWidthPx: 220,
    maxWidthPx: 520,
    mode: "branch",
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
      return Promise.resolve([]);
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("loads Local branches by default and renders branch details", async () => {
    const onBranchSelect = vi.fn();
    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({
          selectedBranch: localBranch,
          onBranchSelect,
        }),
      },
    });

    await waitFor(() => expect(rendered.getByText("Local feature")).toBeTruthy());
    expect(invokeMock).toHaveBeenCalledWith("list_branch_inventory", {
      projectPath: "/tmp/project",
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

    await waitFor(() => expect(rendered.getByText("Local feature")).toBeTruthy());
    await fireEvent.click(rendered.getByText("Remote"));

    await waitFor(() =>
      expect(rendered.getByText("origin/feature/remote")).toBeTruthy(),
    );
  });

  it("merges local and remote refs into one canonical entry in All mode", async () => {
    const mergedEntry: BranchInventoryEntry = {
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
          selectedBranch: localBranch,
        }),
      },
    });

    await waitFor(() => expect(rendered.getByText("Local feature")).toBeTruthy());
    await fireEvent.click(rendered.getByText("All"));

    await waitFor(() =>
      expect(rendered.getByText("Local + Remote")).toBeTruthy(),
    );
    expect(rendered.container.querySelectorAll(".branch-row")).toHaveLength(1);
  });

  it("forwards branch selection to the host shell", async () => {
    const onBranchSelect = vi.fn();
    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({ onBranchSelect }),
      },
    });

    await waitFor(() => expect(rendered.getByText("Local feature")).toBeTruthy());
    await fireEvent.click(rendered.getByText("Local feature"));

    expect(onBranchSelect).toHaveBeenCalledWith(localBranch);
  });

  it("forwards open/focus worktree action for the selected branch", async () => {
    const onBranchActivate = vi.fn();
    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({
          selectedBranch: localBranch,
          onBranchActivate,
        }),
      },
    });

    await waitFor(() =>
      expect(rendered.getByRole("button", { name: "Focus Worktree" })).toBeTruthy(),
    );
    await fireEvent.click(rendered.getByRole("button", { name: "Focus Worktree" }));

    expect(onBranchActivate).toHaveBeenCalledWith(localBranch);
  });

  it("shows create worktree when the selected branch has no materialized worktree", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_branch_inventory") return Promise.resolve([remoteEntry]);
      return Promise.resolve([]);
    });

    const rendered = render(BranchBrowserPanel, {
      props: {
        config: createConfig({
          selectedBranch: remoteBranch,
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
            worktree: null,
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
          selectedBranch: localBranch,
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
});
