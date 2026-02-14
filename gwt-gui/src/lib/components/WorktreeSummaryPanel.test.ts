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
  docker_container_name: "workspace-container",
  docker_compose_args: ["--build", "-f", "docker-compose.dev.yml"],
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
    expect(tabs).toHaveLength(6);
    expect(tabs[0]?.textContent?.trim()).toBe("Summary");
    expect(tabs[0]?.classList.contains("active")).toBe(true);
    expect(tabs[1]?.textContent?.trim()).toBe("Git");
    expect(tabs[2]?.textContent?.trim()).toBe("PR");
    expect(tabs[3]?.textContent?.trim()).toBe("Workflow");
    expect(tabs[4]?.textContent?.trim()).toBe("AI");
    expect(tabs[5]?.textContent?.trim()).toBe("Docker");
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

  it("switches to Workflow tab and shows workflow runs", async () => {
    const onOpenCiLog = vi.fn();
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

  it("switches to Docker tab and shows docker session details", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_branch_quick_start") return [quickStartDockerEntry];
      if (cmd === "get_branch_session_summary") return sessionSummaryFixture;
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
      expect(rendered.getByText("runtime: Docker")).toBeTruthy();
      expect(rendered.getByText("service: workspace")).toBeTruthy();
      expect(rendered.getByText("container: workspace-container")).toBeTruthy();
      expect(
        rendered.getByText("compose args: --build -f docker-compose.dev.yml")
      ).toBeTruthy();
      expect(rendered.getByText("Session session-456")).toBeTruthy();
    });
  });
});
