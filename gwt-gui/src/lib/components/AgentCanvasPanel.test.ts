import { describe, expect, it } from "vitest";
import { fireEvent, render } from "@testing-library/svelte";
import AgentCanvasPanel from "./AgentCanvasPanel.svelte";
import type { Tab, WorktreeInfo } from "../types";

describe("AgentCanvasPanel", () => {
  it("opens worktree details from the worktree card", async () => {
    const tabs: Tab[] = [
      {
        id: "agent-1",
        label: "Agent One",
        type: "agent",
        branchName: "feature/canvas",
        worktreePath: "/tmp/project/.gwt/worktrees/feature-canvas",
      },
      {
        id: "terminal-1",
        label: "Shell",
        type: "terminal",
        branchName: "feature/canvas",
        worktreePath: "/tmp/project/.gwt/worktrees/feature-canvas",
      },
    ];
    const worktrees: WorktreeInfo[] = [
      {
        path: "/tmp/project/.gwt/worktrees/feature-canvas",
        branch: "feature/canvas",
        commit: "abc123",
        status: "active",
        is_main: false,
        has_changes: false,
        has_unpushed: false,
        is_current: true,
        is_protected: false,
        is_agent_running: false,
        agent_status: "unknown",
        ahead: 0,
        behind: 0,
        is_gone: false,
        last_tool_usage: null,
        safety_level: "safe",
      },
    ];

    const rendered = render(AgentCanvasPanel, {
      props: {
        projectPath: "/tmp/project",
        currentBranch: "feature/canvas",
        tabs,
        worktrees,
      },
    });

    expect(rendered.queryByTestId("agent-canvas-worktree-dialog")).toBeNull();
    const worktreeCard = rendered.container.querySelector(
      '[data-testid^="agent-canvas-worktree-card-"]',
    ) as HTMLElement;
    expect(worktreeCard).toBeTruthy();
    await fireEvent.click(worktreeCard);

    const dialog = rendered.getByTestId("agent-canvas-worktree-dialog");
    expect(dialog.textContent).toContain("/tmp/project");
    expect(dialog.textContent).toContain("feature/canvas");
    expect(dialog.textContent).toContain("2");
    expect(rendered.getByTestId("agent-canvas-edge-session-agent-1")).toBeTruthy();
  });
});
