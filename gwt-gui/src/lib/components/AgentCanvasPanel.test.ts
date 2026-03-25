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
  it("opens assistant detail in an overlay instead of a persistent side pane", async () => {
    const rendered = render(AgentCanvasPanel, {
      props: {
        projectPath: "/tmp/project",
        currentBranch: "feature/canvas",
        tabs: [],
        worktrees: [worktree],
      },
    });

    expect(rendered.queryByTestId("agent-canvas-detail-overlay")).toBeNull();
    await fireEvent.click(rendered.getByTestId("agent-canvas-assistant-tile"));
    expect(rendered.getByTestId("agent-canvas-detail-overlay")).toBeTruthy();
    expect(rendered.getByTestId("agent-canvas-detail-dialog").textContent).toContain(
      "Assistant",
    );
  });

  it("opens worktree details from the worktree tile", async () => {
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
    const worktreeTile = rendered.container.querySelector(
      '[data-testid^="agent-canvas-worktree-tile-"]',
    ) as HTMLElement;
    expect(worktreeTile).toBeTruthy();
    await fireEvent.click(worktreeTile);

    const dialog = rendered.getByTestId("agent-canvas-worktree-dialog");
    expect(dialog.textContent).toContain("/tmp/project");
    expect(dialog.textContent).toContain("feature/canvas");
    expect(dialog.textContent).toContain("2");
    expect(rendered.getByTestId("agent-canvas-edge-session-agent-1")).toBeTruthy();
  });

  it("updates zoom controls and drags tiles on the board", async () => {
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
    const worktreeTile = rendered.container.querySelector(
      '[data-testid^="agent-canvas-worktree-tile-"]',
    ) as HTMLElement;
    const dragHandle = worktreeTile.querySelector(".tile-drag-handle") as HTMLElement;
    expect(worktreeTile.style.transform).toContain("translate(40px, 394px)");

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

    expect(worktreeTile.style.transform).toContain("translate(120px, 464px)");
  });

  it("emits persisted viewport and selected tile changes", async () => {
    const onViewportChange = vi.fn();
    const onSelectedTileChange = vi.fn();
    const rendered = render(AgentCanvasPanel, {
      props: {
        projectPath: "/tmp/project",
        currentBranch: "feature/canvas",
        tabs: [],
        worktrees: [worktree],
        persistedViewport: { x: 12, y: 18, zoom: 1.2 },
        persistedSelectedTileId: `worktree:${worktree.path}`,
        onViewportChange,
        onSelectedTileChange,
      },
    });

    const zoomLabel = rendered.getByTestId("agent-canvas-zoom-label");
    expect(zoomLabel.textContent).toBe("120%");
    const worktreeTile = rendered.container.querySelector(
      '[data-testid^="agent-canvas-worktree-tile-"]',
    ) as HTMLElement;
    expect(worktreeTile.className).toContain("selected");
    await fireEvent.click(rendered.getByLabelText("Zoom out"));
    expect(onViewportChange).toHaveBeenCalled();
    expect(onSelectedTileChange).toHaveBeenCalledWith(`worktree:${worktree.path}`);
  });

  it("keeps worktree-session edges visible after zoom and tile drag", async () => {
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
    const worktreeTile = rendered.container.querySelector(
      '[data-testid^="agent-canvas-worktree-tile-"]',
    ) as HTMLElement;
    const dragHandle = worktreeTile.querySelector(".tile-drag-handle") as HTMLElement;

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

  it("renders terminal content directly inside session tiles instead of using an overlay", async () => {
    const onSessionSelect = vi.fn();
    const rendered = render(AgentCanvasPanel, {
      props: {
        projectPath: "/tmp/project",
        currentBranch: "feature/canvas",
        tabs: [
          {
            id: "terminal-1",
            label: "Shell",
            type: "terminal",
            paneId: "pane-1",
            branchName: "feature/canvas",
            worktreePath: worktree.path,
          },
        ],
        worktrees: [worktree],
        onSessionSelect,
      },
    });

    const sessionTile = rendered.getByTestId("agent-canvas-session-terminal-1");
    expect(
      rendered.getByTestId("agent-canvas-session-surface-terminal-1"),
    ).toBeTruthy();
    expect(rendered.queryByTestId("agent-canvas-detail-overlay")).toBeNull();

    await fireEvent.click(sessionTile);

    expect(onSessionSelect).toHaveBeenCalledWith("terminal-1");
    expect(
      rendered.getByTestId("agent-canvas-session-surface-terminal-1"),
    ).toBeTruthy();
    expect(rendered.queryByTestId("agent-canvas-detail-overlay")).toBeNull();
  });
});
