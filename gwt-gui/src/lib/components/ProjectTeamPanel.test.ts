import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, cleanup, fireEvent } from "@testing-library/svelte";
import type { ProjectTeamState, Tab } from "../types";

const baseSession: ProjectTeamState = {
  sessionId: "session-001",
  status: "active",
  lead: {
    messages: [],
    status: "idle",
    llmCallCount: 0,
    estimatedTokens: 0,
  },
  issues: [],
  developerAgentType: "claude",
};

async function renderPanel(
  session: ProjectTeamState | null = null,
  opts: {
    aiReady?: boolean;
    onOpenSettings?: () => void;
    onOpenIssues?: () => void;
    agentBranches?: string[];
  } = {},
) {
  const { default: ProjectTeamPanel } = await import(
    "./ProjectTeamPanel.svelte"
  );
  return render(ProjectTeamPanel, {
    props: {
      session,
      aiReady: opts.aiReady ?? true,
      onOpenSettings: opts.onOpenSettings ?? (() => {}),
      onOpenIssues: opts.onOpenIssues ?? (() => {}),
      agentBranches: opts.agentBranches ?? [],
    },
  });
}

describe("ProjectTeamPanel", () => {
  beforeEach(() => {
    cleanup();
  });

  it("renders 2-column layout with dashboard and chat areas", async () => {
    const rendered = await renderPanel(baseSession);

    const dashboard = rendered.container.querySelector(
      ".project-team-dashboard",
    );
    const chat = rendered.container.querySelector(".project-team-chat");

    expect(dashboard).toBeTruthy();
    expect(chat).toBeTruthy();
  });

  it("shows 'No active session' when session is null", async () => {
    const rendered = await renderPanel(null);

    expect(rendered.getByText("No active session")).toBeTruthy();

    const dashboard = rendered.container.querySelector(
      ".project-team-dashboard",
    );
    expect(dashboard).toBeNull();
  });

  it("displays session status in the header", async () => {
    const rendered = await renderPanel(baseSession);

    expect(rendered.getByText("active")).toBeTruthy();
  });

  it("displays developer agent type in the header", async () => {
    const rendered = await renderPanel({
      ...baseSession,
      developerAgentType: "gemini",
    });

    expect(rendered.getByText("gemini")).toBeTruthy();
  });

  it("shows dashboard placeholder content", async () => {
    const rendered = await renderPanel(baseSession);

    expect(rendered.getByText("Dashboard")).toBeTruthy();
  });

  it("shows chat placeholder content", async () => {
    const rendered = await renderPanel(baseSession);

    expect(rendered.getByText("Lead Chat")).toBeTruthy();
  });

  it("renders paused status correctly", async () => {
    const rendered = await renderPanel({
      ...baseSession,
      status: "paused",
    });

    expect(rendered.getByText("paused")).toBeTruthy();
  });

  it("applies grid layout to the main content area", async () => {
    const rendered = await renderPanel(baseSession);

    const content = rendered.container.querySelector(".project-team-content");
    expect(content).toBeTruthy();
  });

  it("renders the header bar", async () => {
    const rendered = await renderPanel(baseSession);

    const header = rendered.container.querySelector(".project-team-header");
    expect(header).toBeTruthy();
  });

  it("shows issue count when issues exist", async () => {
    const rendered = await renderPanel({
      ...baseSession,
      issues: [
        {
          id: "issue-1",
          githubIssueNumber: 42,
          githubIssueUrl: "https://github.com/test/repo/issues/42",
          title: "Fix login bug",
          status: "in_progress",
          tasks: [],
        },
      ],
    });

    expect(rendered.getByText("1 issue")).toBeTruthy();
  });

  it("shows plural issue count for multiple issues", async () => {
    const rendered = await renderPanel({
      ...baseSession,
      issues: [
        {
          id: "issue-1",
          githubIssueNumber: 42,
          githubIssueUrl: "https://github.com/test/repo/issues/42",
          title: "Fix login bug",
          status: "in_progress",
          tasks: [],
        },
        {
          id: "issue-2",
          githubIssueNumber: 43,
          githubIssueUrl: "https://github.com/test/repo/issues/43",
          title: "Add tests",
          status: "pending",
          tasks: [],
        },
      ],
    });

    expect(rendered.getByText("2 issues")).toBeTruthy();
  });
});

// -- T307: AI not configured error display tests ----------------------------

