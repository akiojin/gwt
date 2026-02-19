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

async function renderPanel(session: ProjectTeamState | null = null) {
  const { default: ProjectTeamPanel } = await import(
    "./ProjectTeamPanel.svelte"
  );
  return render(ProjectTeamPanel, { props: { session } });
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
