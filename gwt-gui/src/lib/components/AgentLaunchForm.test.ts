import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

async function renderLaunchForm(props: any) {
  const { default: AgentLaunchForm } = await import("./AgentLaunchForm.svelte");
  return render(AgentLaunchForm, { props });
}

describe("AgentLaunchForm", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("keeps selectedAgent empty when all agents are unavailable", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.0.0",
            authenticated: true,
            available: false,
          },
          {
            id: "claude",
            name: "Claude Code",
            version: "0.0.0",
            authenticated: true,
            available: false,
          },
        ];
      }
      return [];
    });

    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "",
      onLaunch,
      onClose,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    const agentSelect = rendered.getByLabelText("Agent") as HTMLSelectElement;
    expect(agentSelect.value).toBe("");
    expect(
      (rendered.getByRole("button", { name: "Launch" }) as HTMLButtonElement).disabled
    ).toBe(true);
    expect(
      (rendered.getByRole("option", { name: "Select an agent..." }) as HTMLOptionElement).disabled
    ).toBe(true);
  });

  it("does not close suggest modal when applying an invalid suggestion", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.0.0",
            authenticated: true,
            available: true,
          },
        ];
      }
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "suggest_branch_names") {
        return {
          status: "ok",
          suggestions: ["chore/update-deps", "docs/fix-readme", "feature/good"],
          error: null,
        };
      }
      return [];
    });

    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "",
      onLaunch,
      onClose,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));
    await fireEvent.click(rendered.getByRole("button", { name: "Suggest..." }));

    rendered.getByRole("heading", { name: "Suggest Branch Name" });

    await fireEvent.input(rendered.getByLabelText("Description"), {
      target: { value: "update dependencies" },
    });
    await fireEvent.click(rendered.getByRole("button", { name: "Generate" }));

    await waitFor(() => {
      expect(rendered.queryByText("chore/update-deps")).not.toBeNull();
    });

    // Selecting an invalid suggestion should keep the modal open and show an error.
    await fireEvent.click(rendered.getByText("chore/update-deps"));

    rendered.getByRole("heading", { name: "Suggest Branch Name" });
    rendered.getByText("Invalid suggestion prefix.");
  });

  it("clears suggestions when the backend returns ok with a wrong count", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.0.0",
            authenticated: true,
            available: true,
          },
        ];
      }
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "suggest_branch_names") {
        return {
          status: "ok",
          suggestions: ["feature/a", "bugfix/b"],
          error: null,
        };
      }
      return [];
    });

    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "",
      onLaunch,
      onClose,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));
    await fireEvent.click(rendered.getByRole("button", { name: "Suggest..." }));

    await fireEvent.input(rendered.getByLabelText("Description"), {
      target: { value: "some work" },
    });
    await fireEvent.click(rendered.getByRole("button", { name: "Generate" }));

    await waitFor(() => {
      expect(rendered.queryByText("Failed to generate suggestions.")).not.toBeNull();
    });

    // Suggestions should be cleared when showing the error.
    expect(rendered.queryByText("feature/a")).toBeNull();
  });
});
