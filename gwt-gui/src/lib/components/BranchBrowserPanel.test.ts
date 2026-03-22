import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import BranchBrowserPanel from "./BranchBrowserPanel.svelte";
import type { BranchBrowserPanelConfig, BranchInfo, WorktreeInfo } from "../types";

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
      if (command === "list_worktree_branches") return Promise.resolve([localBranch]);
      if (command === "list_remote_branches") return Promise.resolve([remoteBranch]);
      if (command === "list_worktrees") return Promise.resolve([worktree]);
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
    expect(invokeMock).toHaveBeenCalledWith("list_worktree_branches", {
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
    expect(invokeMock).toHaveBeenCalledWith("list_remote_branches", {
      projectPath: "/tmp/project",
    });
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
});
