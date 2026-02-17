import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";
import type { BranchInfo } from "../types";

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

async function renderPanel(props: any) {
  const { default: Panel } = await import("./WorktreeSummaryPanel.svelte");
  return render(Panel, { props });
}

const branchFixture: BranchInfo = {
  name: "feature/markdown-ui",
  commit: "1234567",
  is_current: false,
  ahead: 0,
  behind: 0,
  divergence_status: "UpToDate",
  last_tool_usage: null,
  is_agent_running: false,
  agent_status: "unknown",
};

const issueBranchFixture = {
  ...branchFixture,
  name: "feature/issue-1097",
};

const sessionSummaryFixture = {
  status: "ok",
  generating: false,
  toolId: "codex",
  sessionId: "session-1",
  sourceType: "scrollback",
  inputMtimeMs: 1_700_000_000_000,
  summaryUpdatedMs: 1_700_000_000_500,
  markdown: "## Summary\nSummary body\n\n## Highlights\n- A\n- B",
  warning: null,
  error: null,
};

const linkedIssueFixture = {
  number: 1097,
  title: "Worktree Summary rework",
  updatedAt: "2026-02-17T00:00:00Z",
  labels: ["enhancement"],
  url: "https://github.com/test/repo/issues/1097",
};

const dockerContextFixture = {
  worktree_path: "/tmp/project/.gwt/worktrees/feature-markdown-ui",
  file_type: "compose",
  compose_services: ["workspace"],
  docker_available: true,
  compose_available: true,
  daemon_running: true,
  force_host: false,
};

const latestPrFixture = {
  number: 42,
  title: "CI Test PR",
  state: "OPEN",
  url: "https://github.com/test/repo/pull/42",
};

const prDetailFixture = {
  number: 42,
  title: "CI Test PR",
  state: "OPEN",
  url: "https://github.com/test/repo/pull/42",
  mergeable: "MERGEABLE",
  author: "alice",
  baseBranch: "main",
  headBranch: branchFixture.name,
  labels: [],
  assignees: [],
  milestone: null,
  linkedIssues: [],
  checkSuites: [
    {
      workflowName: "CI Build",
      runId: 100,
      status: "completed",
      conclusion: "success",
    },
    {
      workflowName: "Lint",
      runId: 101,
      status: "in_progress",
      conclusion: null,
    },
  ],
  reviews: [],
  reviewComments: [],
  changedFilesCount: 1,
  additions: 10,
  deletions: 5,
};

const quickStartDockerEntry = {
  branch: branchFixture.name,
  tool_id: "claude",
  tool_label: "Claude",
  session_id: "session-456",
  mode: "normal",
  model: "sonnet",
  reasoning_level: "high",
  skip_permissions: false,
  tool_version: "latest",
  docker_service: "workspace",
  docker_recreate: false,
  docker_build: true,
  docker_keep: false,
  docker_container_name: "workspace-container",
  docker_compose_args: ["--build", "-f", "docker-compose.dev.yml"],
  timestamp: 1_700_000_002,
};

const olderQuickStartEntry = {
  ...quickStartDockerEntry,
  session_id: "session-123",
  tool_id: "codex",
  tool_label: "Codex",
  model: "gpt-5-codex",
  timestamp: 1_700_000_001,
};

function sessionSummaryCalls() {
  return invokeMock.mock.calls.filter((c) => c[0] === "get_branch_session_summary");
}

