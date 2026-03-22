import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/svelte";
import AgentCanvasPanel from "./AgentCanvasPanel.svelte";
import type { Tab, WorktreeInfo } from "../types";

const worktree: WorktreeInfo = {
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
};

describe("AgentCanvasPanel", () => {
  it("opens worktree details from the worktree card", async () => {
    const tabs: Tab[] = [
      {
        id: "agent-1",
        label: "Agent One",
        type: "agent",
        branchName: "feature/canvas",
        worktreePath: worktree.path,
      },
      {
        id: "terminal-1",
        label: "Shell",
        type: "terminal",
        branchName: "feature/canvas",
        worktreePath: worktree.path,
      },
    ];

    const rendered = render(AgentCanvasPanel, {
      props: {
        projectPath: "/tmp/project",
        currentBranch: "feature/canvas",
        tabs,
        worktrees: [worktree],
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

  it("updates zoom controls and drags cards on the board", async () => {
    const rendered = render(AgentCanvasPanel, {
      props: {
        projectPath: "/tmp/project",
        currentBranch: "feature/canvas",
        tabs: [],
        worktrees: [worktree],
      },
    });

    const zoomLabel = rendered.getByTestId("agent-canvas-zoom-label");
    expect(zoomLabel.textContent).toBe("100%");
    await fireEvent.click(rendered.getByLabelText("Zoom in"));
    expect(zoomLabel.textContent).toBe("110%");
    await fireEvent.click(zoomLabel.closest("button") as HTMLButtonElement);
    expect(zoomLabel.textContent).toBe("100%");

    const board = rendered.getByTestId("agent-canvas-board");
    const worktreeCard = rendered.container.querySelector(
      '[data-testid^="agent-canvas-worktree-card-"]',
    ) as HTMLElement;
    const dragHandle = worktreeCard.querySelector(".card-drag-handle") as HTMLElement;
    expect(worktreeCard.style.transform).toContain("translate(40px, 264px)");

    await fireEvent.pointerDown(dragHandle, {
      button: 0,
      pointerId: 7,
      clientX: 100,
      clientY: 100,
    });
    await fireEvent.pointerMove(board, {
      pointerId: 7,
      clientX: 180,
      clientY: 170,
    });
    await fireEvent.pointerUp(board, {
      pointerId: 7,
      clientX: 180,
      clientY: 170,
    });

    expect(worktreeCard.style.transform).toContain("translate(120px, 334px)");
  });

  it("emits persisted viewport and selected card changes", async () => {
    const onViewportChange = vi.fn();
    const onSelectedCardChange = vi.fn();
    const rendered = render(AgentCanvasPanel, {
      props: {
        projectPath: "/tmp/project",
        currentBranch: "feature/canvas",
        tabs: [],
        worktrees: [worktree],
        persistedViewport: { x: 12, y: 18, zoom: 1.2 },
        persistedSelectedCardId: `worktree:${worktree.path}`,
        onViewportChange,
        onSelectedCardChange,
      },
    });

    const zoomLabel = rendered.getByTestId("agent-canvas-zoom-label");
    expect(zoomLabel.textContent).toBe("120%");
    const worktreeCard = rendered.container.querySelector(
      '[data-testid^="agent-canvas-worktree-card-"]',
    ) as HTMLElement;
    expect(worktreeCard.className).toContain("selected");
    await fireEvent.click(rendered.getByLabelText("Zoom out"));
    expect(onViewportChange).toHaveBeenCalled();
    expect(onSelectedCardChange).toHaveBeenCalledWith(`worktree:${worktree.path}`);
  });

  it("keeps worktree-session edges visible after zoom and card drag", async () => {
    const rendered = render(AgentCanvasPanel, {
      props: {
        projectPath: "/tmp/project",
        currentBranch: "feature/canvas",
        tabs: [
          {
            id: "agent-1",
            label: "Agent One",
            type: "agent",
            paneId: "pane-1",
            branchName: "feature/canvas",
            worktreePath: worktree.path,
          },
        ],
        worktrees: [worktree],
      },
    });

    const edge = rendered.getByTestId("agent-canvas-edge-session-agent-1");
    expect(edge).toBeTruthy();

    await fireEvent.click(rendered.getByLabelText("Zoom in"));
    const board = rendered.getByTestId("agent-canvas-board");
    const worktreeCard = rendered.container.querySelector(
      '[data-testid^="agent-canvas-worktree-card-"]',
    ) as HTMLElement;
    const dragHandle = worktreeCard.querySelector(".card-drag-handle") as HTMLElement;

    await fireEvent.pointerDown(dragHandle, {
      button: 0,
      pointerId: 11,
      clientX: 120,
      clientY: 120,
    });
    await fireEvent.pointerMove(board, {
      pointerId: 11,
      clientX: 210,
      clientY: 180,
    });
    await fireEvent.pointerUp(board, {
      pointerId: 11,
      clientX: 210,
      clientY: 180,
    });

    expect(rendered.getByTestId("agent-canvas-edge-session-agent-1")).toBeTruthy();
  });
});
