import { describe, it, expect, beforeEach } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";
import type { ProjectIssue, CoordinatorState, DeveloperState } from "../types";

async function renderIssueItem(props: {
  issue: ProjectIssue;
  onViewTerminal?: (paneId: string) => void;
}) {
  const { default: IssueItem } = await import("./IssueItem.svelte");
  return render(IssueItem, { props });
}

function makeIssue(overrides: Partial<ProjectIssue> = {}): ProjectIssue {
  return {
    id: "issue-1",
    githubIssueNumber: 42,
    githubIssueUrl: "https://github.com/org/repo/issues/42",
    title: "Implement auth feature",
    status: "in_progress",
    tasks: [],
    ...overrides,
  };
}

function makeCoordinator(
  overrides: Partial<CoordinatorState> = {},
): CoordinatorState {
  return {
    paneId: "coord-pane-1",
    status: "running",
    ...overrides,
  };
}

function makeDeveloper(
  overrides: Partial<DeveloperState> = {},
): DeveloperState {
  return {
    id: "dev-1",
    agentType: "claude",
    paneId: "dev-pane-1",
    status: "running",
    worktree: {
      branchName: "feature/auth-impl",
      path: "/repo/.worktrees/feature-auth-impl",
    },
    ...overrides,
  };
}

describe("IssueItem", () => {
  beforeEach(() => {
    cleanup();
  });

  it("renders issue title and status", async () => {
    const issue = makeIssue({ title: "Login feature", status: "planned" });
    const rendered = await renderIssueItem({ issue });

    expect(rendered.getByText("Login feature")).toBeTruthy();
    const badge = rendered.container.querySelector(
      '[data-testid="issue-detail-status"]',
    );
    expect(badge).toBeTruthy();
    expect(badge?.getAttribute("data-status")).toBe("planned");
  });

  it("renders GitHub issue number as link", async () => {
    const issue = makeIssue({
      githubIssueNumber: 99,
      githubIssueUrl: "https://github.com/org/repo/issues/99",
    });
    const rendered = await renderIssueItem({ issue });

    expect(rendered.getByText("#99")).toBeTruthy();
  });

  it("shows coordinator status when present", async () => {
    const issue = makeIssue({
      coordinator: makeCoordinator({ status: "running" }),
    });
    const rendered = await renderIssueItem({ issue });

    expect(rendered.getByText(/Coordinator/)).toBeTruthy();
    expect(rendered.getByText("running")).toBeTruthy();
  });

  it("shows no coordinator section when absent", async () => {
    const issue = makeIssue({ coordinator: undefined });
    const rendered = await renderIssueItem({ issue });

    expect(rendered.queryByText(/Coordinator/)).toBeNull();
  });

  it("renders developer list within tasks", async () => {
    const issue = makeIssue({
      tasks: [
        {
          id: "task-1",
          name: "Write tests",
          status: "running",
          developers: [
            makeDeveloper({
              id: "dev-1",
              agentType: "claude",
              worktree: {
                branchName: "feature/tests",
                path: "/repo/.worktrees/feature-tests",
              },
            }),
            makeDeveloper({
              id: "dev-2",
              agentType: "codex",
              status: "completed",
              worktree: {
                branchName: "feature/impl",
                path: "/repo/.worktrees/feature-impl",
              },
            }),
          ],
          retryCount: 0,
        },
      ],
    });
    const rendered = await renderIssueItem({ issue });

    expect(rendered.getByText("feature/tests")).toBeTruthy();
    expect(rendered.getByText("feature/impl")).toBeTruthy();
    expect(rendered.getByText("claude")).toBeTruthy();
    expect(rendered.getByText("codex")).toBeTruthy();
  });

  it("renders CI status badge when task has pullRequest with ciStatus", async () => {
    const issue = makeIssue({
      tasks: [
        {
          id: "task-ci",
          name: "Task with CI",
          status: "completed",
          developers: [],
          retryCount: 0,
          pullRequest: {
            number: 55,
            url: "https://github.com/org/repo/pull/55",
            ciStatus: "passed",
          },
        },
      ],
    });
    const rendered = await renderIssueItem({ issue });

    expect(rendered.getByText("PR #55")).toBeTruthy();
    const ciBadge = rendered.container.querySelector(
      '[data-testid="ci-status-task-ci"]',
    );
    expect(ciBadge).toBeTruthy();
    expect(ciBadge?.textContent).toContain("passed");
  });

  it("calls onViewTerminal when View Terminal button is clicked", async () => {
    let clickedPaneId: string | null = null;
    const issue = makeIssue({
      coordinator: makeCoordinator({ paneId: "coord-42" }),
    });
    const rendered = await renderIssueItem({
      issue,
      onViewTerminal: (paneId: string) => {
        clickedPaneId = paneId;
      },
    });

    const viewBtn = rendered.container.querySelector(
      '[data-testid="view-terminal-btn"]',
    );
    expect(viewBtn).toBeTruthy();
    await fireEvent.click(viewBtn!);
    expect(clickedPaneId).toBe("coord-42");
  });

  it("shows empty task state when no tasks", async () => {
    const issue = makeIssue({ tasks: [] });
    const rendered = await renderIssueItem({ issue });

    expect(rendered.getByText("No tasks")).toBeTruthy();
  });

  it("renders task retry count when > 0", async () => {
    const issue = makeIssue({
      tasks: [
        {
          id: "task-retry",
          name: "Flaky task",
          status: "running",
          developers: [],
          retryCount: 3,
        },
      ],
    });
    const rendered = await renderIssueItem({ issue });

    expect(rendered.getByText(/3 retries/)).toBeTruthy();
  });
});