describe("ProjectTeamPanel AI error state", () => {
  beforeEach(() => {
    cleanup();
  });

  it("shows error message when AI is not configured", async () => {
    const rendered = await renderPanel(null, { aiReady: false });

    expect(
      rendered.getByText("AI provider is not configured"),
    ).toBeTruthy();
  });

  it("shows a Settings button when AI is not configured", async () => {
    const onOpenSettings = vi.fn();
    const rendered = await renderPanel(null, {
      aiReady: false,
      onOpenSettings,
    });

    const btn = rendered.getByText("Settings");
    expect(btn).toBeTruthy();
    await fireEvent.click(btn);
    expect(onOpenSettings).toHaveBeenCalledOnce();
  });

  it("does not show AI error when aiReady is true", async () => {
    const rendered = await renderPanel(null, { aiReady: true });

    expect(rendered.getByText("No active session")).toBeTruthy();
    expect(
      rendered.container.querySelector(".ai-error"),
    ).toBeNull();
  });

  it("shows AI error instead of session content when not configured", async () => {
    const rendered = await renderPanel(baseSession, { aiReady: false });

    expect(
      rendered.getByText("AI provider is not configured"),
    ).toBeTruthy();
    // Should not render dashboard/chat when AI not configured
    expect(
      rendered.container.querySelector(".project-team-dashboard"),
    ).toBeNull();
  });
});

// -- T309: Cost visualization tests -----------------------------------------

describe("ProjectTeamPanel cost display", () => {
  beforeEach(() => {
    cleanup();
  });

  it("displays API call count in the header", async () => {
    const rendered = await renderPanel({
      ...baseSession,
      lead: { ...baseSession.lead, llmCallCount: 42, estimatedTokens: 0 },
    });

    expect(rendered.getByText("API Calls: 42")).toBeTruthy();
  });

  it("displays estimated tokens in the header", async () => {
    const rendered = await renderPanel({
      ...baseSession,
      lead: {
        ...baseSession.lead,
        llmCallCount: 0,
        estimatedTokens: 150000,
      },
    });

    expect(rendered.getByText("Tokens: ~150K")).toBeTruthy();
  });

  it("formats tokens below 1000 without K suffix", async () => {
    const rendered = await renderPanel({
      ...baseSession,
      lead: {
        ...baseSession.lead,
        llmCallCount: 1,
        estimatedTokens: 500,
      },
    });

    expect(rendered.getByText("Tokens: ~500")).toBeTruthy();
  });

  it("formats tokens in millions with M suffix", async () => {
    const rendered = await renderPanel({
      ...baseSession,
      lead: {
        ...baseSession.lead,
        llmCallCount: 100,
        estimatedTokens: 2500000,
      },
    });

    expect(rendered.getByText("Tokens: ~2.5M")).toBeTruthy();
  });

  it("shows zero counts when no LLM calls made", async () => {
    const rendered = await renderPanel(baseSession);

    expect(rendered.getByText("API Calls: 0")).toBeTruthy();
    expect(rendered.getByText("Tokens: ~0")).toBeTruthy();
  });
});

// -- Tab integration tests (MainArea renders ProjectTeamPanel) ---------------

async function renderMainArea(props: {
  tabs: Tab[];
  activeTabId: string;
  onTabSelect?: (tabId: string) => void;
  onTabClose?: (tabId: string) => void;
  onTabReorder?: (
    dragTabId: string,
    overTabId: string,
    position: "before" | "after",
  ) => void;
}) {
  const { default: MainArea } = await import("./MainArea.svelte");
  return render(MainArea, {
    props: {
      projectPath: "/tmp/project",
      selectedBranch: null,
      onLaunchAgent: vi.fn(),
      onQuickLaunch: vi.fn(),
      onTabSelect: props.onTabSelect ?? vi.fn(),
      onTabClose: props.onTabClose ?? vi.fn(),
      onTabReorder: props.onTabReorder ?? vi.fn(),
      activeTabId: props.activeTabId,
      tabs: props.tabs,
    },
  });
}

