import { describe, expect, it } from "vitest";
import { fireEvent, render } from "@testing-library/svelte";
import AgentCanvasPanel from "./AgentCanvasPanel.svelte";
import type { Tab } from "../types";

describe("AgentCanvasPanel", () => {
  it("opens worktree details from the worktree card", async () => {
    const tabs: Tab[] = [
      { id: "agent-1", label: "Agent One", type: "agent" },
      { id: "terminal-1", label: "Shell", type: "terminal" },
    ];

    const rendered = render(AgentCanvasPanel, {
      props: {
        projectPath: "/tmp/project",
        currentBranch: "feature/canvas",
        tabs,
      },
    });

    expect(rendered.queryByTestId("agent-canvas-worktree-dialog")).toBeNull();
    await fireEvent.click(rendered.getByTestId("agent-canvas-worktree-card"));

    const dialog = rendered.getByTestId("agent-canvas-worktree-dialog");
    expect(dialog.textContent).toContain("/tmp/project");
    expect(dialog.textContent).toContain("feature/canvas");
    expect(dialog.textContent).toContain("2");
  });
});
