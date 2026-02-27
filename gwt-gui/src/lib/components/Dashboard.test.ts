import { describe, it, expect, beforeEach } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";
import type { DashboardIssue } from "../types";

async function renderDashboard(props: {
  issues: DashboardIssue[];
  onTaskClick?: (taskId: string) => void;
}) {
  const { default: Dashboard } = await import("./Dashboard.svelte");
  return render(Dashboard, { props });
}

function makeIssue(overrides: Partial<DashboardIssue> = {}): DashboardIssue {
  return {
    id: "issue-1",
    githubIssueNumber: 42,
    githubIssueUrl: "https://github.com/org/repo/issues/42",
    title: "Implement auth feature",
    status: "in_progress",
    tasks: [],
    expanded: false,
    taskCompletedCount: 0,
    taskTotalCount: 0,
    ...overrides,
  };
}

describe("Dashboard", () => {
  beforeEach(() => {
    cleanup();
  });

  it("shows empty state when no issues", async () => {
    const rendered = await renderDashboard({ issues: [] });
    expect(rendered.getByText("No issues yet")).toBeTruthy();
  });

  it("renders issue item with title, status badge, and task count", async () => {
    const issue = makeIssue({
      title: "Add login page",
      status: "in_progress",
      taskCompletedCount: 2,
      taskTotalCount: 5,
    });
    const rendered = await renderDashboard({ issues: [issue] });

    expect(rendered.getByText("Add login page")).toBeTruthy();
    expect(rendered.getByText("2/5 tasks")).toBeTruthy();

    const badge = rendered.container.querySelector(
      '[data-testid="issue-status-issue-1"]',
    );
    expect(badge).toBeTruthy();
    expect(badge?.getAttribute("data-status")).toBe("in_progress");
  });

  it("toggles issue expand/collapse", async () => {
    const issue = makeIssue({
      expanded: false,
      tasks: [
        {
          id: "task-1",
          name: "Write tests",
          status: "running",
          developers: [],
          retryCount: 0,
        },
      ],
      taskTotalCount: 1,
    });
    const rendered = await renderDashboard({ issues: [issue] });

    // Task should not be visible initially (collapsed)
    expect(rendered.queryByText("Write tests")).toBeNull();

    // Click the issue header button to expand
    const headerBtn = rendered.container.querySelector(
      '[data-testid="issue-row-issue-1"] .issue-header',
    ) as HTMLButtonElement;
    expect(headerBtn).toBeTruthy();
    await fireEvent.click(headerBtn);

    // Task should now be visible
    expect(rendered.getByText("Write tests")).toBeTruthy();

    // Click again to collapse
    await fireEvent.click(headerBtn);
    expect(rendered.queryByText("Write tests")).toBeNull();
  });

  it("renders task items within expanded issue", async () => {
    const issue = makeIssue({
      expanded: true,
      tasks: [
        {
          id: "task-1",
          name: "Write unit tests",
          status: "completed",
          developers: [],
          retryCount: 0,
        },
        {
          id: "task-2",
          name: "Write integration tests",
          status: "running",
          developers: [],
          retryCount: 0,
        },
      ],
      taskCompletedCount: 1,
      taskTotalCount: 2,
    });
    const rendered = await renderDashboard({ issues: [issue] });

    expect(rendered.getByText("Write unit tests")).toBeTruthy();
    expect(rendered.getByText("Write integration tests")).toBeTruthy();
  });

  it("renders developer items within task", async () => {
    const issue = makeIssue({
      expanded: true,
      tasks: [
        {
          id: "task-1",
          name: "Implement feature",
          status: "running",
          developers: [
            {
              id: "dev-1",
              agentType: "claude",
              paneId: "pane-1",
              status: "running",
              worktree: {
                branchName: "feature/auth-impl",
                path: "/repo/.worktrees/feature-auth-impl",
              },
            },
            {
              id: "dev-2",
              agentType: "codex",
              paneId: "pane-2",
              status: "completed",
              worktree: {
                branchName: "feature/auth-test",
                path: "/repo/.worktrees/feature-auth-test",
              },
            },
          ],
          retryCount: 0,
        },
      ],
      taskTotalCount: 1,
    });
    const rendered = await renderDashboard({ issues: [issue] });

    expect(rendered.getByText("feature/auth-impl")).toBeTruthy();
    expect(rendered.getByText("feature/auth-test")).toBeTruthy();
    expect(rendered.getByText("claude")).toBeTruthy();
    expect(rendered.getByText("codex")).toBeTruthy();
  });

  it("applies correct status badge colors via data-status attribute", async () => {
    const statusList: Array<{ id: string; status: DashboardIssue["status"] }> = [
      { id: "i-pending", status: "pending" },
      { id: "i-planned", status: "planned" },
      { id: "i-in-progress", status: "in_progress" },
      { id: "i-completed", status: "completed" },
      { id: "i-failed", status: "failed" },
      { id: "i-ci-fail", status: "ci_fail" },
    ];
    const issues: DashboardIssue[] = statusList.map(({ id, status }) =>
      makeIssue({ id, status, expanded: false }),
    );
    const rendered = await renderDashboard({ issues });

    for (const { id, status } of statusList) {
      const badge = rendered.container.querySelector(
        `[data-testid="issue-status-${id}"]`,
      );
      expect(badge).toBeTruthy();
      expect(badge?.getAttribute("data-status")).toBe(status);
    }
  });

  it("displays coordinator status when present", async () => {
    const issue = makeIssue({
      expanded: true,
      coordinator: {
        paneId: "coord-pane-1",
        status: "running",
      },
      tasks: [],
    });
    const rendered = await renderDashboard({ issues: [issue] });

    expect(rendered.getByText(/Coordinator:/)).toBeTruthy();
    expect(rendered.getByText(/running/)).toBeTruthy();
  });

  it("calls onTaskClick when a task is clicked", async () => {
    let clickedId: string | null = null;
    const issue = makeIssue({
      expanded: true,
      tasks: [
        {
          id: "task-click-1",
          name: "Clickable task",
          status: "pending",
          developers: [],
          retryCount: 0,
        },
      ],
      taskTotalCount: 1,
    });
    const rendered = await renderDashboard({
      issues: [issue],
      onTaskClick: (id: string) => {
        clickedId = id;
      },
    });

    const taskRow = rendered.getByTestId("task-row-task-click-1");
    await fireEvent.click(taskRow);
    expect(clickedId).toBe("task-click-1");
  });

  it("truncates long issue titles", async () => {
    const longTitle = "A".repeat(200);
    const issue = makeIssue({ title: longTitle });
    const rendered = await renderDashboard({ issues: [issue] });

    const titleEl = rendered.container.querySelector(".issue-title");
    expect(titleEl).toBeTruthy();
    // The CSS handles truncation; we just verify the text is rendered
    expect(titleEl?.textContent).toBe(longTitle);
  });

  // T509/T510: Dashboard→Branch Mode jump
  it("calls onTaskClick with correct id for task with developers (Branch Mode jump)", async () => {
    let clickedId: string | null = null;
    const issue = makeIssue({
      expanded: true,
      tasks: [
        {
          id: "task-branch-jump",
          name: "Implement login",
          status: "running",
          developers: [
            {
              id: "dev-1",
              agentType: "claude",
              paneId: "pane-1",
              status: "running",
              worktree: {
                branchName: "agent/implement-login",
                path: "/repo/.worktrees/agent-implement-login",
              },
            },
          ],
          retryCount: 0,
        },
      ],
      taskTotalCount: 1,
    });
    const rendered = await renderDashboard({
      issues: [issue],
      onTaskClick: (id: string) => {
        clickedId = id;
      },
    });

    const taskRow = rendered.getByTestId("task-row-task-branch-jump");
    await fireEvent.click(taskRow);
    expect(clickedId).toBe("task-branch-jump");
  });

  it("renders multiple issues in order", async () => {
    const issues: DashboardIssue[] = [
      makeIssue({ id: "i-1", title: "First issue" }),
      makeIssue({ id: "i-2", title: "Second issue" }),
      makeIssue({ id: "i-3", title: "Third issue" }),
    ];
    const rendered = await renderDashboard({ issues });

    const rows = Array.from(
      rendered.container.querySelectorAll("[data-testid^='issue-row-']"),
    ).map((el) => el.getAttribute("data-testid"));

    expect(rows).toEqual(["issue-row-i-1", "issue-row-i-2", "issue-row-i-3"]);
  });

  it("renders with default empty issues (default prop value)", async () => {
    // Render with minimal props to exercise the default parameter for issues = [] (line 5)
    const { default: Dashboard } = await import("./Dashboard.svelte");
    const rendered = render(Dashboard, {
      props: {} as any,
    });

    expect(rendered.getByText("No issues yet")).toBeTruthy();
  });

  it("applies statusColor for cancelled and default (unknown) statuses", async () => {
    // Exercise the cancelled and default branches in statusColor (lines 51-53)
    const issues: DashboardIssue[] = [
      makeIssue({
        id: "cancelled-issue",
        title: "Cancelled",
        status: "cancelled" as any,
        expanded: true,
        tasks: [
          {
            id: "task-cancelled",
            name: "Cancelled task",
            status: "cancelled",
            developers: [],
            retryCount: 0,
          },
        ],
        taskTotalCount: 1,
      }),
      makeIssue({
        id: "unknown-issue",
        title: "Unknown status",
        status: "unknown_xyz" as any,
        expanded: false,
      }),
    ];
    const rendered = await renderDashboard({ issues });

    expect(rendered.getByText("Cancelled")).toBeTruthy();
    expect(rendered.getByText("Unknown status")).toBeTruthy();
  });

  it("renders all statusColor switch branches via issues", async () => {
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
      "totally_unknown",
    ];

    const issues: DashboardIssue[] = allStatuses.map((status, i) =>
      makeIssue({
        id: `status-${status}`,
        title: `Issue ${status}`,
        status: status as any,
        expanded: false,
        taskCompletedCount: 0,
        taskTotalCount: 0,
      }),
    );

    const rendered = await renderDashboard({ issues });

    for (const status of allStatuses) {
      expect(rendered.getByText(`Issue ${status}`)).toBeTruthy();
    }
  });

  it("exercises toggleExpand with issue that has explicit expanded override", async () => {
    const issue = makeIssue({
      id: "toggle-test",
      expanded: true,
      tasks: [
        {
          id: "task-toggle",
          name: "Toggle task",
          status: "running",
          developers: [],
          retryCount: 0,
        },
      ],
      taskTotalCount: 1,
    });
    const rendered = await renderDashboard({ issues: [issue] });

    // Initially expanded
    expect(rendered.getByText("Toggle task")).toBeTruthy();

    // Toggle collapse
    const headerBtn = rendered.container.querySelector(
      '[data-testid="issue-row-toggle-test"] .issue-header',
    ) as HTMLButtonElement;
    await fireEvent.click(headerBtn);

    // Should be collapsed
    expect(rendered.queryByText("Toggle task")).toBeNull();

    // Toggle expand again
    await fireEvent.click(headerBtn);

    // Should be expanded again
    expect(rendered.getByText("Toggle task")).toBeTruthy();
  });

  it("renders expanded issue with no tasks", async () => {
    const issue = makeIssue({
      expanded: true,
      tasks: [],
      taskTotalCount: 0,
    });
    const rendered = await renderDashboard({ issues: [issue] });

    expect(rendered.getByText("No tasks")).toBeTruthy();
  });

  it("renders expanded issue without coordinator", async () => {
    const issue = makeIssue({
      expanded: true,
      coordinator: undefined,
      tasks: [
        {
          id: "task-no-coord",
          name: "No coordinator task",
          status: "running",
          developers: [],
          retryCount: 0,
        },
      ],
      taskTotalCount: 1,
    });
    const rendered = await renderDashboard({ issues: [issue] });

    expect(rendered.getByText("No coordinator task")).toBeTruthy();
    expect(rendered.queryByText(/Coordinator:/)).toBeNull();
  });

  it("renders tasks with no developers (no dev-count badge)", async () => {
    const issue = makeIssue({
      expanded: true,
      tasks: [
        {
          id: "task-no-devs",
          name: "Solo task",
          status: "pending",
          developers: [],
          retryCount: 0,
        },
      ],
      taskTotalCount: 1,
    });
    const rendered = await renderDashboard({ issues: [issue] });

    expect(rendered.getByText("Solo task")).toBeTruthy();
    // No dev-count badge
    expect(rendered.container.querySelectorAll(".dev-count").length).toBe(0);
  });

  it("exercises statusPulse for non-pulsing and pulsing statuses", async () => {
    const issues: DashboardIssue[] = [
      makeIssue({
        id: "pulse-test",
        status: "in_progress",
        expanded: true,
        tasks: [
          {
            id: "task-pulse",
            name: "Pulsing",
            status: "running",
            developers: [],
            retryCount: 0,
          },
          {
            id: "task-nopulse",
            name: "Not pulsing",
            status: "completed",
            developers: [],
            retryCount: 0,
          },
        ],
        taskTotalCount: 2,
      }),
    ];
    const rendered = await renderDashboard({ issues });

    const issueDot = rendered.container.querySelector('[data-testid="issue-status-pulse-test"]');
    expect(issueDot?.classList.contains("pulse")).toBe(true);

    const taskDotPulse = rendered.container.querySelector('[data-testid="task-status-task-pulse"]');
    expect(taskDotPulse?.classList.contains("pulse")).toBe(true);

    const taskDotNoPulse = rendered.container.querySelector('[data-testid="task-status-task-nopulse"]');
    expect(taskDotNoPulse?.classList.contains("pulse")).toBe(false);
  });

  it("calls onTaskClick without callback (undefined)", async () => {
    const issue = makeIssue({
      expanded: true,
      tasks: [
        {
          id: "task-nocb",
          name: "No callback task",
          status: "pending",
          developers: [],
          retryCount: 0,
        },
      ],
      taskTotalCount: 1,
    });
    const rendered = await renderDashboard({ issues: [issue] });

    const taskRow = rendered.getByTestId("task-row-task-nocb");
    // Clicking without callback should not throw
    await fireEvent.click(taskRow);
  });

  it("re-renders with different issue configs to exercise update branches", async () => {
    const { default: Dashboard } = await import("./Dashboard.svelte");
    const rendered = render(Dashboard, {
      props: {
        issues: [
          makeIssue({
            id: "rr-issue",
            expanded: true,
            coordinator: { paneId: "c1", status: "running" },
            tasks: [
              {
                id: "rr-task",
                name: "RR task",
                status: "running",
                developers: [
                  {
                    id: "dev-rr",
                    agentType: "claude",
                    paneId: "p1",
                    status: "running",
                    worktree: { branchName: "feat/rr", path: "/tmp" },
                  },
                ],
                retryCount: 0,
              },
            ],
            taskTotalCount: 1,
          }),
        ],
      },
    });

    expect(rendered.getByText("RR task")).toBeTruthy();

    // Re-render to empty
    await rendered.rerender({ issues: [] });
    expect(rendered.getByText("No issues yet")).toBeTruthy();
  });
});