describe("ProjectTeamPanel tab integration", () => {
  beforeEach(() => {
    cleanup();
  });

  it("renders ProjectTeamPanel when projectTeam tab is active", async () => {
    const tabs: Tab[] = [
      { id: "projectTeam", label: "Project Team", type: "projectTeam" },
    ];
    const rendered = await renderMainArea({ tabs, activeTabId: "projectTeam" });

    expect(rendered.getByText("No active session")).toBeTruthy();
  });

  it("keeps projectTeam tab pinned (no close button)", async () => {
    const tabs: Tab[] = [
      { id: "projectTeam", label: "Project Team", type: "projectTeam" },
    ];
    const rendered = await renderMainArea({ tabs, activeTabId: "projectTeam" });

    const tab = rendered.container.querySelector(".tab-bar .tab");
    expect(tab).toBeTruthy();
    expect(tab?.querySelector(".tab-close")).toBeNull();
  });

  it("switches between projectTeam and other tabs", async () => {
    const onTabSelect = vi.fn();
    const tabs: Tab[] = [
      { id: "projectTeam", label: "Project Team", type: "projectTeam" },
      { id: "settings", label: "Settings", type: "settings" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "projectTeam",
      onTabSelect,
    });

    expect(rendered.getByText("No active session")).toBeTruthy();

    const settingsTab = rendered.getByText("Settings").closest(".tab");
    await fireEvent.click(settingsTab as HTMLElement);
    expect(onTabSelect).toHaveBeenCalledWith("settings");
  });
});

// -- T1003/T1004: Direct terminal access tests --------------------------------

const sessionWithDeveloper: ProjectTeamState = {
  ...baseSession,
  issues: [
    {
      id: "issue-1",
      githubIssueNumber: 42,
      githubIssueUrl: "https://github.com/test/repo/issues/42",
      title: "Fix login bug",
      status: "in_progress",
      coordinator: {
        paneId: "coord-pane-1",
        status: "running",
      },
      tasks: [
        {
          id: "task-1",
          name: "Implement login",
          status: "running",
          developers: [
            {
              id: "dev-1",
              agentType: "claude",
              paneId: "dev-pane-1",
              status: "running",
              worktree: {
                branchName: "feature/login",
                path: "/repo/.worktrees/feature-login",
              },
            },
          ],
          retryCount: 0,
        },
      ],
    },
  ],
};

describe("ProjectTeamPanel direct terminal access", () => {
  beforeEach(() => {
    cleanup();
  });

  it("shows terminal embed area when viewTerminalPaneId is set", async () => {
    const { default: ProjectTeamPanel } = await import(
      "./ProjectTeamPanel.svelte"
    );
    const rendered = render(ProjectTeamPanel, {
      props: {
        session: sessionWithDeveloper,
        aiReady: true,
        onOpenSettings: () => {},
        viewTerminalPaneId: "dev-pane-1",
      },
    });

    const terminalEmbed = rendered.container.querySelector(
      '[data-testid="terminal-embed"]',
    );
    expect(terminalEmbed).toBeTruthy();
    expect(terminalEmbed?.getAttribute("data-pane-id")).toBe("dev-pane-1");
  });

  it("terminal embed area has correct pane-id attribute", async () => {
    const { default: ProjectTeamPanel } = await import(
      "./ProjectTeamPanel.svelte"
    );
    const rendered = render(ProjectTeamPanel, {
      props: {
        session: sessionWithDeveloper,
        aiReady: true,
        onOpenSettings: () => {},
        viewTerminalPaneId: "coord-pane-1",
      },
    });

    const terminalEmbed = rendered.container.querySelector(
      '[data-testid="terminal-embed"]',
    );
    expect(terminalEmbed).toBeTruthy();
    expect(terminalEmbed?.getAttribute("data-pane-id")).toBe("coord-pane-1");
  });

  it("Back to Dashboard button returns to normal view", async () => {
    const { default: ProjectTeamPanel } = await import(
      "./ProjectTeamPanel.svelte"
    );
    const rendered = render(ProjectTeamPanel, {
      props: {
        session: sessionWithDeveloper,
        aiReady: true,
        onOpenSettings: () => {},
        viewTerminalPaneId: "dev-pane-1",
        onBackToDashboard: vi.fn(),
      },
    });

    const backBtn = rendered.container.querySelector(
      '[data-testid="back-to-dashboard"]',
    );
    expect(backBtn).toBeTruthy();
  });

  it("calls onBackToDashboard when Back to Dashboard is clicked", async () => {
    const onBackToDashboard = vi.fn();
    const { default: ProjectTeamPanel } = await import(
      "./ProjectTeamPanel.svelte"
    );
    const rendered = render(ProjectTeamPanel, {
      props: {
        session: sessionWithDeveloper,
        aiReady: true,
        onOpenSettings: () => {},
        viewTerminalPaneId: "dev-pane-1",
        onBackToDashboard,
      },
    });

    const backBtn = rendered.container.querySelector(
      '[data-testid="back-to-dashboard"]',
    );
    expect(backBtn).toBeTruthy();
    await fireEvent.click(backBtn!);
    expect(onBackToDashboard).toHaveBeenCalledOnce();
  });

  it("does not show terminal embed when viewTerminalPaneId is not set", async () => {
    const rendered = await renderPanel(sessionWithDeveloper);

    const terminalEmbed = rendered.container.querySelector(
      '[data-testid="terminal-embed"]',
    );
    expect(terminalEmbed).toBeNull();
  });

  it("shows dashboard content when viewTerminalPaneId is not set", async () => {
    const rendered = await renderPanel(sessionWithDeveloper);

    const dashboard = rendered.container.querySelector(
      ".project-team-dashboard",
    );
    expect(dashboard).toBeTruthy();
  });

  it("hides dashboard content when terminal view is active", async () => {
    const { default: ProjectTeamPanel } = await import(
      "./ProjectTeamPanel.svelte"
    );
    const rendered = render(ProjectTeamPanel, {
      props: {
        session: sessionWithDeveloper,
        aiReady: true,
        onOpenSettings: () => {},
        viewTerminalPaneId: "dev-pane-1",
      },
    });

    const dashboard = rendered.container.querySelector(
      ".project-team-dashboard",
    );
    expect(dashboard).toBeNull();
  });
});

