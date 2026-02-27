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

  it("renders pending status color correctly", async () => {
    // Exercise the pending branch in statusColor (line 15)
    const issue = makeIssue({
      status: "pending",
      tasks: [
        {
          id: "task-pending",
          name: "Pending task",
          status: "pending" as any,
          developers: [],
          retryCount: 0,
        },
      ],
    });
    const rendered = await renderIssueItem({ issue });

    const badge = rendered.container.querySelector('[data-testid="issue-detail-status"]');
    expect(badge?.getAttribute("data-status")).toBe("pending");
    expect(rendered.getByText("Pending task")).toBeTruthy();
  });

  it("applies statusColor for all switch branches via task statuses", async () => {
    // Exercise ci_fail, cancelled, not_run, and default (lines 30-37)
    const allStatuses = [
      "pending",
      "planned",
      "ready",
      "in_progress",
      "running",
      "starting",
      "restarting",
      "completed",
      "passed",
      "failed",
      "error",
      "crashed",
      "ci_fail",
      "cancelled",
      "not_run",
      "unknown_xyz",
    ];

    const tasks = allStatuses.map((status, i) => ({
      id: `task-${status}`,
      name: `Task ${status}`,
      status: status as any,
      developers: [] as DeveloperState[],
      retryCount: 0,
    }));

    const issue = makeIssue({
      status: "in_progress",
      tasks,
    });
    const rendered = await renderIssueItem({ issue });

    for (const status of allStatuses) {
      expect(rendered.getByText(`Task ${status}`)).toBeTruthy();
    }
  });

  it("renders task with pullRequest but no ciStatus", async () => {
    // Exercise the {#if task.pullRequest} branch without ciStatus
    const issue = makeIssue({
      tasks: [
        {
          id: "task-pr-no-ci",
          name: "PR no CI",
          status: "running",
          developers: [],
          retryCount: 0,
          pullRequest: {
            number: 10,
            url: "https://github.com/org/repo/pull/10",
          },
        },
      ],
    });
    const rendered = await renderIssueItem({ issue });

    expect(rendered.getByText("PR #10")).toBeTruthy();
    // No CI badge should appear
    const ciBadge = rendered.container.querySelector('[data-testid="ci-status-task-pr-no-ci"]');
    expect(ciBadge).toBeNull();
  });

  it("renders task with no pullRequest and no retry", async () => {
    // Exercise retryCount === 0 (no retry badge) and no pullRequest
    const issue = makeIssue({
      tasks: [
        {
          id: "task-clean",
          name: "Clean task",
          status: "completed",
          developers: [],
          retryCount: 0,
        },
      ],
    });
    const rendered = await renderIssueItem({ issue });

    expect(rendered.getByText("Clean task")).toBeTruthy();
    expect(rendered.queryByText(/retries/)).toBeNull();
    expect(rendered.queryByText(/PR #/)).toBeNull();
  });

  it("renders tasks without developers (empty developer list)", async () => {
    // Exercise the {#if task.developers.length > 0} false branch
    const issue = makeIssue({
      tasks: [
        {
          id: "task-no-devs",
          name: "Solo task",
          status: "pending" as any,
          developers: [],
          retryCount: 0,
        },
      ],
    });
    const rendered = await renderIssueItem({ issue });

    expect(rendered.getByText("Solo task")).toBeTruthy();
    expect(rendered.container.querySelectorAll(".developer-item").length).toBe(0);
  });

  it("renders without onViewTerminal callback", async () => {
    const issue = makeIssue({
      coordinator: makeCoordinator(),
    });
    const rendered = await renderIssueItem({ issue });

    // Click view terminal without callback - should not throw
    const viewBtn = rendered.container.querySelector('[data-testid="view-terminal-btn"]');
    expect(viewBtn).toBeTruthy();
    await fireEvent.click(viewBtn!);
  });

  it("exercises statusPulse for non-pulsing statuses", async () => {
    const issue = makeIssue({
      status: "completed",
      tasks: [
        {
          id: "task-nopulse",
          name: "No pulse",
          status: "completed",
          developers: [],
          retryCount: 0,
        },
      ],
    });
    const rendered = await renderIssueItem({ issue });

    const statusDots = rendered.container.querySelectorAll(".status-dot");
    for (const dot of statusDots) {
      expect(dot.classList.contains("pulse")).toBe(false);
    }
  });

  it("exercises statusPulse for starting/restarting statuses", async () => {
    const issue = makeIssue({
      status: "in_progress",
      tasks: [
        {
          id: "task-pulse-start",
          name: "Starting task",
          status: "running",
          developers: [],
          retryCount: 0,
        },
      ],
    });
    const rendered = await renderIssueItem({ issue });

    // The issue status dot for in_progress should have pulse
    const badge = rendered.container.querySelector('[data-testid="issue-detail-status"]');
    expect(badge?.classList.contains("pulse")).toBe(true);
  });

  it("handles coordinator with various statuses", async () => {
    // Test different coordinator statuses to exercise statusColor branches via coordinator
    for (const status of ["starting", "completed", "crashed"] as const) {
      cleanup();
      const issue = makeIssue({
        coordinator: makeCoordinator({ status }),
      });
      const rendered = await renderIssueItem({ issue });

      expect(rendered.getByText(status)).toBeTruthy();
    }
  });

  it("re-renders with different task configs to exercise update branches", async () => {
    const { default: IssueItem } = await import("./IssueItem.svelte");
    const dev = makeDeveloper({ id: "dev-rr" });
    const rendered = render(IssueItem, {
      props: {
        issue: makeIssue({
          status: "in_progress",
          coordinator: makeCoordinator({ status: "running" }),
          tasks: [
            {
              id: "task-rr",
              name: "Running task",
              status: "running",
              developers: [dev],
              retryCount: 2,
              pullRequest: { number: 5, url: "https://example.com/pr/5", ciStatus: "passed" },
            },
          ],
        }),
      },
    });

    expect(rendered.getByText("Running task")).toBeTruthy();
    expect(rendered.getByText("PR #5")).toBeTruthy();
    expect(rendered.getByText("2 retries")).toBeTruthy();

    // Re-render with no tasks, no coordinator - exercises template conditional update paths
    await rendered.rerender({
      issue: makeIssue({
        status: "completed",
        coordinator: undefined,
        tasks: [],
      }),
    });

    expect(rendered.getByText("No tasks")).toBeTruthy();
    expect(rendered.queryByText("Coordinator")).toBeNull();
  });

  it("unmounts with complex state to exercise teardown branches", async () => {
    const dev = makeDeveloper({ id: "dev-umount", status: "running" });
    const issue = makeIssue({
      status: "in_progress",
      coordinator: makeCoordinator({ status: "running" }),
      tasks: [
        {
          id: "task-umount",
          name: "Unmount task",
          status: "running",
          developers: [dev],
          retryCount: 1,
          pullRequest: { number: 99, url: "https://example.com/pr/99", ciStatus: "failed" },
        },
      ],
    });

    const rendered = await renderIssueItem({ issue });
    expect(rendered.getByText("Unmount task")).toBeTruthy();
    rendered.unmount();
  });
});