describe("WorktreeSummaryPanel", () => {
  beforeEach(() => {
    cleanup();
    listenMock.mockReset();
    listenMock.mockResolvedValue(() => {});

    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    Object.defineProperty(globalThis, "__TAURI_INTERNALS__", {
      value: { invoke: invokeMock },
      configurable: true,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    delete (globalThis as any).__TAURI_INTERNALS__;
  });

  it("renders branch header and fixed 6-tab UI when branch is selected", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(
          ([cmd, payload]) =>
            cmd === "get_branch_session_summary" &&
            payload?.projectPath === "/tmp/project" &&
            payload?.branch === "feature/markdown-ui"
        )
      ).toBe(true);
      expect(rendered.container.querySelector("h2")?.textContent).toBe("feature/markdown-ui");
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    expect(tabs).toHaveLength(6);
    expect(tabs[0]?.textContent?.trim()).toBe("Summary");
    expect(tabs[0]?.classList.contains("active")).toBe(true);
    expect(tabs[1]?.textContent?.trim()).toBe("Git");
    expect(tabs[2]?.textContent?.trim()).toBe("Issue");
    expect(tabs[3]?.textContent?.trim()).toBe("PR");
    expect(tabs[4]?.textContent?.trim()).toBe("Workflow");
    expect(tabs[5]?.textContent?.trim()).toBe("Docker");
    expect(rendered.queryByRole("button", { name: "Quick Start" })).toBeNull();
  });

  it("shows placeholder when no branch is selected", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: null,
    });

    await waitFor(() => {
      expect(rendered.container.querySelector(".placeholder h2")?.textContent).toBe(
        "Worktree Summary"
      );
    });
  });

  it("runs Continue/New from header buttons using latest quick-start entry", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [olderQuickStartEntry, quickStartDockerEntry];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const onQuickLaunch = vi.fn().mockResolvedValue(undefined);
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onQuickLaunch,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_branch_quick_start", {
        projectPath: "/tmp/project",
        branch: "feature/markdown-ui",
      });
      expect(rendered.getByRole("button", { name: "Continue" })).toBeTruthy();
      expect(rendered.getByRole("button", { name: "New" })).toBeTruthy();
    });

    const continueButton = rendered.getByRole("button", { name: "Continue" });
    const newButton = rendered.getByRole("button", { name: "New" });
    expect((continueButton as HTMLButtonElement).disabled).toBe(false);
    expect((newButton as HTMLButtonElement).disabled).toBe(false);

    await fireEvent.click(continueButton);
    await fireEvent.click(newButton);

    await waitFor(() => {
      expect(onQuickLaunch).toHaveBeenCalledTimes(2);
    });
    expect(onQuickLaunch.mock.calls[0]?.[0]?.mode).toBe("continue");
    expect(onQuickLaunch.mock.calls[0]?.[0]?.resumeSessionId).toBe("session-456");
    expect(onQuickLaunch.mock.calls[1]?.[0]?.mode).toBe("normal");
    expect(onQuickLaunch.mock.calls[1]?.[0]?.resumeSessionId).toBeUndefined();
  });

  it("disables header Continue/New buttons when quick-start history is empty", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onQuickLaunch: vi.fn().mockResolvedValue(undefined),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_branch_quick_start", {
        projectPath: "/tmp/project",
        branch: "feature/markdown-ui",
      });
    });

    const continueButton = rendered.getByRole("button", { name: "Continue" });
    const newButton = rendered.getByRole("button", { name: "New" });
    expect((continueButton as HTMLButtonElement).disabled).toBe(true);
    expect((newButton as HTMLButtonElement).disabled).toBe(true);
  });

  it("renders session summary metadata and markdown in Summary tab", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const summaryTab = tabs[0] as HTMLElement;
    await fireEvent.click(summaryTab);

    await waitFor(() => {
      expect(summaryTab.classList.contains("active")).toBe(true);
      expect(rendered.container.querySelector(".session-summary-meta")).toBeTruthy();
      expect(rendered.container.querySelector(".session-summary-markdown h2")).toBeTruthy();
      expect(rendered.container.querySelectorAll(".session-summary-markdown li")).toHaveLength(2);
    });
  });

  it("switches to Issue tab and shows linked branch issue", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return linkedIssueFixture;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: issueBranchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const issueTab = tabs[2] as HTMLElement;
    await fireEvent.click(issueTab);

    await waitFor(() => {
      expect(issueTab.classList.contains("active")).toBe(true);
      expect(rendered.getByText("#1097 Worktree Summary rework")).toBeTruthy();
      expect(rendered.getByText("label: enhancement")).toBeTruthy();
    });
  });

  it("shows empty state in Issue tab when linked issue does not exist", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const issueTab = tabs[2] as HTMLElement;
    await fireEvent.click(issueTab);

    await waitFor(() => {
      expect(rendered.getByText("No issue linked to this branch.")).toBeTruthy();
    });
  });

  it("switches to PR tab and shows PrStatusSection", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return latestPrFixture;
      if (cmd === "fetch_pr_detail") return prDetailFixture;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[3] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(prTab.classList.contains("active")).toBe(true);
      expect(rendered.container.querySelector(".pr-status-section")).toBeTruthy();
      expect(rendered.getByText("#42 CI Test PR")).toBeTruthy();
    });
  });

  it("shows GitHub CLI auth warning in PR tab when CLI is unavailable", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      ghCliStatus: { available: false, authenticated: false },
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[3] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(rendered.getByText("GitHub CLI (gh) is not available.")).toBeTruthy();
    });
  });

  it("switches to Git tab and keeps Git section expanded", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      if (cmd === "get_git_change_summary") {
        return {
          file_count: 2,
          commit_count: 1,
          stash_count: 0,
          base_branch: "main",
          files: [],
          ahead_by: 0,
          behind_by: 0,
        };
      }
      if (cmd === "get_base_branch_candidates") return ["main"];
      if (cmd === "list_git_commits") return [];
      if (cmd === "list_git_stash") return [];
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const gitTab = tabs[1] as HTMLElement;
    await fireEvent.click(gitTab);

    await waitFor(() => {
      expect(gitTab.classList.contains("active")).toBe(true);
      expect(rendered.container.querySelector(".git-section .git-body")).toBeTruthy();
    });

    expect(rendered.container.querySelector(".git-section .collapse-icon")).toBeNull();
  });

  it("switches to Workflow tab and shows workflow runs", async () => {
    const windowOpen = vi.spyOn(window, "open").mockReturnValue(null);

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return latestPrFixture;
      if (cmd === "fetch_pr_detail") return prDetailFixture;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const workflowTab = tabs[4] as HTMLElement;
    await fireEvent.click(workflowTab);

    await waitFor(() => {
      expect(workflowTab.classList.contains("active")).toBe(true);
      expect(rendered.getByText("CI Build")).toBeTruthy();
      expect(rendered.getByText("Lint")).toBeTruthy();
    });

    await fireEvent.click(
      rendered.getByText("Success").closest("button") as HTMLButtonElement
    );
    expect(windowOpen).toHaveBeenCalledWith(
      "https://github.com/test/repo/actions/runs/100",
      "_blank",
      "noopener"
    );
    windowOpen.mockRestore();
  });

  it("renders Workflow from resolved prNumber even when latest branch PR is unavailable", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "fetch_pr_detail") return prDetailFixture;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      prNumber: 42,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const workflowTab = tabs[4] as HTMLElement;
    await fireEvent.click(workflowTab);

    await waitFor(() => {
      expect(workflowTab.classList.contains("active")).toBe(true);
      expect(rendered.getByText("CI Build")).toBeTruthy();
      expect(rendered.queryByText("No PR.")).toBeNull();
    });
  });

  it("ignores stale latest branch PR errors after branch switch", async () => {
    let rejectFirstPrLookup: ((reason?: Error) => void) | undefined;

    invokeMock.mockImplementation(
      (cmd: string, args?: { branch?: string; projectPath?: string }) => {
        if (cmd === "get_branch_quick_start") return [];
        if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
        if (cmd === "fetch_branch_linked_issue") return null;
        if (cmd === "fetch_latest_branch_pr") {
          if (args?.branch === "feature/markdown-ui") {
            return new Promise<null>((_, reject) => {
              rejectFirstPrLookup = (reason?: Error) => {
                reject(reason);
              };
            });
          }
          return null;
        }
        if (cmd === "detect_docker_context") return dockerContextFixture;
        return [];
      }
    );

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("fetch_latest_branch_pr", {
        projectPath: "/tmp/project",
        branch: "feature/markdown-ui",
      });
    });

    await rendered.rerender({
      projectPath: "/tmp/project",
      selectedBranch: { ...branchFixture, name: "feature/next-task" },
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("fetch_latest_branch_pr", {
        projectPath: "/tmp/project",
        branch: "feature/next-task",
      });
    });

    if (rejectFirstPrLookup) {
      rejectFirstPrLookup(new Error("stale request"));
    }

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[3] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(prTab.classList.contains("active")).toBe(true);
      expect(rendered.getByText("No PR")).toBeTruthy();
      expect(rendered.queryByText(/Failed to load PR:/)).toBeNull();
    });
  });

  it("switches to Docker tab and shows current context plus quick-start docker history", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [quickStartDockerEntry];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const dockerTab = tabs[5] as HTMLElement;
    await fireEvent.click(dockerTab);

    await waitFor(() => {
      expect(dockerTab.classList.contains("active")).toBe(true);
      expect(rendered.getByText("type: compose")).toBeTruthy();
      expect(rendered.getByText("services: workspace")).toBeTruthy();
      expect(rendered.getByText("runtime: Docker")).toBeTruthy();
      expect(rendered.getByText("container: workspace-container")).toBeTruthy();
      expect(rendered.getByText("compose args: --build -f docker-compose.dev.yml")).toBeTruthy();
      expect(rendered.getByText("Session session-456")).toBeTruthy();
    });
  });

  it("requests cached-only session summary when no agent tab exists", async () => {
    await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      agentTabBranches: [],
      activeAgentTabBranch: null,
    });

    await waitFor(() => {
      expect(sessionSummaryCalls()[0]?.[1]).toEqual({
        projectPath: "/tmp/project",
        branch: "feature/markdown-ui",
        cachedOnly: true,
        preferredLanguage: "auto",
      });
    });
  });

  it("does not poll when no agent tab exists", async () => {
    vi.useFakeTimers();
    await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      agentTabBranches: [],
      activeAgentTabBranch: null,
    });

    await waitFor(() => {
      expect(sessionSummaryCalls()).toHaveLength(1);
    });

    await vi.advanceTimersByTimeAsync(120_000);
    expect(sessionSummaryCalls()).toHaveLength(1);
  });

  it("polls every 15s when agent tab is focused", async () => {
    vi.useFakeTimers();
    await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      agentTabBranches: ["feature/markdown-ui"],
      activeAgentTabBranch: "feature/markdown-ui",
    });

    await waitFor(() => {
      expect(sessionSummaryCalls()).toHaveLength(1);
    });
    expect(sessionSummaryCalls()[0]?.[1]?.cachedOnly).toBe(false);

    await vi.advanceTimersByTimeAsync(14_999);
    expect(sessionSummaryCalls()).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(1);
    await waitFor(() => {
      expect(sessionSummaryCalls()).toHaveLength(2);
    });
  });

  it("polls every 60s when agent tab exists but is not focused", async () => {
    vi.useFakeTimers();
    await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      agentTabBranches: ["feature/markdown-ui"],
      activeAgentTabBranch: "other-branch",
    });

    await waitFor(() => {
      expect(sessionSummaryCalls()).toHaveLength(1);
    });
    expect(sessionSummaryCalls()[0]?.[1]?.cachedOnly).toBe(false);

    await vi.advanceTimersByTimeAsync(15_000);
    expect(sessionSummaryCalls()).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(45_000);
    await waitFor(() => {
      expect(sessionSummaryCalls()).toHaveLength(2);
    });
  });

  it("shows rebuild progress spinner and refreshes summary on completion", async () => {
    const listeners: Record<string, (event: { payload: any }) => void> = {};
    listenMock.mockImplementation(async (eventName: string, handler: any) => {
      listeners[eventName] = handler;
      return () => {};
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      agentTabBranches: [],
      activeAgentTabBranch: null,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const summaryTab = tabs[0] as HTMLElement;
    await fireEvent.click(summaryTab);

    await waitFor(() => {
      expect(typeof listeners["session-summary-rebuild-progress"]).toBe("function");
    });

    listeners["session-summary-rebuild-progress"]({
      payload: {
        projectPath: "/tmp/project",
        language: "ja",
        total: 3,
        completed: 0,
        branch: null,
        status: "started",
        error: null,
      },
    });

    await waitFor(() => {
      expect(rendered.getByText("Rebuilding summaries (0/3)")).toBeTruthy();
    });

    const callsBeforeComplete = sessionSummaryCalls().length;
    listeners["session-summary-rebuild-progress"]({
      payload: {
        projectPath: "/tmp/project",
        language: "ja",
        total: 3,
        completed: 3,
        branch: null,
        status: "completed",
        error: null,
      },
    });

    await waitFor(() => {
      expect(sessionSummaryCalls().length).toBeGreaterThan(callsBeforeComplete);
    });
  });

  it("keeps rebuild warning after completion until next rebuild starts", async () => {
    const listeners: Record<string, (event: { payload: any }) => void> = {};
    listenMock.mockImplementation(async (eventName: string, handler: any) => {
      listeners[eventName] = handler;
      return () => {};
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      agentTabBranches: [],
      activeAgentTabBranch: null,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const summaryTab = tabs[0] as HTMLElement;
    await fireEvent.click(summaryTab);

    await waitFor(() => {
      expect(typeof listeners["session-summary-rebuild-progress"]).toBe("function");
    });

    listeners["session-summary-rebuild-progress"]({
      payload: {
        projectPath: "/tmp/project",
        language: "ja",
        total: 3,
        completed: 0,
        branch: null,
        status: "started",
        error: null,
      },
    });

    listeners["session-summary-rebuild-progress"]({
      payload: {
        projectPath: "/tmp/project",
        language: "ja",
        total: 3,
        completed: 1,
        branch: "feature/markdown-ui",
        status: "branch-error",
        error: "branch failed",
      },
    });

    listeners["session-summary-rebuild-progress"]({
      payload: {
        projectPath: "/tmp/project",
        language: "ja",
        total: 3,
        completed: 3,
        branch: null,
        status: "completed",
        error: null,
      },
    });

    await waitFor(() => {
      expect(rendered.getByText("Rebuild warning: branch failed")).toBeTruthy();
    });
  });
});