// -- T1202: Agent branches section tests -------------------------------------

describe("ProjectTeamPanel agent branches", () => {
  beforeEach(() => {
    cleanup();
  });

  it("renders agent branches section in the dashboard area", async () => {
    const rendered = await renderPanel(baseSession, {
      agentBranches: ["agent/implement-login", "agent/fix-sidebar"],
    });

    const branchSection = rendered.container.querySelector(
      '[data-testid="agent-branches"]',
    );
    expect(branchSection).toBeTruthy();
  });

  it("displays agent branch names", async () => {
    const rendered = await renderPanel(baseSession, {
      agentBranches: ["agent/implement-login", "agent/fix-sidebar"],
    });

    expect(rendered.getByText("agent/implement-login")).toBeTruthy();
    expect(rendered.getByText("agent/fix-sidebar")).toBeTruthy();
  });

  it("shows agent badge for branches with agent/ prefix", async () => {
    const rendered = await renderPanel(baseSession, {
      agentBranches: ["agent/implement-login"],
    });

    const badge = rendered.container.querySelector(".agent-branch-badge");
    expect(badge).toBeTruthy();
  });

  it("does not render branches section when agentBranches is empty", async () => {
    const rendered = await renderPanel(baseSession, {
      agentBranches: [],
    });

    const branchSection = rendered.container.querySelector(
      '[data-testid="agent-branches"]',
    );
    expect(branchSection).toBeNull();
  });

  it("does not render branches section when no session", async () => {
    const rendered = await renderPanel(null, {
      agentBranches: ["agent/test"],
    });

    const branchSection = rendered.container.querySelector(
      '[data-testid="agent-branches"]',
    );
    expect(branchSection).toBeNull();
  });
});

// -- T1206: GitHub Issues button tests ---------------------------------------

describe("ProjectTeamPanel GitHub Issues button", () => {
  beforeEach(() => {
    cleanup();
  });

  it("renders GitHub Issues button in the header", async () => {
    const rendered = await renderPanel(baseSession);

    const btn = rendered.container.querySelector(
      '[data-testid="github-issues-btn"]',
    );
    expect(btn).toBeTruthy();
    expect(btn?.textContent).toContain("GitHub Issues");
  });

  it("calls onOpenIssues when GitHub Issues button is clicked", async () => {
    const onOpenIssues = vi.fn();
    const rendered = await renderPanel(baseSession, { onOpenIssues });

    const btn = rendered.container.querySelector(
      '[data-testid="github-issues-btn"]',
    );
    expect(btn).toBeTruthy();
    await fireEvent.click(btn!);
    expect(onOpenIssues).toHaveBeenCalledOnce();
  });

  it("does not show GitHub Issues button when no session", async () => {
    const rendered = await renderPanel(null);

    const btn = rendered.container.querySelector(
      '[data-testid="github-issues-btn"]',
    );
    expect(btn).toBeNull();
  });
});
