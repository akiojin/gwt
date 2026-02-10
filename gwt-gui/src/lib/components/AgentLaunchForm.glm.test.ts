import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

async function renderForm(props: any) {
  const { default: AgentLaunchForm } = await import("./AgentLaunchForm.svelte");
  return render(AgentLaunchForm, { props });
}

function defaultAgentConfig() {
  return {
    version: 1,
    claude: {
      provider: "anthropic",
      glm: {
        base_url: "https://api.z.ai/api/anthropic",
        auth_token: "",
        api_timeout_ms: "3000000",
        default_opus_model: "glm-4.7",
        default_sonnet_model: "glm-4.7",
        default_haiku_model: "glm-4.5-air",
      },
    },
  };
}

describe("AgentLaunchForm (Claude GLM)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string, args?: any) => {
      switch (cmd) {
        case "detect_agents":
          return Promise.resolve([
            {
              id: "claude",
              name: "Claude Code",
              version: "1.0.0",
              authenticated: true,
              available: true,
            },
          ]);
        case "list_agent_versions":
          return Promise.resolve({
            agentId: args?.agentId ?? "claude",
            package: "@anthropic-ai/claude-code",
            tags: ["latest"],
            versions: ["1.0.0"],
            source: "cache",
          });
        case "detect_docker_context":
          return Promise.resolve({
            file_type: "none",
            compose_services: [],
            docker_available: false,
            compose_available: false,
            daemon_running: false,
            force_host: false,
          });
        case "get_agent_config":
          return Promise.resolve(defaultAgentConfig());
        case "save_agent_config":
          return Promise.resolve(null);
        default:
          throw new Error(`Unexpected invoke: ${cmd}`);
      }
    });

    // Some codepaths can end up calling the real Tauri invoke implementation in tests.
    // Provide a minimal stub so it routes to our mock instead of crashing on
    // `window.__TAURI_INTERNALS__` being undefined.
    (window as any).__TAURI_INTERNALS__ = {
      ...(window as any).__TAURI_INTERNALS__,
      invoke: (cmd: string, args?: any) => invokeMock(cmd, args),
    };
  });

  afterEach(() => {
    cleanup();
  });

  it("injects GLM env vars on launch and prefers Advanced env overrides", async () => {
    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    const rendered = await renderForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch,
      onClose,
    });

    const providerSelect = await waitFor(() =>
      rendered.getByLabelText("Provider")
    );
    await waitFor(() => {
      expect((providerSelect as HTMLSelectElement).disabled).toBe(false);
    });

    await fireEvent.change(providerSelect, { target: { value: "glm" } });
    await waitFor(() => {
      expect((providerSelect as HTMLSelectElement).value).toBe("glm");
    });

    const tokenInput = await waitFor(() =>
      rendered.getByLabelText("API Token")
    );
    await fireEvent.input(tokenInput, { target: { value: "tok_123" } });

    const advancedBtn = rendered.getByRole("button", { name: "Advanced" });
    await fireEvent.click(advancedBtn);

    const envOverrides = rendered.getByLabelText("Env Overrides");
    await fireEvent.input(envOverrides, { target: { value: "API_TIMEOUT_MS=123" } });

    const launchBtn = rendered.getByRole("button", { name: "Launch" });
    await fireEvent.click(launchBtn);

    await waitFor(() => {
      expect(onLaunch).toHaveBeenCalledTimes(1);
    });

    const request = onLaunch.mock.calls[0][0] as any;
    expect(request.agentId).toBe("claude");
    expect(request.envOverrides.ANTHROPIC_BASE_URL).toBe("https://api.z.ai/api/anthropic");
    expect(request.envOverrides.ANTHROPIC_AUTH_TOKEN).toBe("tok_123");
    expect(request.envOverrides.API_TIMEOUT_MS).toBe("123");
    expect(request.envOverrides.ANTHROPIC_DEFAULT_OPUS_MODEL).toBe("glm-4.7");
    expect(request.envOverrides.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("glm-4.7");
    expect(request.envOverrides.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe("glm-4.5-air");

    const saveCall = invokeMock.mock.calls.find((c: any[]) => c[0] === "save_agent_config");
    expect(saveCall).toBeTruthy();
    expect(saveCall?.[1]?.config?.claude?.provider).toBe("glm");
    expect(saveCall?.[1]?.config?.claude?.glm?.auth_token).toBe("tok_123");
  });

  it("does not inject GLM env vars when provider is Anthropic", async () => {
    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    const rendered = await renderForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch,
      onClose,
    });

    await waitFor(() => {
      expect(rendered.getByLabelText("Provider")).toBeTruthy();
    });

    const launchBtn = rendered.getByRole("button", { name: "Launch" });
    await fireEvent.click(launchBtn);

    await waitFor(() => {
      expect(onLaunch).toHaveBeenCalledTimes(1);
    });

    const request = onLaunch.mock.calls[0][0] as any;
    expect(request.agentId).toBe("claude");
    expect(request.envOverrides).toBeUndefined();
  });

  it("persists switching back to Anthropic before launch", async () => {
    const onLaunch = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    const rendered = await renderForm({
      projectPath: "/tmp/project",
      selectedBranch: "main",
      onLaunch,
      onClose,
    });

    const providerSelect = await waitFor(() =>
      rendered.getByLabelText("Provider")
    );
    await waitFor(() => {
      expect((providerSelect as HTMLSelectElement).disabled).toBe(false);
    });

    await fireEvent.change(providerSelect, { target: { value: "glm" } });
    await waitFor(() => {
      expect((providerSelect as HTMLSelectElement).value).toBe("glm");
    });

    const tokenInput = await waitFor(() =>
      rendered.getByLabelText("API Token")
    );
    await fireEvent.input(tokenInput, { target: { value: "tok_123" } });

    await fireEvent.change(providerSelect, { target: { value: "anthropic" } });

    const launchBtn = rendered.getByRole("button", { name: "Launch" });
    await fireEvent.click(launchBtn);

    await waitFor(() => {
      expect(onLaunch).toHaveBeenCalledTimes(1);
    });

    const request = onLaunch.mock.calls[0][0] as any;
    expect(request.agentId).toBe("claude");
    expect(request.envOverrides).toBeUndefined();

    const saveCalls = invokeMock.mock.calls.filter((c: any[]) => c[0] === "save_agent_config");
    expect(saveCalls.length).toBeGreaterThan(0);
    const lastSave = saveCalls[saveCalls.length - 1];
    expect(lastSave?.[1]?.config?.claude?.provider).toBe("anthropic");
  });
});
