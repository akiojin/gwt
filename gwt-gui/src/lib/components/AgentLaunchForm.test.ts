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

  it("displays new codex model options including gpt-5.3-codex-spark", async () => {
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
      if (cmd === "get_agent_config") {
        return { version: 1, claude: { provider: "anthropic", glm: {} } };
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

    const modelSelect = rendered.getByLabelText("Model") as HTMLSelectElement;
    const options = Array.from(modelSelect.options).map((option) => option.value);
    expect(options).toEqual([
      "",
      "gpt-5.3-codex",
      "gpt-5.3-codex-spark",
      "gpt-5.2-codex",
      "gpt-5.1-codex-max",
      "gpt-5.2",
      "gpt-5.1-codex-mini",
    ]);
  });

  it("passes selected codex model to launch request", async () => {
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
      if (cmd === "get_agent_config") {
        return { version: 1, claude: { provider: "anthropic", glm: {} } };
      }
      return [];
    });

    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch,
      onClose,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    const modelSelect = rendered.getByLabelText("Model") as HTMLSelectElement;
    await fireEvent.change(modelSelect, { target: { value: "gpt-5.3-codex-spark" } });

    const launchBtn = rendered.getByRole("button", { name: "Launch" });
    await fireEvent.click(launchBtn);

    await waitFor(() => {
      expect(onLaunch).toHaveBeenCalledTimes(1);
    });

    const request = onLaunch.mock.calls[0][0] as any;
    expect(request.agentId).toBe("codex");
    expect(request.model).toBe("gpt-5.3-codex-spark");
  });

  it("disables capitalization and completion helpers for text and textarea inputs", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "opencode",
            name: "OpenCode",
            version: "0.0.0",
            authenticated: true,
            available: true,
          },
        ];
      }
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      return [];
    });

    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "feature/current-branch",
      onLaunch,
      onClose,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    const expectInputNormalizationDisabled = (
      element: HTMLInputElement | HTMLTextAreaElement | null
    ) => {
      expect(element).toBeTruthy();
      expect(element?.getAttribute("autocapitalize")).toBe("off");
      expect(element?.getAttribute("autocorrect")).toBe("off");
      expect(element?.getAttribute("autocomplete")).toBe("off");
      expect(element?.getAttribute("spellcheck")).toBe("false");
    };

    const modelInput = rendered.getByLabelText("Model") as HTMLInputElement;
    expectInputNormalizationDisabled(modelInput);

    const branchInput = rendered.container.querySelector(
      "#branch-input"
    ) as HTMLInputElement | null;
    expectInputNormalizationDisabled(branchInput);

    await fireEvent.click(rendered.getByRole("button", { name: "Continue" }));
    const sessionInput = rendered.getByLabelText("Session ID") as HTMLInputElement;
    expectInputNormalizationDisabled(sessionInput);

    await fireEvent.click(rendered.getByRole("button", { name: "Advanced" }));
    const extraArgsInput = rendered.getByLabelText("Extra Args") as HTMLTextAreaElement;
    expectInputNormalizationDisabled(extraArgsInput);
    const envOverridesInput = rendered.getByLabelText("Env Overrides") as HTMLTextAreaElement;
    expectInputNormalizationDisabled(envOverridesInput);

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));
    const newBranchInput = rendered.getByLabelText("New Branch Name") as HTMLInputElement;
    expectInputNormalizationDisabled(newBranchInput);
  });

  it("forces host launch even when docker context is not detected (e.g., remote-only branch without worktree)", async () => {
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
      if (cmd === "list_agent_versions") {
        return {
          agentId: "codex",
          package: "@openai/codex",
          tags: ["latest"],
          versions: ["0.0.0"],
          source: "cache",
        };
      }
      if (cmd === "detect_docker_context") {
        return {
          file_type: "none",
          compose_services: [],
          docker_available: false,
          compose_available: false,
          daemon_running: false,
          force_host: false,
        };
      }
      if (cmd === "get_agent_config") return { version: 1, claude: { provider: "anthropic", glm: {} } };
      return [];
    });

    // Some codepaths can end up calling the real Tauri invoke implementation in tests.
    // Provide a minimal stub so it routes to our mock instead of crashing on
    // `window.__TAURI_INTERNALS__` being undefined.
    (window as any).__TAURI_INTERNALS__ = {
      ...(window as any).__TAURI_INTERNALS__,
      invoke: (cmd: string, args?: any) => invokeMock(cmd, args),
    };

    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "origin/remote-only",
      onLaunch,
      onClose,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    const launchBtn = rendered.getByRole("button", { name: "Launch" });
    await fireEvent.click(launchBtn);

    await waitFor(() => {
      expect(onLaunch).toHaveBeenCalledTimes(1);
    });

    const request = onLaunch.mock.calls[0][0] as any;
    expect(request.dockerForceHost).toBe(true);
  });

  it("keeps issue selection disabled while duplicate-branch check is pending", async () => {
    let resolveBranchCheck!: (value: string | null) => void;
    const branchCheck = new Promise<string | null>((resolve) => {
      resolveBranchCheck = resolve;
    });

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
      if (cmd === "check_gh_cli_status") {
        return { available: true, authenticated: true };
      }
      if (cmd === "fetch_github_issues") {
        return {
          issues: [
            {
              number: 42,
              title: "Issue 42",
              updatedAt: "2026-02-13T00:00:00Z",
              labels: [],
            },
          ],
          hasNextPage: false,
        };
      }
      if (cmd === "find_existing_issue_branch") {
        return branchCheck;
      }
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "detect_docker_context") {
        return {
          file_type: "none",
          compose_services: [],
          docker_available: false,
          compose_available: false,
          daemon_running: false,
          force_host: false,
        };
      }
      if (cmd === "get_agent_config") return { version: 1, claude: { provider: "anthropic", glm: {} } };
      return [];
    });

    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch,
      onClose,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));
    await fireEvent.click(rendered.getByRole("button", { name: "From Issue" }));

    const issueTitle = await waitFor(() => rendered.getByText("Issue 42"));
    const issueButton = issueTitle.closest("button") as HTMLButtonElement;
    expect(issueButton.disabled).toBe(true);

    await fireEvent.click(issueButton);
    expect(rendered.queryByText("Auto-generated from issue #42")).toBeNull();
    expect((rendered.getByRole("button", { name: "Launch" }) as HTMLButtonElement).disabled).toBe(
      true
    );

    resolveBranchCheck(null);

    await waitFor(() => {
      expect((rendered.getByRole("button", { name: /#42/i }) as HTMLButtonElement).disabled).toBe(
        false
      );
    });
  });

  it("does not link or rollback issue branch before async launch job completion", async () => {
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
      if (cmd === "check_gh_cli_status") {
        return { available: true, authenticated: true };
      }
      if (cmd === "fetch_github_issues") {
        return {
          issues: [
            {
              number: 99,
              title: "Issue 99",
              updatedAt: "2026-02-13T00:00:00Z",
              labels: [],
            },
          ],
          hasNextPage: false,
        };
      }
      if (cmd === "find_existing_issue_branch") return null;
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "detect_docker_context") {
        return {
          file_type: "none",
          compose_services: [],
          docker_available: false,
          compose_available: false,
          daemon_running: false,
          force_host: false,
        };
      }
      if (cmd === "get_agent_config") return { version: 1, claude: { provider: "anthropic", glm: {} } };
      return [];
    });

    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch,
      onClose,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));
    await fireEvent.click(rendered.getByRole("button", { name: "From Issue" }));

    await waitFor(() => {
      expect((rendered.getByRole("button", { name: /#99/i }) as HTMLButtonElement).disabled).toBe(
        false
      );
    });
    const issueButton = rendered.getByRole("button", { name: /#99/i });
    await fireEvent.click(issueButton);

    const launchButton = rendered.getByRole("button", { name: "Launch" });
    await fireEvent.click(launchButton);

    await waitFor(() => {
      expect(onLaunch).toHaveBeenCalledTimes(1);
    });

    const request = onLaunch.mock.calls[0][0] as any;
    expect(request.issueNumber).toBe(99);
    expect(
      invokeMock.mock.calls.some((call: any[]) => call[0] === "link_branch_to_issue")
    ).toBe(false);
    expect(
      invokeMock.mock.calls.some((call: any[]) => call[0] === "rollback_issue_branch")
    ).toBe(false);
  });
});
