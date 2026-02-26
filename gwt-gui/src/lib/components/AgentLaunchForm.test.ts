import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";
import {
  AGENT_LAUNCH_DEFAULTS_STORAGE_KEY,
  saveLaunchDefaults,
} from "../agentLaunchDefaults";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) =>
    invokeMock(...(args as [string, Record<string, unknown> | undefined])),
}));

async function renderLaunchForm(props: any) {
  const { default: AgentLaunchForm } = await import("./AgentLaunchForm.svelte");
  return render(AgentLaunchForm, { props });
}

function makeLocalStorageMock() {
  const store = new Map<string, string>();
  return {
    getItem: (key: string) => (store.has(key) ? store.get(key)! : null),
    setItem: (key: string, value: string) => {
      store.set(key, String(value));
    },
    removeItem: (key: string) => {
      store.delete(key);
    },
    clear: () => {
      store.clear();
    },
    key: (index: number) => Array.from(store.keys())[index] ?? null,
    get length() {
      return store.size;
    },
  };
}

describe("AgentLaunchForm", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    const mockLocalStorage = makeLocalStorageMock();
    Object.defineProperty(globalThis, "localStorage", {
      value: mockLocalStorage,
      configurable: true,
    });
    try {
      window.localStorage.removeItem(AGENT_LAUNCH_DEFAULTS_STORAGE_KEY);
    } catch {
      // no-op
    }
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

  it("shows only agent names in the agent dropdown", async () => {
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
          {
            id: "claude",
            name: "Claude Code",
            version: "bunx",
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

    const agentSelect = rendered.getByLabelText("Agent") as HTMLSelectElement;
    const optionLabels = Array.from(agentSelect.options).map(
      (option) => option.textContent?.trim() ?? ""
    );

    expect(optionLabels).toEqual(["Select an agent...", "Codex", "Claude Code"]);
    expect(optionLabels.some((label) => label.includes("("))).toBe(false);
    expect(optionLabels.some((label) => label.includes("bunx"))).toBe(false);
    expect(optionLabels.some((label) => label.includes("npx"))).toBe(false);
    expect(optionLabels.some((label) => label.includes("Unavailable"))).toBe(false);
  });

  it("does not show bunx/npx in fallback hints", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "bunx",
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

    const fallbackNotice = rendered.getByText("Not installed. Launch will use a fallback runner.");
    expect(fallbackNotice).toBeTruthy();

    const binaryFallbackNotice = rendered.getByText(
      "Installed binary not found. Launch will use fallback runner."
    );
    expect(binaryFallbackNotice).toBeTruthy();

    expect(rendered.queryByText(/bunx/)).toBeNull();
    expect(rendered.queryByText(/npx/)).toBeNull();
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

  it("displays claude model options with 1M context and opusplan variants", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "claude",
            name: "Claude Code",
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
    const values = Array.from(modelSelect.options).map((o) => o.value);
    const labels = Array.from(modelSelect.options).map((o) => o.textContent);

    expect(values).toEqual([
      "",
      "opus",
      "sonnet",
      "haiku",
      "opus[1m]",
      "sonnet[1m]",
      "opusplan",
    ]);
    expect(labels).toEqual([
      "Default",
      "Opus",
      "Sonnet",
      "Haiku",
      "Opus (1M context)",
      "Sonnet (1M context)",
      "Opus Plan (plan: opus / exec: sonnet)",
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

    // Switch to Direct mode to show branch name input
    await fireEvent.click(rendered.getByRole("button", { name: "Direct" }));
    const newBranchInput = rendered.getByLabelText("New Branch Name") as HTMLInputElement;
    expectInputNormalizationDisabled(newBranchInput);

    // Also verify AI Description input has normalization disabled
    await fireEvent.click(rendered.getByRole("button", { name: "AI Suggest" }));
    const descInput = rendered.getByLabelText("Description") as HTMLInputElement;
    expectInputNormalizationDisabled(descInput);
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

  it("defers gh CLI check until osEnvReady is true", async () => {
    let ghCheckCount = 0;
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
        ghCheckCount += 1;
        return { available: true, authenticated: true };
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
    const props = {
      projectPath: "/tmp/project",
      selectedBranch: "main",
      osEnvReady: false,
      onLaunch,
      onClose,
    };

    const rendered = await renderLaunchForm(props);
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));
    const fromIssueButton = rendered.getByRole("button", { name: "From Issue" }) as HTMLButtonElement;
    expect(fromIssueButton.disabled).toBe(true);
    expect(ghCheckCount).toBe(0);
    rendered.getByText("Loading shell environment...");

    await rendered.rerender({ ...props, osEnvReady: true });

    await waitFor(() => {
      expect(ghCheckCount).toBeGreaterThanOrEqual(1);
    });
    await waitFor(() => {
      expect((rendered.getByRole("button", { name: "From Issue" }) as HTMLButtonElement).disabled).toBe(
        false
      );
    });
  });

  it("shows gh missing message only after osEnvReady", async () => {
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
        return { available: false, authenticated: false };
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
    const props = {
      projectPath: "/tmp/project",
      selectedBranch: "main",
      osEnvReady: false,
      onLaunch,
      onClose,
    };

    const rendered = await renderLaunchForm(props);
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));
    expect(rendered.queryByText("GitHub CLI (gh) is not installed.")).toBeNull();
    rendered.getByText("Loading shell environment...");

    await rendered.rerender({ ...props, osEnvReady: true });

    await waitFor(() => {
      expect(rendered.queryByText("Loading shell environment...")).toBeNull();
    });
    await waitFor(() => {
      expect(rendered.getByText("GitHub CLI (gh) is not installed.")).toBeTruthy();
    });
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

  it("keeps Launch disabled in fromIssue mode until a prefix is selected", async () => {
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
      if (cmd === "classify_issue_branch_prefix") {
        return { status: "error", error: "classification failed" };
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

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));
    await fireEvent.click(rendered.getByRole("button", { name: "From Issue" }));

    await waitFor(() => {
      expect((rendered.getByRole("button", { name: /#42/i }) as HTMLButtonElement).disabled).toBe(
        false
      );
    });

    await fireEvent.click(rendered.getByRole("button", { name: /#42/i }));

    await waitFor(() => {
      expect(rendered.getByText("Auto-generated from issue #42")).toBeTruthy();
    });

    const launchButton = rendered.getByRole("button", { name: "Launch" }) as HTMLButtonElement;
    expect(launchButton.disabled).toBe(true);
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
              labels: [{ name: "bug" }],
            },
          ],
          hasNextPage: false,
        };
      }
      if (cmd === "classify_issue_branch_prefix") {
        return { status: "ok", prefix: "feature" };
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

    const launchButton = rendered.getByRole("button", { name: "Launch" }) as HTMLButtonElement;
    await waitFor(() => {
      expect(launchButton.disabled).toBe(false);
    });
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

  it("uses previous successful launch options as next defaults", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
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
          versions: ["0.90.0"],
          source: "cache",
        };
      }
      if (cmd === "detect_docker_context") {
        return {
          file_type: "compose",
          compose_services: ["app", "worker"],
          docker_available: true,
          compose_available: true,
          daemon_running: true,
          force_host: false,
        };
      }
      if (cmd === "get_agent_config") {
        return { version: 1, claude: { provider: "anthropic", glm: {} } };
      }
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      return [];
    });

    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    const first = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch,
      onClose,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.change(first.getByLabelText("Model"), {
      target: { value: "gpt-5.3-codex-spark" },
    });
    await fireEvent.change(first.getByLabelText("Agent Version"), {
      target: { value: "latest" },
    });
    await fireEvent.change(first.getByLabelText("Reasoning"), {
      target: { value: "high" },
    });
    await fireEvent.click(first.getByRole("button", { name: "Continue" }));
    await fireEvent.input(first.getByLabelText("Session ID"), {
      target: { value: "session-123" },
    });

    const permissionInput = first
      .getByText("Skip Permissions")
      .closest("label")
      ?.querySelector("input[type='checkbox']") as HTMLInputElement;
    await fireEvent.click(permissionInput);
    expect(permissionInput.checked).toBe(true);

    await fireEvent.click(first.getByRole("button", { name: "Advanced" }));
    await fireEvent.input(first.getByLabelText("Extra Args"), {
      target: { value: "--foo\n--bar" },
    });
    await fireEvent.input(first.getByLabelText("Env Overrides"), {
      target: { value: "FOO=bar" },
    });

    await waitFor(() => {
      const hostBtn = first.getByRole("button", { name: "HostOS" });
      expect(hostBtn).toBeTruthy();
    });
    await fireEvent.click(first.getByRole("button", { name: "HostOS" }));

    await fireEvent.click(first.getByRole("button", { name: "Launch" }));
    await waitFor(() => {
      expect(onLaunch).toHaveBeenCalledTimes(1);
      expect(onClose).toHaveBeenCalledTimes(1);
    });

    cleanup();

    const second = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });
    await waitFor(() => {
      expect(second.getByLabelText("Model")).toBeTruthy();
    });

    expect((second.getByLabelText("Model") as HTMLSelectElement).value).toBe(
      "gpt-5.3-codex-spark"
    );
    expect((second.getByLabelText("Agent Version") as HTMLSelectElement).value).toBe(
      "latest"
    );
    expect((second.getByLabelText("Reasoning") as HTMLSelectElement).value).toBe(
      "high"
    );
    expect(
      (second
        .getByText("Skip Permissions")
        .closest("label")
        ?.querySelector("input[type='checkbox']") as HTMLInputElement).checked
    ).toBe(true);
    expect((second.getByLabelText("Session ID") as HTMLInputElement).value).toBe(
      "session-123"
    );
    expect((second.getByLabelText("Extra Args") as HTMLTextAreaElement).value).toBe(
      "--foo\n--bar"
    );
    expect((second.getByLabelText("Env Overrides") as HTMLTextAreaElement).value).toBe(
      "FOO=bar"
    );
  });

  it("does not update defaults when closed without launching", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
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
          versions: ["0.90.0"],
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
      if (cmd === "get_agent_config") {
        return { version: 1, claude: { provider: "anthropic", glm: {} } };
      }
      return [];
    });

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Continue" }));
    await fireEvent.input(rendered.getByLabelText("Session ID"), {
      target: { value: "should-not-save" },
    });
    await fireEvent.click(rendered.getByRole("button", { name: "Cancel" }));

    cleanup();

    const reopened = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    expect(reopened.queryByLabelText("Session ID")).toBeNull();
  });

  it("does not update defaults when launch fails", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
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
          versions: ["0.90.0"],
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
      if (cmd === "get_agent_config") {
        return { version: 1, claude: { provider: "anthropic", glm: {} } };
      }
      return [];
    });

    const onLaunch = vi.fn().mockRejectedValue(new Error("boom"));
    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch,
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Continue" }));
    await fireEvent.input(rendered.getByLabelText("Session ID"), {
      target: { value: "failed-session" },
    });
    await fireEvent.click(rendered.getByRole("button", { name: "Launch" }));

    await waitFor(() => {
      expect(onLaunch).toHaveBeenCalledTimes(1);
      expect(rendered.getByText("Failed to launch agent: boom")).toBeTruthy();
    });

    cleanup();

    const reopened = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });
    expect(reopened.queryByLabelText("Session ID")).toBeNull();
  });

  it("re-evaluates installed fallback when preferred agent stays the same", async () => {
    saveLaunchDefaults({
      selectedAgent: "codex",
      sessionMode: "normal",
      modelByAgent: { codex: "gpt-5.3-codex-spark" },
      agentVersionByAgent: { codex: "installed" },
      skipPermissions: false,
      reasoningLevel: "",
      resumeSessionId: "",
      showAdvanced: false,
      extraArgsText: "",
      envOverridesText: "",
      runtimeTarget: "host",
      dockerService: "",
      dockerBuild: false,
      dockerRecreate: false,
      dockerKeep: false,
      selectedShell: "",
      branchNamingMode: "ai-suggest",
    });

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "bunx",
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
          versions: ["0.90.0"],
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
      if (cmd === "get_agent_config") {
        return { version: 1, claude: { provider: "anthropic", glm: {} } };
      }
      return [];
    });

    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch,
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await waitFor(() => {
      expect((rendered.getByLabelText("Agent Version") as HTMLSelectElement).value).toBe(
        "latest"
      );
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Launch" }));

    await waitFor(() => {
      expect(onLaunch).toHaveBeenCalledTimes(1);
    });
    expect((onLaunch.mock.calls[0][0] as any).agentVersion).toBe("latest");
  });

  it("falls back when saved defaults contain unavailable agent or invalid runtime/version", async () => {
    saveLaunchDefaults({
      selectedAgent: "unknown-agent",
      sessionMode: "continue",
      modelByAgent: { "unknown-agent": "foo/bar", codex: "gpt-5.3-codex-spark" },
      agentVersionByAgent: { codex: "installed", "unknown-agent": "latest" },
      skipPermissions: true,
      reasoningLevel: "high",
      resumeSessionId: "resume-1",
      showAdvanced: true,
      extraArgsText: "--alpha",
      envOverridesText: "X=1",
      runtimeTarget: "docker",
      dockerService: "missing-service",
      dockerBuild: true,
      dockerRecreate: true,
      dockerKeep: true,
      selectedShell: "",
      branchNamingMode: "ai-suggest",
    });

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "bunx",
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
          versions: ["0.90.0"],
          source: "cache",
        };
      }
      if (cmd === "detect_docker_context") {
        return {
          file_type: "compose",
          compose_services: ["app"],
          docker_available: false,
          compose_available: false,
          daemon_running: false,
          force_host: false,
        };
      }
      if (cmd === "get_agent_config") {
        return { version: 1, claude: { provider: "anthropic", glm: {} } };
      }
      return [];
    });

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    expect((rendered.getByLabelText("Agent") as HTMLSelectElement).value).toBe("codex");
    expect((rendered.getByLabelText("Agent Version") as HTMLSelectElement).value).toBe(
      "latest"
    );
    const hostBtn = await waitFor(() => rendered.getByRole("button", { name: "HostOS" }));
    expect(hostBtn.classList.contains("active")).toBe(true);
  });

  it("does not persist new-branch input fields into next defaults", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
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
          versions: ["0.90.0"],
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
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "get_agent_config") {
        return { version: 1, claude: { provider: "anthropic", glm: {} } };
      }
      return [];
    });

    const first = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(first.getByRole("button", { name: "New Branch" }));
    // Switch to Direct mode
    await fireEvent.click(first.getByRole("button", { name: "Direct" }));
    await fireEvent.input(first.getByLabelText("New Branch Name"), {
      target: { value: "saved-branch-name" },
    });
    await fireEvent.click(first.getByRole("button", { name: "Launch" }));

    cleanup();

    const second = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(second.queryByText("Detecting agents...")).toBeNull();
    });

    const existingBtn = second.getByRole("button", { name: "Existing Branch" });
    expect(existingBtn.classList.contains("active")).toBe(true);

    await fireEvent.click(second.getByRole("button", { name: "New Branch" }));
    await fireEvent.click(second.getByRole("button", { name: "Direct" }));
    expect((second.getByLabelText("New Branch Name") as HTMLInputElement).value).toBe("");
  });

  it("loads base branches and allows direct branch name entry", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
            authenticated: true,
            available: true,
          },
        ];
      }
      if (cmd === "list_worktree_branches") return [{ name: "main" }, { name: "develop" }];
      if (cmd === "list_remote_branches") return [{ name: "origin/release" }];
      if (cmd === "list_agent_versions") {
        return {
          agentId: "codex",
          package: "@openai/codex",
          tags: ["latest", "v0.90.0", "latest"],
          versions: ["0.90.0", "0.89.0", "0.90.0"],
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

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));

    const baseSelect = rendered.getByLabelText("Base Branch") as HTMLSelectElement;
    await waitFor(() => {
      const values = Array.from(baseSelect.options).map((o) => o.value);
      expect(values).toContain("main");
      expect(values).toContain("develop");
      expect(values).toContain("origin/release");
    });

    // Switch to Direct mode (default is AI Suggest)
    await fireEvent.click(rendered.getByRole("button", { name: "Direct" }));

    // Test branch name paste in Direct mode
    const newBranchInput = rendered.getByLabelText("New Branch Name") as HTMLInputElement;
    const prefixSelect = rendered.container.querySelector("#new-branch-prefix-select") as HTMLSelectElement;
    await fireEvent.input(newBranchInput, { target: { value: "release/ship-now" } });
    expect(prefixSelect.value).toBe("release/");
    expect(newBranchInput.value).toBe("ship-now");
  });


  it("shows gh unauthenticated warning when gh exists but auth is missing", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
            authenticated: true,
            available: true,
          },
        ];
      }
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: false };
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "list_agent_versions") {
        return {
          agentId: "codex",
          package: "@openai/codex",
          tags: ["latest"],
          versions: ["0.90.0"],
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

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("check_gh_cli_status", {
        projectPath: "/tmp/project",
      });
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));
    await waitFor(() => {
      expect(
        rendered.getByText("GitHub CLI (gh) is not authenticated. Run: gh auth login")
      ).toBeTruthy();
      expect((rendered.getByRole("button", { name: "From Issue" }) as HTMLButtonElement).disabled).toBe(
        true
      );
    });
  });

  it("renders issue labels, branch-exists state, search filter, and infinite scroll paging", async () => {
    let issuePageCalls = 0;
    invokeMock.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
            authenticated: true,
            available: true,
          },
        ];
      }
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: true };
      if (cmd === "fetch_github_issues") {
        issuePageCalls += 1;
        if (args?.page === 1) {
          return {
            issues: [
              {
                number: 1,
                title: "First issue",
                updatedAt: "2026-02-13T00:00:00Z",
                labels: [{ name: "backend", color: "0075ca" }, { name: "urgent", color: "e4e669" }],
              },
            ],
            hasNextPage: true,
          };
        }
        return {
          issues: [
            {
              number: 2,
              title: "Second issue",
              updatedAt: "2026-02-14T00:00:00Z",
              labels: [],
            },
          ],
          hasNextPage: false,
        };
      }
      if (cmd === "find_existing_issue_branch") {
        if (args?.issueNumber === 1) return "feature/issue-1";
        return null;
      }
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "list_agent_versions") {
        return {
          agentId: "codex",
          package: "@openai/codex",
          tags: ["latest"],
          versions: ["0.90.0"],
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

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));
    await fireEvent.click(rendered.getByRole("button", { name: "From Issue" }));

    await waitFor(() => {
      expect(rendered.getByText("First issue")).toBeTruthy();
    });
    await waitFor(() => {
      expect(rendered.getByText("backend")).toBeTruthy();
      expect(rendered.getByText("urgent")).toBeTruthy();
      expect(rendered.getByText("Branch exists")).toBeTruthy();
    });

    await fireEvent.input(rendered.getByLabelText("Search Issues"), {
      target: { value: "Second" },
    });
    await waitFor(() => {
      expect(rendered.queryByText("First issue")).toBeNull();
    });

    await fireEvent.input(rendered.getByLabelText("Search Issues"), {
      target: { value: "" },
    });
    const issueList = rendered.container.querySelector(".issue-list") as HTMLDivElement;
    Object.defineProperty(issueList, "scrollHeight", { configurable: true, value: 200 });
    Object.defineProperty(issueList, "clientHeight", { configurable: true, value: 50 });
    Object.defineProperty(issueList, "scrollTop", {
      configurable: true,
      writable: true,
      value: 151,
    });
    await fireEvent.scroll(issueList);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("fetch_github_issues", {
        projectPath: "/tmp/project",
        page: 2,
        perPage: 30,
        state: "open",
      });
      expect(rendered.getByText("Second issue")).toBeTruthy();
      expect(issuePageCalls).toBeGreaterThanOrEqual(2);
    });
  });

  it("shows GitHub API rate-limit error on issue fetch failure", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
            authenticated: true,
            available: true,
          },
        ];
      }
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: true };
      if (cmd === "fetch_github_issues") throw new Error("API rate limit exceeded");
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "list_agent_versions") {
        return {
          agentId: "codex",
          package: "@openai/codex",
          tags: ["latest"],
          versions: ["0.90.0"],
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

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));
    await fireEvent.click(rendered.getByRole("button", { name: "From Issue" }));

    await waitFor(() => {
      expect(rendered.getByText("GitHub API rate limit reached. Please try again later.")).toBeTruthy();
    });
  });

  it("shows agent config and version loading warnings when those backend calls fail", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "claude",
            name: "Claude Code",
            version: "1.2.3",
            authenticated: true,
            available: true,
          },
        ];
      }
      if (cmd === "list_agent_versions") throw "registry unavailable";
      if (cmd === "get_agent_config") throw "config unavailable";
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
      return [];
    });

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await waitFor(() => {
      expect(rendered.getByText("Failed to load agent config: config unavailable")).toBeTruthy();
      expect(rendered.getByText("Failed to load version list from registry.")).toBeTruthy();
    });
  });

  it("blocks docker launch when compose service is missing", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
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
          versions: ["0.90.0"],
          source: "cache",
        };
      }
      if (cmd === "detect_docker_context") {
        return {
          file_type: "compose",
          compose_services: [],
          docker_available: true,
          compose_available: true,
          daemon_running: false,
          force_host: false,
        };
      }
      if (cmd === "get_agent_config") return { version: 1, claude: { provider: "anthropic", glm: {} } };
      return [];
    });

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await waitFor(() => {
      expect(rendered.getByText("No services found in compose file.")).toBeTruthy();
      expect(
        rendered.getByText("Docker daemon is not running. gwt will try to start it on launch.")
      ).toBeTruthy();
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Launch" }));
    await waitFor(() => {
      expect(rendered.getByText("Docker service is required.")).toBeTruthy();
    });
  });

  it("includes docker compose launch options in the launch request", async () => {
    const onLaunch = vi.fn().mockResolvedValue(undefined);
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
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
          versions: ["0.90.0"],
          source: "cache",
        };
      }
      if (cmd === "detect_docker_context") {
        return {
          file_type: "compose",
          compose_services: ["app", "worker"],
          docker_available: true,
          compose_available: true,
          daemon_running: true,
          force_host: false,
        };
      }
      if (cmd === "get_agent_config") return { version: 1, claude: { provider: "anthropic", glm: {} } };
      return [];
    });

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch,
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    const serviceSelect = await waitFor(
      () => rendered.getByLabelText("Service") as HTMLSelectElement
    );
    await fireEvent.change(serviceSelect, { target: { value: "worker" } });

    const checks = rendered.container.querySelectorAll(".check-row input[type='checkbox']");
    await fireEvent.click(checks[1] as HTMLInputElement);
    await fireEvent.click(checks[2] as HTMLInputElement);
    await fireEvent.click(checks[3] as HTMLInputElement);

    await fireEvent.click(rendered.getByRole("button", { name: "Launch" }));
    await waitFor(() => {
      expect(onLaunch).toHaveBeenCalledTimes(1);
    });

    const req = onLaunch.mock.calls[0][0] as any;
    expect(req.branch).toBe("main");
    expect(req.dockerService).toBe("worker");
    expect(req.dockerBuild).toBe(true);
    expect(req.dockerRecreate).toBe(true);
    expect(req.dockerKeep).toBe(true);
  });

  it("shows env override parse errors and handles Escape key modal behavior", async () => {
    const onClose = vi.fn();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
            authenticated: true,
            available: true,
          },
        ];
      }
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "list_agent_versions") {
        return {
          agentId: "codex",
          package: "@openai/codex",
          tags: ["latest"],
          versions: ["0.90.0"],
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

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Advanced" }));
    await fireEvent.input(rendered.getByLabelText("Env Overrides"), {
      target: { value: "INVALID_LINE" },
    });
    await fireEvent.click(rendered.getByRole("button", { name: "Launch" }));

    await waitFor(() => {
      expect(rendered.getByText("Invalid env override at line 1. Use KEY=VALUE.")).toBeTruthy();
    });

    // Verify pressing Escape closes the main dialog
    const overlay = rendered.container.querySelector(".overlay") as HTMLDivElement;
    await fireEvent.keyDown(overlay, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("hides shell dropdown when no shells are available", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
            authenticated: true,
            available: true,
          },
        ];
      }
      if (cmd === "get_available_shells") return [];
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

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Advanced" }));

    expect(rendered.queryByLabelText("Shell")).toBeNull();
  });

  it("shows shell dropdown when shells are available", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
            authenticated: true,
            available: true,
          },
        ];
      }
      if (cmd === "get_available_shells") {
        return [
          { id: "powershell", name: "PowerShell", version: "7.4.0" },
          { id: "cmd", name: "Command Prompt" },
        ];
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

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Advanced" }));

    await waitFor(() => {
      const shellSelect = rendered.getByLabelText("Shell") as HTMLSelectElement;
      expect(shellSelect).toBeTruthy();
      const options = Array.from(shellSelect.options).map((o) => o.textContent);
      expect(options).toEqual(["Auto", "PowerShell (7.4.0)", "Command Prompt"]);
    });
  });

  it("disables shell dropdown when docker runtime is selected", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") {
        return [
          {
            id: "codex",
            name: "Codex",
            version: "0.90.0",
            authenticated: true,
            available: true,
          },
        ];
      }
      if (cmd === "get_available_shells") {
        return [
          { id: "powershell", name: "PowerShell", version: "7.4.0" },
          { id: "cmd", name: "Command Prompt" },
        ];
      }
      if (cmd === "list_agent_versions") {
        return {
          agentId: "codex",
          package: "@openai/codex",
          tags: ["latest"],
          versions: ["0.90.0"],
          source: "cache",
        };
      }
      if (cmd === "detect_docker_context") {
        return {
          file_type: "compose",
          compose_services: ["app"],
          docker_available: true,
          compose_available: true,
          daemon_running: true,
          force_host: false,
        };
      }
      if (cmd === "get_agent_config") return { version: 1, claude: { provider: "anthropic", glm: {} } };
      return [];
    });

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Advanced" }));

    await waitFor(() => {
      const shellSelect = rendered.getByLabelText("Shell") as HTMLSelectElement;
      expect(shellSelect.disabled).toBe(true);
      expect(rendered.getByText("Container default")).toBeTruthy();
    });
  });

  // ======== Phase 3 (T030-T031): US2 Direct mode tests ========

  it("shows prefix and suffix inputs when Direct mode is selected", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") return [{ id: "codex", name: "Codex", version: "0.0.0", authenticated: true, available: true }];
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      return [];
    });

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));

    // Default is AI Suggest - verify Description is shown, not branch name
    expect(rendered.queryByLabelText("Description")).not.toBeNull();
    expect(rendered.queryByLabelText("New Branch Name")).toBeNull();

    // Switch to Direct mode
    await fireEvent.click(rendered.getByRole("button", { name: "Direct" }));

    // Now Prefix+Suffix should be visible
    expect(rendered.queryByLabelText("New Branch Name")).not.toBeNull();
    expect(rendered.queryByLabelText("Description")).toBeNull();
    expect(rendered.container.querySelector("#new-branch-prefix-select")).not.toBeNull();
  });

  // ======== Phase 4 (T032, T036): Persistence tests ========

  it("persists and restores branchNamingMode from localStorage", async () => {
    // Save defaults with direct mode
    saveLaunchDefaults({
      selectedAgent: "codex",
      sessionMode: "normal",
      modelByAgent: {},
      agentVersionByAgent: {},
      skipPermissions: false,
      reasoningLevel: "",
      resumeSessionId: "",
      showAdvanced: false,
      extraArgsText: "",
      envOverridesText: "",
      runtimeTarget: "host",
      dockerService: "",
      dockerBuild: false,
      dockerRecreate: false,
      dockerKeep: false,
      selectedShell: "",
      branchNamingMode: "direct",
    });

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") return [{ id: "codex", name: "Codex", version: "0.0.0", authenticated: true, available: true }];
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      return [];
    });

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));

    // Should restore "direct" mode - New Branch Name should be visible, not Description
    expect(rendered.queryByLabelText("New Branch Name")).not.toBeNull();
    expect(rendered.queryByLabelText("Description")).toBeNull();
  });

  // ======== Phase 5 (T037-T046): Fallback + AI not configured ========

  it("uses AI-suggested branch name for launch request", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { description?: string }) => {
      if (cmd === "detect_agents") return [{ id: "codex", name: "Codex", version: "0.0.0", authenticated: true, available: true }];
      if (cmd === "list_worktree_branches") return [{ name: "main" }];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "suggest_branch_name") {
        if (args?.description === "my feature") {
          return { status: "ok", suggestion: "feature/my-feature", error: null };
        }
        return { status: "ok", suggestion: "feature/test", error: null };
      }
      return [];
    });

    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "",
      onLaunch,
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));
    await fireEvent.input(rendered.getByLabelText("Description"), { target: { value: "my feature" } });

    await waitFor(() => {
      const baseSelect = rendered.getByLabelText("Base Branch") as HTMLSelectElement;
      const options = Array.from(baseSelect.options).map((o) => o.value);
      expect(options).toContain("main");
    });
    await fireEvent.change(rendered.getByLabelText("Base Branch"), { target: { value: "main" } });

    await fireEvent.click(rendered.getByRole("button", { name: "Launch" }));

    await waitFor(() => {
      expect(onLaunch).toHaveBeenCalledTimes(1);
    });

    const request = onLaunch.mock.calls[0][0] as any;
    expect(request.branch).toBe("feature/my-feature");
    expect(request.createBranch).toEqual({ name: "feature/my-feature", base: "main" });
    expect(request.aiBranchDescription).toBeUndefined();
  });

  it("falls back to Direct mode with error banner when AI suggestion fails", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") return [{ id: "codex", name: "Codex", version: "0.0.0", authenticated: true, available: true }];
      if (cmd === "list_worktree_branches") return [{ name: "main" }];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "suggest_branch_name") return { status: "error", suggestion: "", error: "timeout" };
      return [];
    });

    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "",
      onLaunch,
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));

    // Enter description in AI Suggest mode (default)
    await fireEvent.input(rendered.getByLabelText("Description"), { target: { value: "my feature" } });

    // Wait for base branch to be loaded and select it
    await waitFor(() => {
      const baseSelect = rendered.getByLabelText("Base Branch") as HTMLSelectElement;
      expect(baseSelect).toBeTruthy();
      const options = Array.from(baseSelect.options).map((o) => o.value);
      expect(options).toContain("main");
    });
    await fireEvent.change(rendered.getByLabelText("Base Branch"), { target: { value: "main" } });

    await fireEvent.click(rendered.getByRole("button", { name: "Launch" }));

    await waitFor(() => {
      expect(rendered.queryByText(/AI branch name generation failed/)).not.toBeNull();
    });

    expect(onLaunch).not.toHaveBeenCalled();

    // Should switch to Direct mode
    expect(rendered.queryByLabelText("New Branch Name")).not.toBeNull();
  });

  it("clears fallback error banner when mode is switched", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { description?: string }) => {
      if (cmd === "detect_agents") return [{ id: "codex", name: "Codex", version: "0.0.0", authenticated: true, available: true }];
      if (cmd === "list_worktree_branches") return [{ name: "main" }];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "suggest_branch_name") {
        if (args?.description === "my feature") {
          return { status: "error", suggestion: "", error: "timeout" };
        }
        return { status: "ok", suggestion: "feature/test", error: null };
      }
      return [];
    });

    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "",
      onLaunch,
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));
    await fireEvent.input(rendered.getByLabelText("Description"), { target: { value: "my feature" } });

    // Wait for base branch to be loaded and select it
    await waitFor(() => {
      const baseSelect = rendered.getByLabelText("Base Branch") as HTMLSelectElement;
      const options = Array.from(baseSelect.options).map((o) => o.value);
      expect(options).toContain("main");
    });
    await fireEvent.change(rendered.getByLabelText("Base Branch"), { target: { value: "main" } });

    await fireEvent.click(rendered.getByRole("button", { name: "Launch" }));

    await waitFor(() => {
      expect(rendered.queryByText(/AI branch name generation failed/)).not.toBeNull();
    });

    // Switch back to AI Suggest - error should clear
    await fireEvent.click(rendered.getByRole("button", { name: "AI Suggest" }));
    expect(rendered.queryByText(/AI branch name generation failed/)).toBeNull();
  });

  it("disables AI Suggest segment when AI is not configured", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") return [{ id: "codex", name: "Codex", version: "0.0.0", authenticated: true, available: true }];
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "suggest_branch_name") return { status: "ai-not-configured", suggestion: "", error: null };
      return [];
    });

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));

    await waitFor(() => {
      const aiSuggestBtn = rendered.getByRole("button", { name: "AI Suggest" });
      expect(aiSuggestBtn).toHaveProperty("disabled", true);
    });

    // Should fall back to Direct mode
    expect(rendered.queryByLabelText("New Branch Name")).not.toBeNull();
  });

  // ======== Phase 6 (T047-T048): fromIssue tab isolation ========

  it("does not show branch naming toggle in fromIssue tab", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_agents") return [{ id: "codex", name: "Codex", version: "0.0.0", authenticated: true, available: true }];
      if (cmd === "list_worktree_branches") return [];
      if (cmd === "list_remote_branches") return [];
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: true };
      if (cmd === "fetch_github_issues") return { issues: [], hasNextPage: false };
      return [];
    });

    const rendered = await renderLaunchForm({
      projectPath: "/tmp/project",
      selectedBranch: "",
      onLaunch: vi.fn().mockResolvedValue(undefined),
      onClose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "New Branch" }));

    // In manual tab, toggle should exist
    expect(rendered.container.querySelector(".branch-naming-toggle")).not.toBeNull();

    // Switch to fromIssue tab
    await fireEvent.click(rendered.getByRole("button", { name: "From Issue" }));

    // Toggle should not be visible
    expect(rendered.container.querySelector(".branch-naming-toggle")).toBeNull();
  });
});
