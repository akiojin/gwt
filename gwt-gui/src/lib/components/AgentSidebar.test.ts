import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";
import type { BranchInfo } from "../types";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

type AgentSidebarProps = {
  projectPath: string;
  currentBranch?: string;
  selectedBranch?: BranchInfo | null;
};

async function renderSidebar(props: AgentSidebarProps) {
  const { default: AgentSidebar } = await import("./AgentSidebar.svelte");
  return render(AgentSidebar, { props });
}

describe("AgentSidebar", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
  });

  it("switches assigned agents when selecting a task", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_agent_sidebar_view") {
        return {
          spec_id: "SPEC-ba3f610c",
          tasks: [
            {
              id: "T001",
              title: "T001 [US1] implement auth",
              status: "running",
              sub_agents: [
                {
                  id: "sess-codex",
                  name: "Codex",
                  tool_id: "codex-cli",
                  status: "running",
                  model: "gpt-5-codex",
                  branch: "feature/auth",
                  worktree_rel_path: ".worktrees/feature-auth",
                  worktree_abs_path: "/repo/.worktrees/feature-auth",
                },
              ],
            },
            {
              id: "T002",
              title: "T002 [US2] add tests",
              status: "pending",
              sub_agents: [
                {
                  id: "sess-claude",
                  name: "Claude",
                  tool_id: "claude-code",
                  status: "completed",
                  model: "opus",
                  branch: "feature/auth-tests",
                  worktree_rel_path: ".worktrees/feature-auth-tests",
                  worktree_abs_path: "/repo/.worktrees/feature-auth-tests",
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
        return { spec_id: null, tasks: [] };
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
          spec_id: "SPEC-order0001",
          tasks: [
            { id: "T004", title: "failed task", status: "failed", sub_agents: [] },
            { id: "T001", title: "completed task", status: "completed", sub_agents: [] },
            { id: "T003", title: "running task", status: "running", sub_agents: [] },
            { id: "T002", title: "pending task", status: "pending", sub_agents: [] },
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
          spec_id: "SPEC-multi0001",
          tasks: [
            {
              id: "T010",
              title: "T010 multi-assignee task",
              status: "running",
              sub_agents: [
                {
                  id: "sess-codex",
                  name: "Codex",
                  tool_id: "codex-cli",
                  status: "running",
                  model: "gpt-5-codex",
                  branch: "feature/a",
                  worktree_rel_path: ".worktrees/feature-a",
                  worktree_abs_path: "/repo/.worktrees/feature-a",
                },
                {
                  id: "sess-claude",
                  name: "Claude",
                  tool_id: "claude-code",
                  status: "completed",
                  model: "opus",
                  branch: "feature/b",
                  worktree_rel_path: ".worktrees/feature-b",
                  worktree_abs_path: "/repo/.worktrees/feature-b",
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
});
