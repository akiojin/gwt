import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";
import type { BranchInfo } from "../types";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...(args as [string, Record<string, unknown> | undefined])),
}));

type AgentSidebarProps = {
  projectPath: string;
  currentBranch?: string;
  selectedBranch?: BranchInfo | null;
  agentTabBranches?: string[];
  activeAgentTabBranch?: string | null;
  preferredLanguage?: "auto" | "ja" | "en";
};

async function renderSidebar(props: AgentSidebarProps) {
  const { default: AgentSidebar } = await import("./AgentSidebar.svelte");
  return render(AgentSidebar, { props });
}

describe("AgentSidebar", () => {
  beforeEach(async () => {
    cleanup();
    invokeMock.mockReset();
    await new Promise((r) => setTimeout(r, 0));
  });

  it("switches assigned agents when selecting a task", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return {
          issueNumber: 1438,
          tasks: [
            {
              id: "T001",
              title: "T001 [US1] implement auth",
              status: "running",
              subAgents: [
                {
                  id: "sess-codex",
                  name: "Codex",
                  toolId: "codex-cli",
                  status: "running",
                  model: "gpt-5-codex",
                  branch: "feature/auth",
                  worktreeRelPath: ".worktrees/feature-auth",
                  worktreeAbsPath: "/repo/.worktrees/feature-auth",
                },
              ],
            },
            {
              id: "T002",
              title: "T002 [US2] add tests",
              status: "pending",
              subAgents: [
                {
                  id: "sess-claude",
                  name: "Claude",
                  toolId: "claude-code",
                  status: "completed",
                  model: "opus",
                  branch: "feature/auth-tests",
                  worktreeRelPath: ".worktrees/feature-auth-tests",
                  worktreeAbsPath: "/repo/.worktrees/feature-auth-tests",
                },
              ],
            },
          ],
        };
      }
      if (command === "get_branch_session_summary") {
        return {
          status: "no-session",
          generating: false,
          bulletPoints: [],
        };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
      selectedBranch: null,
    });

    await waitFor(() => {
      expect(rendered.getByText("T001 [US1] implement auth")).toBeTruthy();
    });

    expect(rendered.getByText("Codex")).toBeTruthy();
    expect(() => rendered.getByText("Claude")).toThrow();

    const taskButton = rendered.getByTestId("agent-task-T002");
    await fireEvent.click(taskButton);

    await waitFor(() => {
      expect(rendered.getByText("Claude")).toBeTruthy();
    });
    expect(() => rendered.getByText("Codex")).toThrow();

    const relPath = rendered.getByText(".worktrees/feature-auth-tests");
    expect(relPath.getAttribute("title")).toBe("/repo/.worktrees/feature-auth-tests");
  });

  it("shows empty state when no tasks are returned", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return {
          status: "no-session",
          generating: false,
          bulletPoints: [],
        };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
      selectedBranch: null,
    });

    await waitFor(() => {
      expect(rendered.getByText("No tasks yet.")).toBeTruthy();
    });
  });

  it("renders tasks in status order: running > pending > failed > completed", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return {
          issueNumber: 1438,
          tasks: [
            { id: "T004", title: "failed task", status: "failed", subAgents: [] },
            { id: "T001", title: "completed task", status: "completed", subAgents: [] },
            { id: "T003", title: "running task", status: "running", subAgents: [] },
            { id: "T002", title: "pending task", status: "pending", subAgents: [] },
          ],
        };
      }
      if (command === "get_branch_session_summary") {
        return {
          status: "no-session",
          generating: false,
          bulletPoints: [],
        };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
      selectedBranch: null,
    });

    await waitFor(() => {
      expect(rendered.getByText("running task")).toBeTruthy();
    });

    const rows = Array.from(
      rendered.container.querySelectorAll<HTMLButtonElement>(".task-row"),
    ).map((el) => el.getAttribute("data-testid"));

    expect(rows).toEqual([
      "agent-task-T003",
      "agent-task-T002",
      "agent-task-T004",
      "agent-task-T001",
    ]);
  });

  it("shows all assigned agents for a task", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return {
          issueNumber: 1438,
          tasks: [
            {
              id: "T010",
              title: "T010 multi-assignee task",
              status: "running",
              subAgents: [
                {
                  id: "sess-codex",
                  name: "Codex",
                  toolId: "codex-cli",
                  status: "running",
                  model: "gpt-5-codex",
                  branch: "feature/a",
                  worktreeRelPath: ".worktrees/feature-a",
                  worktreeAbsPath: "/repo/.worktrees/feature-a",
                },
                {
                  id: "sess-claude",
                  name: "Claude",
                  toolId: "claude-code",
                  status: "completed",
                  model: "opus",
                  branch: "feature/b",
                  worktreeRelPath: ".worktrees/feature-b",
                  worktreeAbsPath: "/repo/.worktrees/feature-b",
                },
              ],
            },
          ],
        };
      }
      if (command === "get_branch_session_summary") {
        return {
          status: "no-session",
          generating: false,
          bulletPoints: [],
        };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
      selectedBranch: null,
    });

    await waitFor(() => {
      expect(rendered.getByText("T010 multi-assignee task")).toBeTruthy();
    });

    expect(rendered.getByText("Codex")).toBeTruthy();
    expect(rendered.getByText("Claude")).toBeTruthy();
    expect(rendered.getByText(".worktrees/feature-a")).toBeTruthy();
    expect(rendered.getByText(".worktrees/feature-b")).toBeTruthy();
  });

  // --- New tests for coverage improvement ---

  it("displays spec ID when present in the sidebar view", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return {
          issueNumber: 1438,
          tasks: [],
        };
      }
      if (command === "get_branch_session_summary") {
        return { status: "no-session", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("#1438")).toBeTruthy();
    });
  });

  it("shows 'Agent Tasks' title", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "no-session", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    expect(rendered.getByText("Agent Tasks")).toBeTruthy();
  });

  it("shows 'Select a branch to view tasks.' when no branch is active", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "no-session", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "",
      selectedBranch: null,
    });

    await waitFor(() => {
      expect(rendered.getByText("Select a branch to view tasks.")).toBeTruthy();
    });
  });

  it("shows branch name in agent-branch when available", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "no-session", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "feature/my-feature",
    });

    await waitFor(() => {
      expect(rendered.getByText("feature/my-feature")).toBeTruthy();
    });
  });

  it("normalizes origin/ prefix from branch names", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "no-session", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "origin/feature/remote-branch",
    });

    await waitFor(() => {
      expect(rendered.getByText("feature/remote-branch")).toBeTruthy();
    });
  });

  it("shows error message when sidebar view loading fails", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        throw new Error("API error");
      }
      if (command === "get_branch_session_summary") {
        return { status: "no-session", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText(/Failed to load tasks/)).toBeTruthy();
    });
  });

  it("shows 'Select a task to view assigned agents.' when no task selected and tasks empty", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "no-session", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("Select a task to view assigned agents.")).toBeTruthy();
    });
  });

  it("shows 'No assigned agents.' when task has empty subAgents", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return {
          issueNumber: null,
          tasks: [
            { id: "T001", title: "Empty agents task", status: "running", subAgents: [] },
          ],
        };
      }
      if (command === "get_branch_session_summary") {
        return { status: "no-session", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("No assigned agents.")).toBeTruthy();
    });
  });

  it("displays summary status labels correctly", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "ok", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("Ready")).toBeTruthy();
    });
  });

  it("shows 'Generating' label when summary is generating", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "ok", generating: true };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("Generating")).toBeTruthy();
    });
  });

  it("shows 'No session' label for no-session status", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "no-session", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("No session")).toBeTruthy();
    });
  });

  it("shows 'AI not configured' label", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "ai-not-configured", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("AI not configured")).toBeTruthy();
    });
  });

  it("shows 'Disabled' label", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "disabled", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("Disabled")).toBeTruthy();
    });
  });

  it("shows 'Error' label for error status", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "error", generating: false, error: "Something went wrong" };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("Error")).toBeTruthy();
      expect(rendered.getByText("Something went wrong")).toBeTruthy();
    });
  });

  it("shows session summary error from load failure", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        throw new Error("Backend down");
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText(/Failed to load summary/)).toBeTruthy();
    });
  });

  it("shows warning from session summary result", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "ok", generating: false, warning: "Stale data" };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("Stale data")).toBeTruthy();
    });
  });

  it("shows tool ID when present in session summary", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "ok", generating: false, toolId: "claude-code" };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("Latest tool: claude-code")).toBeTruthy();
    });
  });

  it("shows task sub-agent model name when present", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return {
          issueNumber: null,
          tasks: [
            {
              id: "T001",
              title: "Task with model",
              status: "running",
              subAgents: [
                {
                  id: "sess-1",
                  name: "Claude",
                  toolId: "claude-code",
                  status: "running",
                  model: "claude-opus-4",
                  branch: "feature/x",
                  worktreeRelPath: ".worktrees/feature-x",
                  worktreeAbsPath: null,
                },
              ],
            },
          ],
        };
      }
      if (command === "get_branch_session_summary") {
        return { status: "no-session", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("claude-opus-4")).toBeTruthy();
    });
  });

  it("uses selectedBranch when available over currentBranch", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "no-session", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
      selectedBranch: {
        name: "feature/selected",
        commit: "abc",
        is_current: false,
        is_agent_running: false,
        agent_status: "unknown",
        ahead: 0,
        behind: 0,
        divergence_status: "UpToDate",
      },
    });

    await waitFor(() => {
      expect(rendered.getByText("feature/selected")).toBeTruthy();
    });
  });

  it("shows 'No branch context.' in summary section when no branch", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "no-session", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "",
      selectedBranch: null,
    });

    await waitFor(() => {
      expect(rendered.getByText("No branch context.")).toBeTruthy();
    });
  });

  it("shows 'No summary yet.' for no-session status in summary section", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "no-session", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("No summary yet.")).toBeTruthy();
    });
  });

  it("shows 'Latest summary ready.' for ok status without toolId or warning", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return { issueNumber: null, tasks: [] };
      }
      if (command === "get_branch_session_summary") {
        return { status: "ok", generating: false };
      }
      return null;
    });

    const rendered = await renderSidebar({
      projectPath: "/repo",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("Latest summary ready.")).toBeTruthy();
    });
  });
});
