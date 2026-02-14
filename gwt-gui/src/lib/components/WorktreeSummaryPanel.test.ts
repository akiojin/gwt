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

async function renderPanel(props: any) {
  const { default: Panel } = await import("./WorktreeSummaryPanel.svelte");
  return render(Panel, { props });
}

const branchFixture = {
  name: "feature/markdown-ui",
  commit: "1234567",
  is_current: false,
  ahead: 0,
  behind: 0,
  divergence_status: "UpToDate",
  last_tool_usage: null,
};

const sessionSummaryFixture = {
  status: "ok",
  generating: false,
  toolId: "codex",
  sessionId: "session-1",
  markdown: "## 要約\n- 変更点を整理した\n- テストを追加",
  bulletPoints: ["変更点を整理した", "テストを追加"],
  error: null,
};

const quickStartHostEntry = {
  branch: branchFixture.name,
  tool_id: "codex",
  tool_label: "Codex",
  session_id: "session-123",
  mode: "normal",
  model: "gpt-5",
  reasoning_level: "high",
  skip_permissions: true,
  tool_version: "0.33.0",
  docker_force_host: true,
  timestamp: 1_700_000_001,
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
  timestamp: 1_700_000_002,
};
describe("WorktreeSummaryPanel", () => {
  beforeEach(() => {
    cleanup();
    listenMock.mockReset();
    listenMock.mockResolvedValue(() => {});
    invokeMock.mockReset();
    invokeMock.mockResolvedValue([]);
    Object.defineProperty(globalThis, "__TAURI_INTERNALS__", {
      value: { invoke: invokeMock },
      configurable: true,
    });
  });

  it("renders branch header and tab UI when branch is selected", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.container.querySelector("h2")?.textContent).toBe(
        "feature/markdown-ui"
      );
    });

    // Summary tab is active by default
    const tabs = rendered.container.querySelectorAll(".summary-tab");
    expect(tabs).toHaveLength(5);
    expect(tabs[0]?.textContent?.trim()).toBe("Summary");
    expect(tabs[0]?.classList.contains("active")).toBe(true);
    expect(tabs[1]?.textContent?.trim()).toBe("Git");
    expect(tabs[2]?.textContent?.trim()).toBe("PR");
    expect(tabs[3]?.textContent?.trim()).toBe("Workflow");
    expect(tabs[4]?.textContent?.trim()).toBe("AI");
  });

  it("shows placeholder when no branch is selected", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: null,
    });

    await waitFor(() => {
      expect(
        rendered.container.querySelector(".placeholder h2")?.textContent
      ).toBe("Worktree Summary");
    });
  });

  it("renders Quick Start section in summary tab", async () => {
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

    await waitFor(() => {
      expect(
        rendered.container.querySelector(".quick-title")?.textContent
      ).toBe("Quick Start");
    });
  });

  it("switches to PR tab and shows PrStatusSection", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[2] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(prTab.classList.contains("active")).toBe(true);
      expect(
        rendered.container.querySelector(".pr-status-section")
      ).toBeTruthy();
    });
  });

  it("shows GitHub CLI auth warning in PR tab when CLI is unavailable", async () => {
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      ghCliStatus: { available: false, authenticated: false },
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const prTab = tabs[2] as HTMLElement;
    await fireEvent.click(prTab);

    await waitFor(() => {
      expect(
        rendered.getByText("GitHub CLI (gh) is not available.")
      ).toBeTruthy();
    });
  });

  it("switches to Git tab and keeps Git section expanded", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary")
        return {
          ...sessionSummaryFixture,
          markdown: null,
          bulletPoints: [],
        };
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
      expect(
        rendered.container.querySelector(".git-section .git-body")
      ).toBeTruthy();
    });

    // Git tab mode disables collapse UI.
    expect(rendered.container.querySelector(".git-section .collapse-icon")).toBeNull();
  });

  it("switches to Workflow tab and opens workflow run page", async () => {
    const windowOpen = vi.spyOn(window, "open").mockImplementation(() => null);
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") {
        return {
          ...sessionSummaryFixture,
          markdown: null,
          bulletPoints: [],
        };
      }
      if (cmd === "fetch_pr_detail") {
        return {
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
      }
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      prNumber: 42,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const workflowTab = tabs[3] as HTMLElement;
    await fireEvent.click(workflowTab);

    await waitFor(() => {
      expect(workflowTab.classList.contains("active")).toBe(true);
      expect(rendered.getByText("Success")).toBeTruthy();
      expect(rendered.getByText("Running")).toBeTruthy();
      expect(rendered.queryByText("CI Build")).toBeNull();
      expect(rendered.queryByText("Lint")).toBeNull();
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

  it("displays HostOS runtime for quick start entry", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [quickStartHostEntry];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.getByText("runtime: HostOS")).toBeTruthy();
    });
  });

  it("displays Docker runtime and service for quick start entry", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [quickStartDockerEntry];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(rendered.getByText("runtime: Docker")).toBeTruthy();
      expect(rendered.getByText("service: workspace")).toBeTruthy();
    });
  });

  it("calls onLaunchAgent when Launch Agent button is clicked", async () => {
    const onLaunchAgent = vi.fn();
    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onLaunchAgent,
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Launch Agent..." }));
    expect(onLaunchAgent).toHaveBeenCalledTimes(1);
  });

  it("shows quick start error when quick start loading fails", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") throw new Error("quick start failed");
      if (cmd === "get_branch_session_summary") {
        return {
          status: "no-session",
          generating: false,
          bulletPoints: [],
        };
      }
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    await waitFor(() => {
      expect(
        rendered.getByText("Failed to load Quick Start: quick start failed")
      ).toBeTruthy();
    });
  });

  it("shows quick launch error when quick launch fails", async () => {
    const onQuickLaunch = vi.fn(async () => {
      throw new Error("launch failed");
    });

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [quickStartHostEntry];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      onQuickLaunch,
    });

    await waitFor(() => {
      expect(rendered.getByRole("button", { name: "Continue" })).toBeTruthy();
    });
    await fireEvent.click(rendered.getByRole("button", { name: "Continue" }));

    await waitFor(() => {
      expect(onQuickLaunch).toHaveBeenCalledWith(
        expect.objectContaining({
          agentId: "codex",
          branch: branchFixture.name,
          mode: "continue",
          resumeSessionId: "session-123",
        })
      );
      expect(rendered.getByText("Failed to launch: launch failed")).toBeTruthy();
    });
  });

  it("shows AI not configured message in AI tab", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") {
        return {
          status: "ai-not-configured",
          generating: false,
          bulletPoints: [],
        };
      }
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const aiTab = tabs[4] as HTMLElement;
    await fireEvent.click(aiTab);

    await waitFor(() => {
      expect(
        rendered.getByText("Configure AI in Settings to enable session summary.")
      ).toBeTruthy();
    });
  });

  it("shows AI error message in AI tab", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") {
        return {
          status: "error",
          generating: false,
          error: "summary failed",
          bulletPoints: [],
        };
      }
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const aiTab = tabs[4] as HTMLElement;
    await fireEvent.click(aiTab);

    await waitFor(() => {
      expect(rendered.getByText("summary failed")).toBeTruthy();
    });
  });

  it("shows workflow no-pr state when prNumber is absent", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      prNumber: null,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const workflowTab = tabs[3] as HTMLElement;
    await fireEvent.click(workflowTab);

    await waitFor(() => {
      expect(rendered.getByText("No PR.")).toBeTruthy();
    });
  });

  it("shows workflow error when pr detail fetch fails", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_pr_detail") throw new Error("pr detail failed");
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      prNumber: 42,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const workflowTab = tabs[3] as HTMLElement;
    await fireEvent.click(workflowTab);

    await waitFor(() => {
      expect(rendered.getByText("pr detail failed")).toBeTruthy();
    });
  });

  it("ignores session-summary-updated event when session id is different", async () => {
    listenMock.mockResolvedValue(() => {});

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") {
        return {
          status: "ok",
          generating: false,
          markdown: "old summary",
          toolId: "codex",
          sessionId: "session-1",
          bulletPoints: [],
        };
      }
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const aiTab = tabs[4] as HTMLElement;
    await fireEvent.click(aiTab);

    await waitFor(() => {
      expect(rendered.getByText("old summary")).toBeTruthy();
      expect(
        listenMock.mock.calls.some(
          (call) =>
            call[0] === "session-summary-updated" && typeof call[1] === "function"
        )
      ).toBe(true);
    });

    const summaryHandler = listenMock.mock.calls.find(
      (call) => call[0] === "session-summary-updated"
    )?.[1] as ((event: { payload: any }) => void) | undefined;
    if (!summaryHandler) {
      throw new Error("summaryUpdatedHandler is not registered");
    }
    summaryHandler({
      payload: {
        projectPath: "/tmp/project",
        branch: branchFixture.name,
        result: {
          status: "ok",
          generating: false,
          markdown: "new summary",
          toolId: "codex",
          sessionId: "session-other",
          bulletPoints: [],
        },
      },
    });

    await waitFor(() => {
      expect(rendered.getByText("old summary")).toBeTruthy();
    });
  });

  it("renders divergence counts and maps quick start tool variants", async () => {
    const onQuickLaunch = vi.fn(async () => undefined);
    const branchWithDivergence = {
      ...branchFixture,
      ahead: 2,
      behind: 1,
    };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") {
        return [
          {
            ...quickStartHostEntry,
            tool_id: "gemini-cli",
            tool_label: "Gemini CLI",
            session_id: "session-gemini",
            model: "",
            timestamp: 1_700_000_010,
          },
          {
            ...quickStartHostEntry,
            tool_id: "open-code",
            tool_label: "OpenCode CLI",
            session_id: "session-opencode",
            docker_force_host: false,
            docker_service: "",
            docker_recreate: undefined,
            docker_build: undefined,
            docker_keep: undefined,
            model: "",
            timestamp: 1_700_000_011,
          },
          {
            ...quickStartHostEntry,
            tool_id: "custom-tool",
            tool_label: "Custom",
            session_id: "session-custom",
            model: "",
            timestamp: 1_700_000_012,
          },
        ];
      }
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchWithDivergence,
      onQuickLaunch,
    });

    await waitFor(() => {
      expect(rendered.container.textContent).toContain("(+2)");
      expect(rendered.container.textContent).toContain("(-1)");
      expect(rendered.getByText("Gemini")).toBeTruthy();
      expect(rendered.getByText("OpenCode")).toBeTruthy();
      expect(rendered.getByText("Custom")).toBeTruthy();
    });

    const newButtons = rendered.getAllByRole("button", { name: "New" });
    await fireEvent.click(newButtons[0] as HTMLButtonElement);
    await waitFor(() => {
      expect(onQuickLaunch).toHaveBeenCalledWith(
        expect.objectContaining({
          agentId: "gemini",
          branch: branchWithDivergence.name,
          mode: "normal",
        })
      );
    });
  });

  it("shows unauthenticated GitHub CLI warning in workflow tab", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      ghCliStatus: { available: true, authenticated: false },
      prNumber: 42,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const workflowTab = tabs[3] as HTMLElement;
    await fireEvent.click(workflowTab);

    await waitFor(() => {
      expect(rendered.getByText("GitHub CLI issue")).toBeTruthy();
      expect(
        rendered.getByText("GitHub CLI (gh) is not authenticated. Run: gh auth login")
      ).toBeTruthy();
    });
  });

  it("renders workflow status labels for all supported conclusions", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_pr_detail") {
        return {
          number: 101,
          title: "Workflow matrix",
          state: "OPEN",
          url: "https://github.com/test/repo/pull/101",
          mergeable: "MERGEABLE",
          author: "alice",
          baseBranch: "main",
          headBranch: branchFixture.name,
          labels: [],
          assignees: [],
          milestone: null,
          linkedIssues: [],
          checkSuites: [
            { workflowName: "Q", runId: 1, status: "queued", conclusion: null },
            { workflowName: "P", runId: 2, status: "waiting", conclusion: null },
            { workflowName: "F", runId: 3, status: "completed", conclusion: "failure" },
            { workflowName: "N", runId: 4, status: "completed", conclusion: "neutral" },
            { workflowName: "S", runId: 5, status: "completed", conclusion: "skipped" },
            { workflowName: "C", runId: 6, status: "completed", conclusion: "cancelled" },
            { workflowName: "T", runId: 7, status: "completed", conclusion: "timed_out" },
            { workflowName: "A", runId: 8, status: "completed", conclusion: "action_required" },
            { workflowName: "U", runId: 9, status: "completed", conclusion: "mystery" },
          ],
          reviews: [],
          reviewComments: [],
          changedFilesCount: 0,
          additions: 0,
          deletions: 0,
        };
      }
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      prNumber: 101,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const workflowTab = tabs[3] as HTMLElement;
    await fireEvent.click(workflowTab);

    await waitFor(() => {
      expect(rendered.getByText("Queued")).toBeTruthy();
      expect(rendered.getByText("Pending")).toBeTruthy();
      expect(rendered.getByText("Failed")).toBeTruthy();
      expect(rendered.getByText("Neutral")).toBeTruthy();
      expect(rendered.getByText("Skipped")).toBeTruthy();
      expect(rendered.getByText("Cancelled")).toBeTruthy();
      expect(rendered.getByText("Timed out")).toBeTruthy();
      expect(rendered.getByText("Action required")).toBeTruthy();
      expect(rendered.getByText("Unknown")).toBeTruthy();
    });
  });

  it("does not open workflow run page when PR URL is invalid", async () => {
    const windowOpen = vi.spyOn(window, "open").mockImplementation(() => null);
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
      if (cmd === "fetch_pr_detail") {
        return {
          number: 7,
          title: "Invalid URL",
          state: "OPEN",
          url: "not-a-url",
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
              workflowName: "CI",
              runId: 100,
              status: "completed",
              conclusion: "success",
            },
          ],
          reviews: [],
          reviewComments: [],
          changedFilesCount: 0,
          additions: 0,
          deletions: 0,
        };
      }
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
      prNumber: 7,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const workflowTab = tabs[3] as HTMLElement;
    await fireEvent.click(workflowTab);
    await waitFor(() => {
      expect(rendered.getByText("Success")).toBeTruthy();
    });

    await fireEvent.click(rendered.getByText("Success").closest("button") as HTMLButtonElement);
    expect(windowOpen).not.toHaveBeenCalled();
    windowOpen.mockRestore();
  });

  it("renders AI disabled state with warning", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") {
        return {
          status: "disabled",
          generating: false,
          markdown: null,
          toolId: null,
          sessionId: null,
          warning: "summary disabled by policy",
          bulletPoints: [],
          error: null,
        };
      }
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const aiTab = tabs[4] as HTMLElement;
    await fireEvent.click(aiTab);

    await waitFor(() => {
      expect(rendered.getByText("Disabled")).toBeTruthy();
      expect(rendered.getByText("summary disabled by policy")).toBeTruthy();
      expect(rendered.getByText("Session summary disabled.")).toBeTruthy();
    });
  });

  it("renders AI live generating state for pane session", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [];
      if (cmd === "get_branch_session_summary") {
        return {
          status: "ok",
          generating: true,
          markdown: null,
          toolId: "codex",
          sessionId: "pane:42",
          warning: null,
          bulletPoints: [],
          error: null,
        };
      }
      return [];
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      selectedBranch: branchFixture,
    });

    const tabs = rendered.container.querySelectorAll(".summary-tab");
    const aiTab = tabs[4] as HTMLElement;
    await fireEvent.click(aiTab);
    await waitFor(() => {
      expect(rendered.container.textContent).toContain("codex - Live (pane summary)");
      expect(rendered.getByText("Generating...")).toBeTruthy();
    });
  });
});
