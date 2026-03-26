import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";
import type { BranchInfo } from "../types";

const invokeMock = vi.fn();
const listenMock = vi.fn();
const openExternalUrlMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
  listen: listenMock,
}));

vi.mock("../openExternalUrl", () => ({
  openExternalUrl: (...args: unknown[]) => openExternalUrlMock(...args),
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

const prPreflightFixture = {
  baseBranch: "develop",
  aheadBy: 0,
  behindBy: 2,
  status: "behind",
  blockingReason: "Branch update required before creating a PR.",
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

function commandCalls(command: string) {
  return invokeMock.mock.calls.filter((c) => c[0] === command);
}

describe("WorktreeSummaryPanel", () => {
  beforeEach(() => {
    cleanup();
    listenMock.mockReset();
    listenMock.mockResolvedValue(() => {});

    invokeMock.mockReset();
    openExternalUrlMock.mockReset();
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

  it("renders branch header and fixed 5-tab UI when branch is selected (no Workflow tab)", async () => {
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
      expect(rendered.container.querySelector("h2 .branch-display-name")?.textContent).toBe("feature/markdown-ui");
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    expect(tabs).toHaveLength(5);
    expect(tabs[0]?.textContent?.trim()).toBe("Summary");
    expect(tabs[0]?.classList.contains("active")).toBe(true);
    expect(tabs[1]?.textContent?.trim()).toBe("Git");
    expect(tabs[2]?.textContent?.trim()).toBe("Issue");
    expect(tabs[3]?.textContent?.trim()).toBe("PR");
    expect(tabs[4]?.textContent?.trim()).toBe("Docker");

    // Verify Workflow tab does not exist
    const tabTexts = Array.from(tabs).map((t) => t.textContent?.trim());
    expect(tabTexts).not.toContain("Workflow");
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

  it("does not prefetch Issue/PR/Docker data on branch selection", async () => {
    await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(commandCalls("get_branch_session_summary").length).toBeGreaterThan(0);
    });

    expect(commandCalls("fetch_branch_linked_issue")).toHaveLength(0);
    expect(commandCalls("fetch_latest_branch_pr")).toHaveLength(0);
    expect(commandCalls("detect_docker_context")).toHaveLength(0);
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
    expect(commandCalls("fetch_branch_linked_issue")).toHaveLength(0);
    await fireEvent.click(issueTab);

    await waitFor(() => {
      expect(issueTab.classList.contains("active")).toBe(true);
      expect(rendered.getByText("#1097 Worktree Summary rework")).toBeTruthy();
      expect(rendered.getByText("label: enhancement")).toBeTruthy();
    });
    expect(commandCalls("fetch_branch_linked_issue")).toHaveLength(1);
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
    expect(commandCalls("fetch_latest_branch_pr")).toHaveLength(0);
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(prTab.classList.contains("active")).toBe(true);
      expect(rendered.container.querySelector(".pr-status-section")).toBeTruthy();
      expect(rendered.getByText("#42 CI Test PR")).toBeTruthy();
    });
    expect(commandCalls("fetch_latest_branch_pr")).toHaveLength(1);
  });

  it("shows merged PR detail when latestBranchPr is MERGED and no sidebar PR number exists", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr")
        return {
          number: 99,
          title: "Old merged PR",
          state: "MERGED",
          url: "https://github.com/test/repo/pull/99",
        };
      if (cmd === "fetch_pr_detail") return { ...prDetailFixture, number: 99, title: "Old merged PR", state: "MERGED" };
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
      expect(rendered.getByText("#99 Old merged PR")).toBeTruthy();
    });
    expect(commandCalls("fetch_pr_detail").some(([, p]) => p?.prNumber === 99)).toBe(true);
  });

  it("prefers sidebar prNumber over latestBranchPr when both are present", async () => {
    invokeMock.mockImplementation(
      async (cmd: string, args?: { prNumber?: number }) => {
        if (cmd === "get_branch_quick_start") return [];
        if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
        if (cmd === "fetch_branch_linked_issue") return null;
        if (cmd === "fetch_latest_branch_pr")
          return {
            number: 99,
            title: "Old merged PR",
            state: "MERGED",
            url: "https://github.com/test/repo/pull/99",
          };
        if (cmd === "fetch_pr_detail") {
          if (args?.prNumber === 42) return prDetailFixture;
          return { ...prDetailFixture, number: 99, title: "Old merged PR", state: "MERGED" };
        }
        if (cmd === "detect_docker_context") return dockerContextFixture;
        return [];
      }
    );

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      prNumber: 42,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[3] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(rendered.getByText("#42 CI Test PR")).toBeTruthy();
    });
    expect(
      commandCalls("fetch_pr_detail").some(([, payload]) => payload?.prNumber === 99)
    ).toBe(false);
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

  it("shows CI checks inside PR tab via PrStatusSection Checks section", async () => {
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
    const prTab = tabs[3] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(prTab.classList.contains("active")).toBe(true);
      expect(rendered.container.querySelector(".pr-status-section")).toBeTruthy();
      // Checks section should exist within PrStatusSection
      expect(rendered.container.querySelector(".checks-section")).toBeTruthy();
      expect(rendered.container.textContent).toContain("Checks (2)");
    });
  });

  it("polls PR detail after Update Branch until merge state changes", async () => {
    let prDetailCallCount = 0;

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return latestPrFixture;
      if (cmd === "fetch_pr_detail") {
        prDetailCallCount += 1;
        if (prDetailCallCount <= 2) {
          return { ...prDetailFixture, mergeStateStatus: "BEHIND" };
        }
        return { ...prDetailFixture, mergeStateStatus: "CLEAN" };
      }
      if (cmd === "update_pr_branch") return "accepted";
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
      expect(rendered.getByRole("button", { name: "Update Branch" })).toBeTruthy();
    });

    vi.useFakeTimers();
    await fireEvent.click(rendered.getByRole("button", { name: "Update Branch" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("update_pr_branch", {
        projectPath: "/tmp/project",
        prNumber: 42,
      });
    });

    await vi.advanceTimersByTimeAsync(20_000);

    await waitFor(() => {
      expect(commandCalls("fetch_pr_detail").length).toBeGreaterThanOrEqual(3);
      expect(rendered.queryByRole("button", { name: "Update Branch" })).toBeNull();
    });
  });

  it("keeps polling PR detail after merge even if user switches away from PR tab", async () => {
    let prDetailCallCount = 0;

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return latestPrFixture;
      if (cmd === "fetch_pr_detail") {
        prDetailCallCount += 1;
        if (prDetailCallCount <= 2) {
          return { ...prDetailFixture, state: "OPEN" };
        }
        return { ...prDetailFixture, state: "MERGED", mergeable: "UNKNOWN" };
      }
      if (cmd === "merge_pull_request") {
        return await new Promise<string>((resolve) => {
          setTimeout(() => resolve("accepted"), 1);
        });
      }
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const summaryTab = tabs[0] as HTMLElement;
    const prTab = tabs[3] as HTMLElement;

    await fireEvent.click(prTab);
    await waitFor(() => {
      expect(rendered.getByRole("button", { name: "Mergeable" })).toBeTruthy();
      expect(commandCalls("fetch_pr_detail")).toHaveLength(1);
    });

    vi.useFakeTimers();

    await fireEvent.click(rendered.getByRole("button", { name: "Mergeable" }));
    await waitFor(() => {
      expect(rendered.getByRole("button", { name: "Merge" })).toBeTruthy();
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Merge" }));
    await fireEvent.click(summaryTab);
    expect(summaryTab.classList.contains("active")).toBe(true);
    await vi.advanceTimersByTimeAsync(10_000);

    await waitFor(() => {
      expect(commandCalls("fetch_pr_detail").length).toBeGreaterThan(1);
    });
  });

  it("closes merge confirm modal when PR context changes", async () => {
    invokeMock.mockImplementation(
      async (cmd: string, args?: { branch?: string }) => {
        if (cmd === "get_branch_quick_start") return [];
        if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
        if (cmd === "fetch_branch_linked_issue") return null;
        if (cmd === "fetch_latest_branch_pr") {
          if (args?.branch === "feature/markdown-ui") return latestPrFixture;
          return null;
        }
        if (cmd === "fetch_pr_detail") return prDetailFixture;
        if (cmd === "detect_docker_context") return dockerContextFixture;
        return [];
      }
    );

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[3] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(rendered.getByRole("button", { name: "Mergeable" })).toBeTruthy();
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Mergeable" }));

    await waitFor(() => {
      expect(rendered.getByRole("button", { name: "Merge" })).toBeTruthy();
    });

    await rendered.rerender({
      projectPath: "/tmp/project",
      selectedBranch: { ...branchFixture, name: "feature/next-task" },
    });

    await waitFor(() => {
      expect(rendered.queryByRole("button", { name: "Merge" })).toBeNull();
    });
  });

  it("shows update-branch failure in PR tab", async () => {
    const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      invokeMock.mockImplementation(async (cmd: string) => {
        if (cmd === "get_branch_quick_start") return [];
        if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
        if (cmd === "fetch_branch_linked_issue") return null;
        if (cmd === "fetch_latest_branch_pr") return latestPrFixture;
        if (cmd === "fetch_pr_detail") return { ...prDetailFixture, mergeStateStatus: "BEHIND" };
        if (cmd === "update_pr_branch") throw new Error("403 Forbidden");
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
        expect(rendered.getByRole("button", { name: "Update Branch" })).toBeTruthy();
      });

      await fireEvent.click(rendered.getByRole("button", { name: "Update Branch" }));

      await waitFor(() => {
        expect(rendered.getByText("Failed to update branch: 403 Forbidden")).toBeTruthy();
        expect(rendered.getByText("#42 CI Test PR")).toBeTruthy();
        expect(rendered.getByRole("button", { name: "Update Branch" })).toBeTruthy();
        expect(rendered.container.querySelector(".pr-status-error")).toBeNull();
      });
    } finally {
      consoleErrorSpy.mockRestore();
    }
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

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[3] as HTMLElement;
    await fireEvent.click(prTab);

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

    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("fetch_latest_branch_pr", {
        projectPath: "/tmp/project",
        branch: "feature/next-task",
      });
    });

    if (rejectFirstPrLookup) {
      rejectFirstPrLookup(new Error("stale request"));
    }

    await waitFor(() => {
      expect(prTab.classList.contains("active")).toBe(true);
      expect(rendered.getByText("No PR")).toBeTruthy();
      expect(rendered.queryByText(/Failed to load PR:/)).toBeNull();
    });
  });

  it("reuses cached latest PR within ttl when switching back to same branch", async () => {
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
    const prTab = tabs[3] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(commandCalls("fetch_latest_branch_pr")).toHaveLength(1);
      expect(rendered.getByText("#42 CI Test PR")).toBeTruthy();
    });

    await rendered.rerender({
      projectPath: "/tmp/project",
      selectedBranch: { ...branchFixture, name: "feature/next-task" },
    });
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(commandCalls("fetch_latest_branch_pr")).toHaveLength(2);
    });

    await rendered.rerender({
      projectPath: "/tmp/project",
      selectedBranch: { ...branchFixture },
    });
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(rendered.getByText("#42 CI Test PR")).toBeTruthy();
    });
    expect(commandCalls("fetch_latest_branch_pr")).toHaveLength(2);
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
    const dockerTab = tabs[4] as HTMLElement;
    expect(commandCalls("detect_docker_context")).toHaveLength(0);
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
    expect(commandCalls("detect_docker_context")).toHaveLength(1);
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

  it("shows Docker tab with no context when detection returns null", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return null;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const dockerTab = tabs[4] as HTMLElement;
    await fireEvent.click(dockerTab);

    await waitFor(() => {
      expect(dockerTab.classList.contains("active")).toBe(true);
      expect(rendered.container.textContent).toContain("No Docker context");
    });
  });

  it("shows session summary with warning when response has warning", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return { ...sessionSummaryFixture, warning: "Cache is stale" };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Cache is stale");
    });
  });

  it("shows no summary placeholder when markdown is null and no error", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return { ...sessionSummaryFixture, markdown: null, error: null };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("No summary.");
    });
  });

  it("shows empty state in PR tab when no PR exists", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[3] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(rendered.getByText("No PR")).toBeTruthy();
    });
  });

  it("loads branch PR preflight and shows blocking banner when no PR exists yet", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "fetch_branch_pr_preflight") return prPreflightFixture;
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
      expect(invokeMock).toHaveBeenCalledWith("fetch_branch_pr_preflight", {
        projectPath: "/tmp/project",
        branch: "feature/markdown-ui",
      });
      expect(rendered.container.textContent).toContain(
        "Branch update required before creating a PR."
      );
    });
  });

  it("shows branch preflight error in PR tab when preflight lookup fails", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "fetch_branch_pr_preflight") throw new Error("sync lookup failed");
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
      expect(rendered.container.textContent).toContain("Failed to check branch sync");
      expect(rendered.container.textContent).toContain("No PR");
    });
  });

  it("switches tabs and goes back preserving state", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");

    // Go to Git tab
    await fireEvent.click(tabs[1] as HTMLElement);
    await waitFor(() => {
      expect((tabs[1] as HTMLElement).classList.contains("active")).toBe(true);
    });

    // Go to Issue tab
    await fireEvent.click(tabs[2] as HTMLElement);
    await waitFor(() => {
      expect((tabs[2] as HTMLElement).classList.contains("active")).toBe(true);
    });

    // Go back to Summary tab
    await fireEvent.click(tabs[0] as HTMLElement);
    await waitFor(() => {
      expect((tabs[0] as HTMLElement).classList.contains("active")).toBe(true);
    });
  });

  it("renders branch header with commit hash", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: { ...branchFixture, commit: "abcdef1" },
    });

    await waitFor(() => {
      expect(rendered.container.querySelector("h2 .branch-display-name")?.textContent).toBe("feature/markdown-ui");
    });
  });

  it("shows latest agent badge in the branch header when last_tool_usage exists", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: { ...branchFixture, last_tool_usage: "codex" },
    });

    await waitFor(() => {
      expect(rendered.getByText("Latest agent: Codex")).toBeTruthy();
    });

    const badge = rendered.getByText("Latest agent: Codex");
    expect(badge.classList.contains("branch-tool-badge")).toBe(true);
    expect(badge.classList.contains("codex")).toBe(true);
  });

  it("renders header action buttons with hover titles", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [olderQuickStartEntry, quickStartDockerEntry];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onQuickLaunch: vi.fn().mockResolvedValue(undefined),
      onNewTerminal: vi.fn(),
      onLaunchAgent: vi.fn(),
    });

    await waitFor(() => {
      expect(rendered.getByTitle("Continue")).toBeTruthy();
      expect(rendered.getByTitle("New")).toBeTruthy();
      expect(rendered.getByTitle("Check/Fix Docs + Edit")).toBeTruthy();
      expect(rendered.getByTitle("New Terminal")).toBeTruthy();
      expect(rendered.getByTitle("Launch Agent")).toBeTruthy();
    });
  });

  it("renders New Terminal button and fires onNewTerminal callback", async () => {
    const onNewTerminal = vi.fn();
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onNewTerminal,
    });

    await waitFor(() => {
      expect(rendered.getByTitle("New Terminal")).toBeTruthy();
    });

    const btn = rendered.getByTitle("New Terminal");
    await fireEvent.click(btn);
    expect(onNewTerminal).toHaveBeenCalledTimes(1);
  });

  it("checks/fixes docs and opens editor callback from header button", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      if (cmd === "check_and_fix_agent_instruction_docs") {
        return {
          worktreePath: "/tmp/project/.gwt/worktrees/feature-markdown-ui",
          checkedFiles: ["CLAUDE.md", "AGENTS.md", "GEMINI.md"],
          updatedFiles: ["AGENTS.md"],
        };
      }
      return [];
    });

    const onOpenDocsEditor = vi.fn().mockResolvedValue(undefined);
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onOpenDocsEditor,
    });

    const button = await rendered.findByRole("button", { name: "Check/Fix Docs + Edit" });
    await fireEvent.click(button);

    await waitFor(() => {
      expect(commandCalls("check_and_fix_agent_instruction_docs")).toHaveLength(1);
      expect(onOpenDocsEditor).toHaveBeenCalledWith(
        "/tmp/project/.gwt/worktrees/feature-markdown-ui"
      );
    });
  });

  it("shows docs check error and skips editor callback on failure", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      if (cmd === "check_and_fix_agent_instruction_docs") {
        throw new Error("worktree not found");
      }
      return [];
    });

    const onOpenDocsEditor = vi.fn().mockResolvedValue(undefined);
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onOpenDocsEditor,
    });

    const button = await rendered.findByRole("button", { name: "Check/Fix Docs + Edit" });
    await fireEvent.click(button);

    await waitFor(() => {
      expect(rendered.getByText(/Failed to check\/fix docs:/)).toBeTruthy();
    });
    expect(onOpenDocsEditor).not.toHaveBeenCalled();
  });

  it("renders New Terminal button even without onNewTerminal callback", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.getByTitle("New Terminal")).toBeTruthy();
    });

    // Clicking without callback should not throw
    const btn = rendered.getByTitle("New Terminal");
    await fireEvent.click(btn);
  });

  it("shows ghCliStatus not-authenticated message in PR tab", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      ghCliStatus: { available: true, authenticated: false },
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[3] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(
        rendered.getByText("GitHub CLI (gh) is not authenticated. Run: gh auth login")
      ).toBeTruthy();
    });
  });

  it("shows session summary with status=error and custom error message", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return {
          ...sessionSummaryFixture,
          status: "error",
          markdown: null,
          error: "API key invalid",
        };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("API key invalid");
      expect(rendered.container.textContent).toContain("Error");
    });
  });

  it("shows ai-not-configured placeholder in Summary tab", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return {
          ...sessionSummaryFixture,
          status: "ai-not-configured",
          markdown: null,
        };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("AI not configured");
      expect(rendered.container.textContent).toContain(
        "Configure AI in Settings to enable session summary."
      );
    });
  });

  it("shows disabled placeholder in Summary tab", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return {
          ...sessionSummaryFixture,
          status: "disabled",
          markdown: null,
        };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Disabled");
      expect(rendered.container.textContent).toContain("Session summary disabled.");
    });
  });

  it("shows no-session placeholder in Summary tab", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return {
          ...sessionSummaryFixture,
          status: "no-session",
          markdown: null,
        };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("No session");
      expect(rendered.container.textContent).toContain("No session.");
    });
  });

  it("shows Generating... placeholder when generating with no markdown", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return {
          ...sessionSummaryFixture,
          status: "ok",
          generating: true,
          markdown: null,
        };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Generating...");
    });
  });

  it("shows session summary with generating=true and existing markdown (Updating...)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return {
          ...sessionSummaryFixture,
          status: "ok",
          generating: true,
          markdown: "## Existing\nSome content",
          toolId: "claude",
          sessionId: "session-1",
        };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Updating...");
    });
  });

  it("shows session summary error when get_branch_session_summary throws", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") throw new Error("Connection refused");
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain(
        "Failed to generate session summary: Connection refused"
      );
    });
  });

  it("shows quick start error when get_branch_quick_start throws", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") throw new Error("Quick start failed");
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

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Failed to load Quick Start");
    });
  });

  it("shows quick launch error when onQuickLaunch throws", async () => {
    // loadQuickStart resets quickLaunchError, so we need the mock to be
    // stable throughout the test to avoid the error being cleared by a
    // subsequent $effect re-run of loadQuickStart.
    let quickStartCallCount = 0;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") {
        quickStartCallCount++;
        return [quickStartDockerEntry];
      }
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const onQuickLaunch = vi.fn().mockRejectedValue(new Error("Launch failed"));
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onQuickLaunch,
    });

    await waitFor(() => {
      const btn = rendered.getByRole("button", { name: "Continue" });
      expect((btn as HTMLButtonElement).disabled).toBe(false);
    });

    // Stop loadQuickStart from resetting quickLaunchError by returning from cache
    // (the cache is already populated from the first call)
    await fireEvent.click(rendered.getByRole("button", { name: "Continue" }));

    await waitFor(() => {
      expect(onQuickLaunch).toHaveBeenCalledTimes(1);
    });

    // The error should be visible now
    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Failed to launch");
    });
  });

  it("shows linked issue error when fetch_branch_linked_issue throws", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") throw new Error("Issue fetch failed");
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const issueTab = tabs[2] as HTMLElement;
    await fireEvent.click(issueTab);

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Failed to load linked issue");
    });
  });

  it("shows docker context error when detect_docker_context throws", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") throw new Error("Docker detection failed");
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const dockerTab = tabs[4] as HTMLElement;
    await fireEvent.click(dockerTab);

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Failed to detect Docker context");
    });
  });

  it("shows latest branch PR error when fetch_latest_branch_pr throws", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") throw new Error("PR fetch failed");
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
      expect(rendered.container.textContent).toContain("Failed to load PR");
    });
  });

  it("normalizes origin/ branch name and shows branch header", async () => {
    const originBranch: BranchInfo = {
      ...branchFixture,
      name: "origin/feature/markdown-ui",
    };

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: originBranch,
    });

    await waitFor(() => {
      expect(rendered.container.querySelector("h2 .branch-display-name")?.textContent).toBe(
        "origin/feature/markdown-ui"
      );
    });

    // The normalized branch name should be used in invoke calls
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(
          ([cmd, payload]) =>
            cmd === "get_branch_session_summary" &&
            payload?.branch === "feature/markdown-ui"
        )
      ).toBe(true);
    });
  });

  it("displays session summary pane: prefix as Live indicator", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return {
          ...sessionSummaryFixture,
          sessionId: "pane:0",
          toolId: "claude",
          sourceType: "scrollback",
        };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Live (pane summary)");
      expect(rendered.container.textContent).toContain("Live (scrollback)");
    });
  });

  it("displays session summary with toolId only (no sessionId)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return {
          ...sessionSummaryFixture,
          sessionId: null,
          toolId: "codex",
        };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      // Should show just "codex" without session ID
      const subtitle = rendered.container.querySelector(".quick-subtitle");
      expect(subtitle?.textContent?.trim()).toContain("codex");
    });
  });

  it("displays session summary subtitle with non-pane session id", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return {
          ...sessionSummaryFixture,
          sessionId: "session-42",
          toolId: "codex",
        };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("codex #session-42");
    });
  });

  it("keeps previous markdown during silent polling when refreshed result has null markdown", async () => {
    vi.useFakeTimers();
    let callCount = 0;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") {
        callCount += 1;
        if (callCount === 1) {
          return {
            ...sessionSummaryFixture,
            status: "ok",
            markdown: "## First summary\\nKeep me",
          };
        }
        return {
          ...sessionSummaryFixture,
          status: "ok",
          markdown: null,
        };
      }
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      agentTabBranches: ["feature/markdown-ui"],
      activeAgentTabBranch: "feature/markdown-ui",
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("First summary");
      expect(sessionSummaryCalls().length).toBeGreaterThanOrEqual(1);
    });

    await vi.advanceTimersByTimeAsync(15_000);

    await waitFor(() => {
      expect(sessionSummaryCalls().length).toBeGreaterThanOrEqual(2);
      expect(rendered.container.textContent).toContain("First summary");
    });
  });

  it("shows meta section with language label and timestamps", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return {
          ...sessionSummaryFixture,
          language: "ja",
          sourceType: "session",
          inputMtimeMs: 1_700_000_000_000,
          summaryUpdatedMs: 1_700_000_000_500,
        };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Language: Japanese");
      expect(rendered.container.textContent).toContain("Source: Session");
      expect(rendered.container.textContent).toContain("Input updated:");
      expect(rendered.container.textContent).toContain("Summary updated:");
    });
  });

  it("shows language=English label when summary language is en", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return {
          ...sessionSummaryFixture,
          language: "en",
        };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Language: English");
    });
  });

  it("shows language=Auto label when summary language is auto", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return {
          ...sessionSummaryFixture,
          language: "auto",
        };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Language: Auto");
    });
  });

  it("shows linked issue with no labels as labels: none", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue")
        return { ...linkedIssueFixture, labels: [] };
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
      expect(rendered.getByText("labels: none")).toBeTruthy();
    });
  });

  it("shows linked issue with updatedAt timestamp", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue")
        return { ...linkedIssueFixture, updatedAt: "2026-02-17T00:00:00Z" };
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
      expect(rendered.container.textContent).toContain("updated:");
    });
  });

  it("shows Docker tab with HostOS runtime when docker_force_host is true", async () => {
    const hostOsEntry = {
      ...quickStartDockerEntry,
      docker_force_host: true,
      docker_service: "",
      docker_container_name: "",
      docker_compose_args: [],
      docker_recreate: undefined,
      docker_build: undefined,
      docker_keep: undefined,
    };

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [hostOsEntry];
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
    const dockerTab = tabs[4] as HTMLElement;
    await fireEvent.click(dockerTab);

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("runtime: HostOS");
      expect(rendered.container.textContent).toContain("force-host: on");
    });
  });

  it("shows Docker tab with docker_recreate and docker_keep flags", async () => {
    const dockerEntry = {
      ...quickStartDockerEntry,
      docker_recreate: true,
      docker_build: false,
      docker_keep: true,
    };

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [dockerEntry];
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
    const dockerTab = tabs[4] as HTMLElement;
    await fireEvent.click(dockerTab);

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("recreate: on");
      expect(rendered.container.textContent).toContain("build: off");
      expect(rendered.container.textContent).toContain("keep: on");
    });
  });

  it("clears caches when projectPath changes", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_branch_quick_start", {
        projectPath: "/tmp/project",
        branch: "feature/markdown-ui",
      });
    });

    await rendered.rerender({
      projectPath: "/tmp/project2",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(
          ([cmd, payload]) =>
            cmd === "get_branch_quick_start" && payload?.projectPath === "/tmp/project2"
        )
      ).toBe(true);
    });
  });

  it("fires onLaunchAgent when Launch Agent button is clicked", async () => {
    const onLaunchAgent = vi.fn();
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onLaunchAgent,
    });

    await waitFor(() => {
      expect(rendered.getByRole("button", { name: "Launch Agent" })).toBeTruthy();
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Launch Agent" }));
    expect(onLaunchAgent).toHaveBeenCalledTimes(1);
  });

  it("handles session-summary-updated event from Tauri", async () => {
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

    await waitFor(() => {
      expect(typeof listeners["session-summary-updated"]).toBe("function");
    });

    // Fire a session-summary-updated event with updated markdown
    listeners["session-summary-updated"]({
      payload: {
        projectPath: "/tmp/project",
        branch: "feature/markdown-ui",
        result: {
          status: "ok",
          generating: false,
          toolId: "claude",
          sessionId: "session-1",
          sourceType: "scrollback",
          inputMtimeMs: 1_700_000_000_000,
          summaryUpdatedMs: 1_700_000_000_500,
          markdown: "## Updated Summary\nNew content",
          warning: null,
          error: null,
        },
      },
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Updated Summary");
    });
  });

  it("ignores session-summary-updated event for different project", async () => {
    const listeners: Record<string, (event: { payload: any }) => void> = {};
    listenMock.mockImplementation(async (eventName: string, handler: any) => {
      listeners[eventName] = handler;
      return () => {};
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(typeof listeners["session-summary-updated"]).toBe("function");
    });

    listeners["session-summary-updated"]({
      payload: {
        projectPath: "/tmp/other-project",
        branch: "feature/markdown-ui",
        result: {
          status: "ok",
          generating: false,
          toolId: "claude",
          sessionId: "session-1",
          markdown: "## Should Not Appear",
          warning: null,
          error: null,
        },
      },
    });

    // The original summary should remain unchanged
    await waitFor(() => {
      expect(rendered.container.textContent).not.toContain("Should Not Appear");
    });
  });

  it("ignores session-summary-updated payloads that are empty, branch-mismatched, or session-mismatched", async () => {
    const listeners: Record<string, (event: { payload: any }) => void> = {};
    listenMock.mockImplementation(async (eventName: string, handler: any) => {
      listeners[eventName] = handler;
      return () => {};
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(typeof listeners["session-summary-updated"]).toBe("function");
      expect(rendered.container.textContent).toContain("Summary body");
    });

    listeners["session-summary-updated"]({ payload: null });
    listeners["session-summary-updated"]({
      payload: {
        projectPath: "/tmp/project",
        branch: "feature/other-branch",
        result: {
          ...sessionSummaryFixture,
          sessionId: "session-1",
          markdown: "## Wrong Branch",
        },
      },
    });
    listeners["session-summary-updated"]({
      payload: {
        projectPath: "/tmp/project",
        branch: "feature/markdown-ui",
        result: {
          ...sessionSummaryFixture,
          sessionId: null,
          markdown: "## Missing Session",
        },
      },
    });
    listeners["session-summary-updated"]({
      payload: {
        projectPath: "/tmp/project",
        branch: "feature/markdown-ui",
        result: {
          ...sessionSummaryFixture,
          sessionId: "session-999",
          markdown: "## Different Session",
        },
      },
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Summary body");
      expect(rendered.container.textContent).not.toContain("Wrong Branch");
      expect(rendered.container.textContent).not.toContain("Missing Session");
      expect(rendered.container.textContent).not.toContain("Different Session");
    });
  });

  it("shows quick launch error message when callback rejects with a string", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [quickStartDockerEntry];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const onQuickLaunch = vi.fn().mockImplementation(async () => {
      throw "launch failed";
    });
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onQuickLaunch,
    });

    const continueButton = rendered.getByRole("button", { name: "Continue" }) as HTMLButtonElement;
    await waitFor(() => {
      expect(continueButton.disabled).toBe(false);
    });

    await fireEvent.click(continueButton);

    await waitFor(() => {
      expect(rendered.getByText("Failed to launch: launch failed")).toBeTruthy();
    });
  });

  it("loads deferred tab data even when requestAnimationFrame is unavailable", async () => {
    const originalRaf = window.requestAnimationFrame;
    (window as any).requestAnimationFrame = undefined;

    try {
      const rendered = await renderPanel({
        projectPath: "/tmp/project",
        selectedBranch: branchFixture,
      });

      await waitFor(() => {
        expect(rendered.container.querySelectorAll(".summary-tab").length).toBe(5);
      });

      const issueTab = Array.from(rendered.container.querySelectorAll(".summary-tab")).find(
        (tab) => tab.textContent?.trim() === "Issue",
      ) as HTMLElement | undefined;
      expect(issueTab).toBeTruthy();
      await fireEvent.click(issueTab!);

      await waitFor(() => {
        expect(invokeMock).toHaveBeenCalledWith("fetch_branch_linked_issue", {
          projectPath: "/tmp/project",
          branch: "feature/markdown-ui",
        });
      });
    } finally {
      window.requestAnimationFrame = originalRaf;
    }
  });

  it("shows Git tab with ahead/behind counters", async () => {
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

    const branchWithDivergence: BranchInfo = {
      ...branchFixture,
      ahead: 3,
      behind: 2,
      is_current: true,
      divergence_status: "Diverged",
    };

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchWithDivergence,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const gitTab = tabs[1] as HTMLElement;
    await fireEvent.click(gitTab);

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("(+3)");
      expect(rendered.container.textContent).toContain("(-2)");
      expect(rendered.container.textContent).toContain("Yes");
      expect(rendered.container.textContent).toContain("Diverged");
    });
  });

  it("opens CI log in external URL when onOpenCiLog is not provided", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return latestPrFixture;
      if (cmd === "fetch_pr_detail")
        return {
          ...prDetailFixture,
          url: "https://github.com/test/repo/pull/42",
          checkSuites: [
            {
              workflowName: "CI Build",
              runId: 100,
              status: "completed",
              conclusion: "failure",
            },
          ],
        };
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      // No onOpenCiLog provided
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[3] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(rendered.container.querySelector(".checks-section")).toBeTruthy();
    });

    const checksToggle = rendered.container.querySelector(".checks-toggle") as HTMLElement | null;
    expect(checksToggle).toBeTruthy();
    await fireEvent.click(checksToggle!);

    const checkItem = rendered.container.querySelector(".check-item") as HTMLElement | null;
    expect(checkItem).toBeTruthy();
    await fireEvent.click(checkItem!);
    await waitFor(() => {
      expect(openExternalUrlMock).toHaveBeenCalledWith(
        "https://github.com/test/repo/actions/runs/100"
      );
    });
  });

  it("fires onOpenCiLog callback when provided and CI row is clicked", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return latestPrFixture;
      if (cmd === "fetch_pr_detail")
        return {
          ...prDetailFixture,
          checkSuites: [
            {
              workflowName: "CI Build",
              runId: 200,
              status: "completed",
              conclusion: "success",
            },
          ],
        };
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const onOpenCiLog = vi.fn();
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onOpenCiLog,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[3] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(rendered.container.querySelector(".checks-section")).toBeTruthy();
    });

    const checksToggle = rendered.container.querySelector(".checks-toggle") as HTMLElement | null;
    expect(checksToggle).toBeTruthy();
    await fireEvent.click(checksToggle!);

    const checkItem = rendered.container.querySelector(".check-item") as HTMLElement | null;
    expect(checkItem).toBeTruthy();
    await fireEvent.click(checkItem!);
    await waitFor(() => {
      expect(onOpenCiLog).toHaveBeenCalledWith(200);
    });
  });

  it("shows PR detail error when fetch_pr_detail fails", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return latestPrFixture;
      if (cmd === "fetch_pr_detail") throw new Error("detail lookup failed");
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
      expect(rendered.container.textContent).toContain("detail lookup failed");
    });
  });

  it("does not skip disabled/ai-not-configured polling", async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return { ...sessionSummaryFixture, status: "disabled", markdown: null };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      agentTabBranches: ["feature/markdown-ui"],
      activeAgentTabBranch: "feature/markdown-ui",
    });

    await waitFor(() => {
      expect(sessionSummaryCalls()).toHaveLength(1);
    });

    // Even after 15s, the disabled status should not trigger additional calls
    await vi.advanceTimersByTimeAsync(15_000);
    expect(sessionSummaryCalls()).toHaveLength(1);
  });

  it("shows Gemini tool name in Docker tab quick start history", async () => {
    const geminiEntry = {
      ...quickStartDockerEntry,
      tool_id: "gemini-cli",
      tool_label: "Gemini",
      session_id: "session-gemini",
    };

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [geminiEntry];
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
    const dockerTab = tabs[4] as HTMLElement;
    await fireEvent.click(dockerTab);

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Gemini");
    });
  });

  it("maps gemini tool id to quick launch agentId", async () => {
    const geminiEntry = {
      ...quickStartDockerEntry,
      tool_id: "gemini-cli",
      tool_label: "Some Gemini Label",
      session_id: "session-gemini-launch",
      timestamp: quickStartDockerEntry.timestamp + 20,
    };

    const onQuickLaunch = vi.fn(async () => {});
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [geminiEntry];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onQuickLaunch,
    });

    await waitFor(() => {
      const continueBtn = rendered.getByRole("button", { name: "Continue" }) as HTMLButtonElement;
      expect(continueBtn.disabled).toBe(false);
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Continue" }));

    await waitFor(() => {
      expect(onQuickLaunch).toHaveBeenCalled();
    });

    expect(onQuickLaunch).toHaveBeenCalledWith(
      expect.objectContaining({
        agentId: "gemini",
        mode: "continue",
        resumeSessionId: "session-gemini-launch",
      }),
    );
  });

  it("maps github-copilot tool id to quick launch agentId", async () => {
    const copilotEntry = {
      ...quickStartDockerEntry,
      tool_id: "github-copilot",
      tool_label: "GitHub Copilot",
      session_id: "session-copilot-launch",
      timestamp: quickStartDockerEntry.timestamp + 21,
    };

    const onQuickLaunch = vi.fn(async () => {});
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [copilotEntry];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onQuickLaunch,
    });

    await waitFor(() => {
      const continueBtn = rendered.getByRole("button", { name: "Continue" }) as HTMLButtonElement;
      expect(continueBtn.disabled).toBe(false);
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Continue" }));

    await waitFor(() => {
      expect(onQuickLaunch).toHaveBeenCalled();
    });

    expect(onQuickLaunch).toHaveBeenCalledWith(
      expect.objectContaining({
        agentId: "copilot",
        mode: "continue",
        resumeSessionId: "session-copilot-launch",
      }),
    );
  });

  it("maps open-code tool id to opencode and keeps docker history entries with compose args", async () => {
    const openCodeComposeOnly = {
      ...quickStartDockerEntry,
      tool_id: "open-code-cli",
      tool_label: "Custom Label",
      session_id: "session-open-code-compose-only",
      docker_service: "",
      docker_recreate: undefined,
      docker_build: undefined,
      docker_keep: undefined,
      docker_force_host: undefined,
      docker_container_name: "",
      docker_compose_args: ["--profile", "dev"],
      timestamp: quickStartDockerEntry.timestamp + 30,
    };
    const noDockerInfoEntry = {
      ...quickStartDockerEntry,
      tool_id: "claude",
      session_id: "session-no-docker-info",
      docker_service: "",
      docker_recreate: undefined,
      docker_build: undefined,
      docker_keep: undefined,
      docker_force_host: undefined,
      docker_container_name: "",
      docker_compose_args: [],
      timestamp: quickStartDockerEntry.timestamp + 10,
    };

    const onQuickLaunch = vi.fn(async () => {});
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [noDockerInfoEntry, openCodeComposeOnly];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return null;
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onQuickLaunch,
    });

    await waitFor(() => {
      const continueBtn = rendered.getByRole("button", { name: "Continue" }) as HTMLButtonElement;
      expect(continueBtn.disabled).toBe(false);
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Continue" }));
    await waitFor(() => {
      expect(onQuickLaunch).toHaveBeenCalled();
    });
    expect(onQuickLaunch).toHaveBeenCalledWith(
      expect.objectContaining({
        agentId: "opencode",
        mode: "continue",
      }),
    );

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const dockerTab = tabs[4] as HTMLElement;
    await fireEvent.click(dockerTab);

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("OpenCode");
      expect(rendered.container.textContent).toContain("1 record");
      expect(rendered.container.textContent).toContain("compose args: --profile dev");
    });

    expect(rendered.container.textContent).not.toContain("session-no-docker-info");
  });

  it("falls back to tool_label/tool_id for unknown tools in Docker history", async () => {
    const unknownWithLabel = {
      ...quickStartDockerEntry,
      tool_id: "mystery-tool",
      tool_label: "Mystery Label",
      session_id: "session-unknown-with-label",
      timestamp: quickStartDockerEntry.timestamp + 40,
    };
    const unknownWithId = {
      ...quickStartDockerEntry,
      tool_id: "mystery-id-only",
      tool_label: "",
      session_id: "session-unknown-with-id",
      timestamp: quickStartDockerEntry.timestamp + 39,
    };

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [unknownWithLabel, unknownWithId];
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
    const dockerTab = tabs[4] as HTMLElement;
    await fireEvent.click(dockerTab);

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Mystery Label");
      expect(rendered.container.textContent).toContain("mystery-id-only");
    });
  });

  it("shows merge error toast when merge_pull_request fails", async () => {
    const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      invokeMock.mockImplementation(async (cmd: string) => {
        if (cmd === "get_branch_quick_start") return [];
        if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
        if (cmd === "fetch_branch_linked_issue") return null;
        if (cmd === "fetch_latest_branch_pr") return latestPrFixture;
        if (cmd === "fetch_pr_detail") return prDetailFixture;
        if (cmd === "merge_pull_request") throw new Error("Merge conflict");
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
        expect(rendered.getByRole("button", { name: "Mergeable" })).toBeTruthy();
      });

      await fireEvent.click(rendered.getByRole("button", { name: "Mergeable" }));
      await waitFor(() => {
        expect(rendered.getByRole("button", { name: "Merge" })).toBeTruthy();
      });

      await fireEvent.click(rendered.getByRole("button", { name: "Merge" }));

      await waitFor(() => {
        expect(consoleErrorSpy).toHaveBeenCalledWith(
          "Failed to merge PR:",
          expect.any(Error),
        );
      });
    } finally {
      consoleErrorSpy.mockRestore();
    }
  });

  it("shows rebuild progress with branch name when available", async () => {
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
        total: 5,
        completed: 2,
        branch: "feature/some-branch",
        status: "in_progress",
        error: null,
      },
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Rebuilding summaries (2/5)");
      expect(rendered.container.textContent).toContain("- feature/some-branch");
    });
  });

  it("shows rebuild progress without branch suffix when branch is null", async () => {
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

    await waitFor(() => {
      expect(typeof listeners["session-summary-rebuild-progress"]).toBe("function");
    });

    listeners["session-summary-rebuild-progress"]({
      payload: {
        projectPath: "/tmp/project",
        language: "en",
        total: 3,
        completed: 1,
        branch: null,
        status: "in_progress",
        error: null,
      },
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("Rebuilding summaries (1/3)");
      expect(rendered.container.textContent).not.toContain("- feature/");
    });
  });

  it("ignores rebuild progress from different project path", async () => {
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

    await waitFor(() => {
      expect(typeof listeners["session-summary-rebuild-progress"]).toBe("function");
    });

    listeners["session-summary-rebuild-progress"]({
      payload: {
        projectPath: "/tmp/other-project",
        language: "ja",
        total: 3,
        completed: 0,
        branch: null,
        status: "started",
        error: null,
      },
    });

    await new Promise((r) => setTimeout(r, 50));
    expect(rendered.queryByText("Rebuilding summaries")).toBeNull();
  });

  it("ignores rebuild progress when payload is null", async () => {
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

    await waitFor(() => {
      expect(typeof listeners["session-summary-rebuild-progress"]).toBe("function");
    });

    // Should not throw
    listeners["session-summary-rebuild-progress"]({
      payload: null,
    });

    await new Promise((r) => setTimeout(r, 50));
    expect(rendered.queryByText("Rebuilding summaries")).toBeNull();
  });

  it("shows OpenCode tool name in Docker tab quick start history", async () => {
    const opencodeEntry = {
      ...quickStartDockerEntry,
      tool_id: "opencode",
      tool_label: "OpenCode",
      session_id: "session-opencode",
    };

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [opencodeEntry];
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
    const dockerTab = tabs[4] as HTMLElement;
    await fireEvent.click(dockerTab);

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("OpenCode");
    });
  });

  it("does not open external URL when onOpenCiLog is absent and PR URL does not match github pattern", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return latestPrFixture;
      if (cmd === "fetch_pr_detail")
        return {
          ...prDetailFixture,
          // Use a non-GitHub URL so the regex does not match and workflowBase is null
          url: "https://gitlab.com/test/repo/pull/42",
          checkSuites: [
            {
              workflowName: "CI Build",
              runId: 100,
              status: "completed",
              conclusion: "failure",
            },
          ],
        };
      if (cmd === "detect_docker_context") return dockerContextFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      // No onOpenCiLog provided — exercises the fallback path
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[3] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(rendered.container.querySelector(".checks-section")).toBeTruthy();
    });

    const checksToggle = rendered.container.querySelector(".checks-toggle") as HTMLElement | null;
    expect(checksToggle).toBeTruthy();
    await fireEvent.click(checksToggle!);

    const checkItem = rendered.container.querySelector(".check-item") as HTMLElement | null;
    expect(checkItem).toBeTruthy();
    await fireEvent.click(checkItem!);
    await new Promise((r) => setTimeout(r, 50));
    expect(openExternalUrlMock).not.toHaveBeenCalled();
  });

  it("does not open external URL when onOpenCiLog is absent and prDetail URL is empty", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
      if (cmd === "fetch_branch_linked_issue") return null;
      if (cmd === "fetch_latest_branch_pr") return latestPrFixture;
      if (cmd === "fetch_pr_detail")
        return {
          ...prDetailFixture,
          url: "",
          checkSuites: [
            {
              workflowName: "CI Build",
              runId: 100,
              status: "completed",
              conclusion: "failure",
            },
          ],
        };
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
      expect(rendered.container.querySelector(".checks-section")).toBeTruthy();
    });

    const checksToggle = rendered.container.querySelector(".checks-toggle") as HTMLElement | null;
    expect(checksToggle).toBeTruthy();
    await fireEvent.click(checksToggle!);

    const checkItem = rendered.container.querySelector(".check-item") as HTMLElement | null;
    expect(checkItem).toBeTruthy();
    await fireEvent.click(checkItem!);
    await new Promise((r) => setTimeout(r, 50));
    expect(openExternalUrlMock).not.toHaveBeenCalled();
  });

  it("shows display_name in header when set", async () => {
    const branchWithDisplayName: BranchInfo = {
      ...branchFixture,
      display_name: "My custom name",
    };

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchWithDisplayName,
    });

    await waitFor(() => {
      expect(
        rendered.container.querySelector("h2 .branch-display-name")?.textContent
      ).toBe("My custom name");
    });
  });

  it("shows branch name when no display_name", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(
        rendered.container.querySelector("h2 .branch-display-name")?.textContent
      ).toBe("feature/markdown-ui");
    });
  });

  it("shows edit display name button", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      const editBtn = rendered.container.querySelector(".edit-display-name-btn");
      expect(editBtn).toBeTruthy();
    });
  });

  it("clicking edit shows display name input", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.querySelector(".edit-display-name-btn")).toBeTruthy();
    });

    const editBtn = rendered.container.querySelector(".edit-display-name-btn") as HTMLElement;
    await fireEvent.click(editBtn);

    await waitFor(() => {
      const input = rendered.container.querySelector(".display-name-input");
      expect(input).toBeTruthy();
    });
  });

  it("shows real branch name subtitle when display_name differs", async () => {
    const branchWithDisplayName: BranchInfo = {
      ...branchFixture,
      display_name: "Custom display name",
    };

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchWithDisplayName,
    });

    await waitFor(() => {
      expect(
        rendered.container.querySelector("h2 .branch-display-name")?.textContent
      ).toBe("Custom display name");
      const realName = rendered.container.querySelector(".branch-real-name");
      expect(realName).toBeTruthy();
      expect(realName?.textContent).toBe("feature/markdown-ui");
    });
  });

  it("getInvoke throws when __TAURI_INTERNALS__ is absent and tauriInvoke module invoke is null", async () => {
    const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const savedInternals = (globalThis as any).__TAURI_INTERNALS__;
    delete (globalThis as any).__TAURI_INTERNALS__;

    vi.doMock("$lib/tauriInvoke", () => ({ invoke: null }));
    vi.resetModules();

    try {
      const { default: Panel } = await import("./WorktreeSummaryPanel.svelte");
      const {
        render: isolatedRender,
        waitFor: isolatedWaitFor,
        fireEvent: isolatedFireEvent,
        cleanup: isolatedCleanup,
      } = await import("@testing-library/svelte");
      const baseInvoke = vi.fn(async (cmd: string) => {
        if (cmd === "get_branch_quick_start") return [];
        if (cmd === "get_branch_session_summary") return { ...sessionSummaryFixture, markdown: null };
        if (cmd === "fetch_branch_linked_issue") return null;
        if (cmd === "fetch_latest_branch_pr") return latestPrFixture;
        if (cmd === "fetch_pr_detail") return prDetailFixture;
        if (cmd === "detect_docker_context") return dockerContextFixture;
        return [];
      });

      Object.defineProperty(globalThis, "__TAURI_INTERNALS__", {
        value: { invoke: baseInvoke },
        configurable: true,
      });

      const rendered = isolatedRender(Panel, {
        props: {
          projectPath: "/tmp/project",
          selectedBranch: branchFixture,
        },
      });

      const tabs = rendered.container.querySelectorAll(".summary-tab");
      const prTab = tabs[3] as HTMLElement;
      await isolatedFireEvent.click(prTab);

      await isolatedWaitFor(() => {
        expect(rendered.getByRole("button", { name: "Mergeable" })).toBeTruthy();
      });

      delete (globalThis as any).__TAURI_INTERNALS__;

      await isolatedFireEvent.click(rendered.getByRole("button", { name: "Mergeable" }));
      await isolatedWaitFor(() => {
        expect(rendered.getByRole("button", { name: "Merge" })).toBeTruthy();
      });

      await isolatedFireEvent.click(rendered.getByRole("button", { name: "Merge" }));

      await isolatedWaitFor(() => {
        expect(consoleErrorSpy).toHaveBeenCalledWith(
          "Failed to merge PR:",
          expect.any(Error),
        );
      });
      isolatedCleanup();
    } finally {
      consoleErrorSpy.mockRestore();
      vi.doMock("$lib/tauriInvoke", () => ({ invoke: invokeMock }));
      vi.resetModules();
      Object.defineProperty(globalThis, "__TAURI_INTERNALS__", {
        value: savedInternals ?? { invoke: invokeMock },
        configurable: true,
      });
    }
  });

});
