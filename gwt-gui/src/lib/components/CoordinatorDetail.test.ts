import { describe, it, expect, beforeEach } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";
import type {
  CoordinatorState,
  DeveloperState,
  ProjectTask,
} from "../types";

async function renderCoordinatorDetail(props: {
  coordinator: CoordinatorState;
  issueTitle: string;
  developers: DeveloperState[];
  tasks: ProjectTask[];
  onViewTerminal?: (paneId: string) => void;
}) {
  const { default: CoordinatorDetail } = await import(
    "./CoordinatorDetail.svelte"
  );
  return render(CoordinatorDetail, { props });
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

function makeTask(overrides: Partial<ProjectTask> = {}): ProjectTask {
  return {
    id: "task-1",
    name: "Implement feature",
    status: "running",
    developers: [],
    retryCount: 0,
    ...overrides,
  };
}

describe("CoordinatorDetail", () => {
  beforeEach(() => {
    cleanup();
  });

  it("renders coordinator status badge", async () => {
    const rendered = await renderCoordinatorDetail({
      coordinator: makeCoordinator({ status: "running" }),
      issueTitle: "Implement auth",
      developers: [],
      tasks: [],
    });

    const statusBadge = rendered.container.querySelector(
      '[data-testid="coordinator-status"]',
    );
    expect(statusBadge).toBeTruthy();
    expect(statusBadge?.getAttribute("data-status")).toBe("running");
  });

  it("renders issue title", async () => {
    const rendered = await renderCoordinatorDetail({
      coordinator: makeCoordinator(),
      issueTitle: "Implement auth feature",
      developers: [],
      tasks: [],
    });

    expect(rendered.getByText("Implement auth feature")).toBeTruthy();
  });

  it("renders developer list with status badges", async () => {
    const devs = [
      makeDeveloper({
        id: "dev-1",
        agentType: "claude",
        status: "running",
        paneId: "dev-pane-1",
        worktree: {
          branchName: "feature/login",
          path: "/repo/.worktrees/feature-login",
        },
      }),
      makeDeveloper({
        id: "dev-2",
        agentType: "codex",
        status: "completed",
        paneId: "dev-pane-2",
        worktree: {
          branchName: "feature/tests",
          path: "/repo/.worktrees/feature-tests",
        },
      }),
    ];

    const rendered = await renderCoordinatorDetail({
      coordinator: makeCoordinator(),
      issueTitle: "Auth feature",
      developers: devs,
      tasks: [],
    });

    expect(rendered.getByText("claude")).toBeTruthy();
    expect(rendered.getByText("codex")).toBeTruthy();
    expect(rendered.getByText("feature/login")).toBeTruthy();
    expect(rendered.getByText("feature/tests")).toBeTruthy();

    // Check developer status badges exist
    const devItems = rendered.container.querySelectorAll(
      '[data-testid^="developer-item-"]',
    );
    expect(devItems.length).toBe(2);
  });

  it("View Terminal button calls onViewTerminal with correct developer paneId", async () => {
    let clickedPaneId: string | null = null;
    const devs = [
      makeDeveloper({ id: "dev-1", paneId: "dev-pane-42" }),
    ];

    const rendered = await renderCoordinatorDetail({
      coordinator: makeCoordinator(),
      issueTitle: "Auth feature",
      developers: devs,
      tasks: [],
      onViewTerminal: (paneId: string) => {
        clickedPaneId = paneId;
      },
    });

    const viewBtn = rendered.container.querySelector(
      '[data-testid="view-terminal-dev-1"]',
    );
    expect(viewBtn).toBeTruthy();
    await fireEvent.click(viewBtn!);
    expect(clickedPaneId).toBe("dev-pane-42");
  });

  it("Coordinator terminal link calls onViewTerminal with coordinator paneId", async () => {
    let clickedPaneId: string | null = null;

    const rendered = await renderCoordinatorDetail({
      coordinator: makeCoordinator({ paneId: "coord-pane-99" }),
      issueTitle: "Auth feature",
      developers: [],
      tasks: [],
      onViewTerminal: (paneId: string) => {
        clickedPaneId = paneId;
      },
    });

    const coordTerminalBtn = rendered.container.querySelector(
      '[data-testid="view-terminal-coordinator"]',
    );
    expect(coordTerminalBtn).toBeTruthy();
    await fireEvent.click(coordTerminalBtn!);
    expect(clickedPaneId).toBe("coord-pane-99");
  });

  it("shows task assignments per developer", async () => {
    const dev = makeDeveloper({ id: "dev-1", paneId: "dev-pane-1" });
    const tasks = [
      makeTask({
        id: "task-1",
        name: "Write login tests",
        status: "running",
        developers: [dev],
      }),
    ];

    const rendered = await renderCoordinatorDetail({
      coordinator: makeCoordinator(),
      issueTitle: "Auth feature",
      developers: [dev],
      tasks,
    });

    // Task name appears in both the tasks section and dev-tasks section
    const matches = rendered.getAllByText("Write login tests");
    expect(matches.length).toBeGreaterThanOrEqual(1);
  });

  it("shows 'No developers assigned' when developer list is empty", async () => {
    const rendered = await renderCoordinatorDetail({
      coordinator: makeCoordinator(),
      issueTitle: "Empty issue",
      developers: [],
      tasks: [],
    });

    expect(rendered.getByText("No developers assigned")).toBeTruthy();
  });

  it("renders different coordinator statuses correctly", async () => {
    for (const status of [
      "starting",
      "running",
      "completed",
      "crashed",
      "restarting",
    ] as const) {
      cleanup();
      const rendered = await renderCoordinatorDetail({
        coordinator: makeCoordinator({ status }),
        issueTitle: "Test issue",
        developers: [],
        tasks: [],
      });

      const badge = rendered.container.querySelector(
        '[data-testid="coordinator-status"]',
      );
      expect(badge?.getAttribute("data-status")).toBe(status);
    }
  });

  it("renders multiple tasks with their assigned developers", async () => {
    const dev1 = makeDeveloper({
      id: "dev-1",
      agentType: "claude",
      paneId: "dev-pane-1",
    });
    const dev2 = makeDeveloper({
      id: "dev-2",
      agentType: "codex",
      paneId: "dev-pane-2",
    });

    const tasks = [
      makeTask({
        id: "task-1",
        name: "Implement login",
        developers: [dev1],
      }),
      makeTask({
        id: "task-2",
        name: "Write tests",
        developers: [dev2],
      }),
    ];

    const rendered = await renderCoordinatorDetail({
      coordinator: makeCoordinator(),
      issueTitle: "Auth feature",
      developers: [dev1, dev2],
      tasks,
    });

    // Task names appear in both the tasks section and dev-tasks section
    const loginMatches = rendered.getAllByText("Implement login");
    expect(loginMatches.length).toBeGreaterThanOrEqual(1);
    const testMatches = rendered.getAllByText("Write tests");
    expect(testMatches.length).toBeGreaterThanOrEqual(1);
  });
});
