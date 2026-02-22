import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

import type {
  SkillRegistrationStatus,
  ProfilesConfig,
  SettingsData,
  ShellInfo,
} from "../types";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
}));

const settingsFixture: SettingsData = {
  protected_branches: ["main", "develop"],
  default_base_branch: "main",
  worktree_root: "/tmp/worktrees",
  debug: false,
  log_retention_days: 30,
  agent_default: "codex",
  agent_auto_install_deps: true,
  agent_skill_registration_default_scope: "user",
  agent_skill_registration_codex_scope: null,
  agent_skill_registration_claude_scope: null,
  agent_skill_registration_gemini_scope: null,
  docker_force_host: true,
  ui_font_size: 13,
  terminal_font_size: 13,
  ui_font_family: 'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
  terminal_font_family: '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace',
  app_language: "auto",
  voice_input: {
    enabled: false,
    hotkey: "Mod+Shift+M",
    language: "auto",
    model: "base",
  },
};

const profilesFixture: ProfilesConfig = {
  version: 1,
  active: "default",
  profiles: {
    default: {
      name: "default",
      description: "",
      env: { API_KEY: "xxx" },
      disabled_env: [],
      ai_enabled: true,
      ai: {
        endpoint: "https://api.openai.com/v1",
        api_key: "test-key",
        model: "gpt-4o-mini",
        language: "en",
        summary_enabled: true,
      },
    },
  },
};

const skillStatusFixture: SkillRegistrationStatus = {
  overall: "ok",
  agents: [
    {
      agent_id: "claude",
      label: "Claude Code",
      skills_path: "/tmp/.claude/skills",
      registered: true,
      missing_skills: [],
      error_code: null,
      error_message: null,
    },
    {
      agent_id: "codex",
      label: "Codex",
      skills_path: "/tmp/.codex/skills",
      registered: true,
      missing_skills: [],
      error_code: null,
      error_message: null,
    },
  ],
  last_checked_at: 1_739_763_600_000,
  last_error_message: null,
};

const shellsFixture: ShellInfo[] = [
  { id: "powershell", name: "PowerShell", version: "7.4.1" },
  { id: "cmd", name: "Command Prompt" },
  { id: "wsl", name: "WSL (Ubuntu)", version: "2.0" },
];

async function renderSettingsPanel(overrides: Record<string, unknown> = {}) {
  const { default: SettingsPanel } = await import("./SettingsPanel.svelte");
  return render(SettingsPanel, {
    props: {
      onClose: vi.fn(),
      ...overrides,
    },
  });
}

/** Click a settings tab button by its label text. */
async function switchToTab(
  rendered: Awaited<ReturnType<typeof renderSettingsPanel>>,
  tabName: string,
) {
  const tabButtons = rendered.container.querySelectorAll(".settings-tab-btn");
  const target = Array.from(tabButtons).find(
    (btn) => btn.textContent?.trim() === tabName,
  ) as HTMLButtonElement | undefined;
  expect(target).toBeTruthy();
  await fireEvent.click(target!);
}

describe("SettingsPanel", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "get_skill_registration_status_cmd") return structuredClone(skillStatusFixture);
      if (command === "repair_skill_registration_cmd") return structuredClone(skillStatusFixture);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });
    vi.spyOn(window, "dispatchEvent");
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("loads settings and shows tab bar with all tabs", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_settings");
      expect(invokeMock).toHaveBeenCalledWith("get_profiles");
      expect(invokeMock).toHaveBeenCalledWith("get_skill_registration_status_cmd");
    });

    const tabButtons = rendered.container.querySelectorAll(".settings-tab-btn");
    const tabNames = Array.from(tabButtons).map((btn) => btn.textContent?.trim());
    expect(tabNames).toEqual(["Appearance", "Voice Input", "GitHub Integration", "Profiles"]);
  });

  it("shows Appearance tab content by default", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.getByText("Terminal Font Size")).toBeTruthy();
    });

    const activeTab = rendered.container.querySelector(".settings-tab-btn.active");
    expect(activeTab?.textContent?.trim()).toBe("Appearance");
  });

  it("switches tabs and shows only active content", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.getByText("Terminal Font Size")).toBeTruthy();
    });

    // Switch to Voice Input
    await switchToTab(rendered, "Voice Input");
    await waitFor(() => {
      expect(rendered.getByText("Enable Voice Input")).toBeTruthy();
      expect(rendered.queryByText("Terminal Font Size")).toBeNull();
    });

    // Switch to GitHub Integration
    await switchToTab(rendered, "GitHub Integration");
    await waitFor(() => {
      expect(rendered.container.querySelector(".skill-overview")).toBeTruthy();
      expect(rendered.queryByText("Enable Voice Input")).toBeNull();
    });

    // Switch to Profiles
    await switchToTab(rendered, "Profiles");
    await waitFor(() => {
      expect(rendered.getByText("Active Profile")).toBeTruthy();
      expect(rendered.container.querySelector(".skill-overview")).toBeNull();
    });

    // Switch back to Appearance
    await switchToTab(rendered, "Appearance");
    await waitFor(() => {
      expect(rendered.getByText("Terminal Font Size")).toBeTruthy();
      expect(rendered.queryByText("Active Profile")).toBeNull();
    });
  });

  it("repairs skill registration status", async () => {
    const degradedStatus: SkillRegistrationStatus = {
      ...skillStatusFixture,
      overall: "degraded",
      agents: skillStatusFixture.agents.map((agent) =>
        agent.agent_id === "codex"
          ? {
              ...agent,
              registered: false,
              missing_skills: ["gwt-pty-communication", "gwt-issue-spec-ops"],
            }
          : agent
      ),
      last_error_message: "Missing skills: gwt-pty-communication, gwt-issue-spec-ops",
    };
    const repairedStatus: SkillRegistrationStatus = {
      ...skillStatusFixture,
      overall: "ok",
      agents: skillStatusFixture.agents.map((agent) => ({
        ...agent,
        registered: true,
        missing_skills: [],
      })),
      last_error_message: null,
    };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "get_skill_registration_status_cmd") return structuredClone(degradedStatus);
      if (command === "repair_skill_registration_cmd") return structuredClone(repairedStatus);
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "GitHub Integration");

    await waitFor(() => {
      expect(rendered.getByText("Overall: DEGRADED")).toBeTruthy();
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Repair Skill Registration" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("repair_skill_registration_cmd");
      expect(rendered.getByText("Overall: OK")).toBeTruthy();
      expect(rendered.getByText("Skill registration repaired.")).toBeTruthy();
    });
  });

  it("adds a protected branch via Enter key", async () => {
    const rendered = await renderSettingsPanel();

    await rendered.findByText("Protected Branches");
    const input = rendered.getByPlaceholderText("Add branch...") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "release" } });
    await fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => {
      expect(rendered.getByText("release")).toBeTruthy();
    });
  });

  it("creates and deletes a profile", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("New Profile");
    const newProfileInput = rendered.getByPlaceholderText(
      "e.g. development"
    ) as HTMLInputElement;
    await fireEvent.input(newProfileInput, { target: { value: "staging" } });

    const createButton = rendered.getByRole("button", { name: "Create" });
    await fireEvent.click(createButton);

    await waitFor(() => {
      const options = Array.from(
        rendered.container.querySelectorAll("#profile-edit option")
      ).map((o) => o.textContent?.trim());
      expect(options).toContain("staging");
    });

    const deleteButton = rendered.getByRole("button", { name: "Delete" });
    await fireEvent.click(deleteButton);

    await waitFor(() => {
      const options = Array.from(
        rendered.container.querySelectorAll("#profile-edit option")
      ).map((o) => o.textContent?.trim());
      expect(options).not.toContain("staging");
    });
  });

  it("loads AI model options on manual refresh", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");
    expect(invokeMock.mock.calls.some(([command]) => command === "list_ai_models")).toBe(false);

    await fireEvent.click(rendered.getByRole("button", { name: "Refresh" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("list_ai_models", {
        endpoint: "https://api.openai.com/v1",
        apiKey: "test-key",
      });
    });

    const modelOptions = Array.from(
      rendered.container.querySelectorAll(".ai-model-select option")
    ).map((o) => o.textContent?.trim());
    expect(modelOptions).toContain("gpt-5");
    expect(modelOptions).toContain("gpt-4o-mini");
  });

  it("does not auto-fetch models while editing Endpoint", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");

    const endpointLabel = rendered.getByText("Endpoint");
    const endpointInput = endpointLabel.parentElement?.querySelector("input") as HTMLInputElement;
    await fireEvent.input(endpointInput, { target: { value: "https://example.local/v1" } });
    await fireEvent.input(endpointInput, { target: { value: "https://example.local/v1/" } });

    expect(invokeMock.mock.calls.some(([command]) => command === "list_ai_models")).toBe(false);
  });

  it("does not auto-fetch models while editing API Key", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");

    const apiKeyLabel = rendered.getByText("API Key");
    const apiKeyInput = apiKeyLabel.parentElement?.querySelector("input") as HTMLInputElement;
    await fireEvent.input(apiKeyInput, { target: { value: "new-key-1" } });
    await fireEvent.input(apiKeyInput, { target: { value: "new-key-2" } });

    expect(invokeMock.mock.calls.some(([command]) => command === "list_ai_models")).toBe(false);
  });

  it("saves settings and profiles", async () => {
    const onClose = vi.fn();
    const rendered = await renderSettingsPanel({ onClose });

    await rendered.findByRole("button", { name: "Save" });
    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_settings", {
        settings: expect.any(Object),
      });
      expect(invokeMock).toHaveBeenCalledWith("save_profiles", {
        config: expect.any(Object),
      });
      expect(rendered.getByText("Settings saved.")).toBeTruthy();
    });
  });

  it("edits and saves skill registration scope settings", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "GitHub Integration");

    const defaultScope = rendered.container.querySelector(
      "#skill-scope-default"
    ) as HTMLSelectElement;
    const codexScope = rendered.container.querySelector(
      "#skill-scope-codex"
    ) as HTMLSelectElement;
    const claudeScope = rendered.container.querySelector(
      "#skill-scope-claude"
    ) as HTMLSelectElement;
    const geminiScope = rendered.container.querySelector(
      "#skill-scope-gemini"
    ) as HTMLSelectElement;

    await fireEvent.change(defaultScope, { target: { value: "project" } });
    await fireEvent.change(codexScope, { target: { value: "user" } });
    await fireEvent.change(claudeScope, { target: { value: "project" } });
    await fireEvent.change(geminiScope, { target: { value: "local" } });

    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_settings", {
        settings: expect.objectContaining({
          agent_skill_registration_default_scope: "project",
          agent_skill_registration_codex_scope: "user",
          agent_skill_registration_claude_scope: "project",
          agent_skill_registration_gemini_scope: "local",
        }),
      });
    });
  });

  it("rejects scope override save when default scope is missing", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "GitHub Integration");

    const defaultScope = rendered.container.querySelector(
      "#skill-scope-default"
    ) as HTMLSelectElement;
    const codexScope = rendered.container.querySelector(
      "#skill-scope-codex"
    ) as HTMLSelectElement;

    await fireEvent.change(defaultScope, { target: { value: "" } });
    await fireEvent.change(codexScope, { target: { value: "user" } });
    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(
        rendered.getByText("Choose default skill registration scope before setting agent overrides.")
      ).toBeTruthy();
    });
    expect(invokeMock).not.toHaveBeenCalledWith("save_settings", {
      settings: expect.objectContaining({
        agent_skill_registration_codex_scope: "user",
      }),
    });
  });

  it("shows load failure when settings retrieval fails", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") throw new Error("settings failed");
      if (command === "get_profiles") return structuredClone(profilesFixture);
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.getByText("Failed to load settings: settings failed")).toBeTruthy();
    });
  });

  it("shows save failure when save command fails", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "save_settings") throw new Error("save failed");
      return null;
    });

    const rendered = await renderSettingsPanel();

    await rendered.findByRole("button", { name: "Save" });
    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(
        rendered.getByText("Failed to save settings: save failed")
      ).toBeTruthy();
    });
  });

  it("shows validation error for invalid profile name", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("New Profile");
    const newProfileInput = rendered.getByPlaceholderText(
      "e.g. development"
    ) as HTMLInputElement;
    await fireEvent.input(newProfileInput, { target: { value: "Invalid Name" } });
    await fireEvent.click(rendered.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(
        rendered.getByText("Profile name must be lowercase letters, numbers, or hyphens.")
      ).toBeTruthy();
    });
  });

  it("shows validation error when creating duplicate profile", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("New Profile");
    const newProfileInput = rendered.getByPlaceholderText(
      "e.g. development"
    ) as HTMLInputElement;
    await fireEvent.input(newProfileInput, { target: { value: "default" } });
    await fireEvent.click(rendered.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(rendered.getByText("Profile already exists.")).toBeTruthy();
    });
  });

  it("removes protected branch from tags", async () => {
    const rendered = await renderSettingsPanel();

    await rendered.findByText("Protected Branches");
    const beforeCount = rendered.container.querySelectorAll(".branch-tag").length;
    const targetTag = Array.from(rendered.container.querySelectorAll(".branch-tag")).find((tag) =>
      (tag.textContent ?? "").includes("develop")
    ) as HTMLElement;
    const removeBtn = targetTag.querySelector(".tag-remove") as HTMLButtonElement;
    await fireEvent.click(removeBtn);

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".branch-tag").length).toBe(beforeCount - 1);
    });
  });

  it("always shows AI settings fields in Profiles", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("AI Settings (per profile)");
    expect(rendered.getByText("Endpoint")).toBeTruthy();
    expect(rendered.getByText("API Key")).toBeTruthy();
    expect(rendered.getByText("Model")).toBeTruthy();
    expect(rendered.getByText("Session Summary")).toBeTruthy();
    expect(rendered.getByText("Profile Language")).toBeTruthy();
    expect(rendered.container.querySelector("#ai-enabled")).toBeNull();

    const apiKeyLabel = rendered.getByText("API Key");
    const apiKeyInput = apiKeyLabel.parentElement?.querySelector("input") as HTMLInputElement;
    expect(apiKeyInput.type).toBe("password");
  });

  it("adjusts font sizes and clamps numeric inputs", async () => {
    const rendered = await renderSettingsPanel();

    await rendered.findByText("Appearance");
    const controls = Array.from(rendered.container.querySelectorAll(".font-size-control"));
    const terminalControl = controls[0] as HTMLElement;
    const uiControl = controls[1] as HTMLElement;
    const terminalInput = terminalControl.querySelector("input") as HTMLInputElement;
    const uiInput = uiControl.querySelector("input") as HTMLInputElement;
    const terminalButtons = terminalControl.querySelectorAll("button");
    const uiButtons = uiControl.querySelectorAll("button");

    await fireEvent.input(terminalInput, { target: { value: "100" } });
    await fireEvent.change(terminalInput);
    await waitFor(() => {
      expect(terminalInput.value).toBe("24");
    });
    expect((terminalButtons[1] as HTMLButtonElement).disabled).toBe(true);

    await fireEvent.input(uiInput, { target: { value: "1" } });
    await fireEvent.change(uiInput);
    await waitFor(() => {
      expect(uiInput.value).toBe("8");
    });
    expect((uiButtons[0] as HTMLButtonElement).disabled).toBe(true);

    await fireEvent.click(uiButtons[1] as HTMLButtonElement);
    await waitFor(() => {
      expect(uiInput.value).toBe("9");
    });

    await fireEvent.click(terminalButtons[0] as HTMLButtonElement);
    await waitFor(() => {
      expect(terminalInput.value).toBe("23");
    });
  });

  it("updates font family selects and includes them in saved settings", async () => {
    const rendered = await renderSettingsPanel();

    await rendered.findByText("Appearance");
    const terminalFontFamily = rendered.container.querySelector(
      "#terminal-font-family"
    ) as HTMLSelectElement;
    const uiFontFamily = rendered.container.querySelector(
      "#ui-font-family"
    ) as HTMLSelectElement;

    await fireEvent.change(terminalFontFamily, {
      target: { value: '"Cascadia Mono", "Cascadia Code", Consolas, monospace' },
    });
    await fireEvent.change(uiFontFamily, {
      target: {
        value: '"Inter", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
      },
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_settings", {
        settings: expect.objectContaining({
          ui_font_family:
            '"Inter", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
          terminal_font_family:
            '"Cascadia Mono", "Cascadia Code", Consolas, monospace',
        }),
      });
    });
  });

  it("keeps non-preset font families when loading and saving settings", async () => {
    const customUiFamily = '"IBM Plex Sans", system-ui, sans-serif';
    const customTerminalFamily = '"Iosevka Term", monospace';
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") {
        return structuredClone({
          ...settingsFixture,
          ui_font_family: customUiFamily,
          terminal_font_family: customTerminalFamily,
        });
      }
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "get_skill_registration_status_cmd") return structuredClone(skillStatusFixture);
      if (command === "repair_skill_registration_cmd") return structuredClone(skillStatusFixture);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();
    await rendered.findByText("Appearance");

    await waitFor(() => {
      expect(
        document.documentElement.style.getPropertyValue("--ui-font-family")
      ).toBe(customUiFamily);
      expect((window as any).__gwtTerminalFontFamily).toBe(customTerminalFamily);
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_settings", {
        settings: expect.objectContaining({
          ui_font_family: customUiFamily,
          terminal_font_family: customTerminalFamily,
        }),
      });
    });
  });

  it("restores font family preview when closing without save", async () => {
    const onClose = vi.fn();
    document.documentElement.style.setProperty("--ui-font-family", settingsFixture.ui_font_family);
    document.documentElement.style.setProperty(
      "--terminal-font-family",
      settingsFixture.terminal_font_family
    );
    (window as any).__gwtTerminalFontFamily = settingsFixture.terminal_font_family;

    const rendered = await renderSettingsPanel({ onClose });

    await rendered.findByText("Appearance");
    const terminalFontFamily = rendered.container.querySelector(
      "#terminal-font-family"
    ) as HTMLSelectElement;
    const uiFontFamily = rendered.container.querySelector(
      "#ui-font-family"
    ) as HTMLSelectElement;

    await fireEvent.change(terminalFontFamily, {
      target: { value: '"SF Mono", Menlo, Monaco, Consolas, monospace' },
    });
    await fireEvent.change(uiFontFamily, {
      target: {
        value:
          '"Source Sans 3", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
      },
    });

    await waitFor(() => {
      expect(
        document.documentElement.style.getPropertyValue("--ui-font-family")
      ).toBe(
        '"Source Sans 3", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif'
      );
      expect((window as any).__gwtTerminalFontFamily).toBe(
        '"SF Mono", Menlo, Monaco, Consolas, monospace'
      );
    });

    const closeBtn = rendered.container.querySelector(".settings-header .close-btn") as HTMLButtonElement;
    expect(closeBtn).toBeTruthy();
    await fireEvent.click(closeBtn);

    await waitFor(() => {
      expect(
        document.documentElement.style.getPropertyValue("--ui-font-family")
      ).toBe(settingsFixture.ui_font_family);
      expect((window as any).__gwtTerminalFontFamily).toBe(
        settingsFixture.terminal_font_family
      );
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("toggles env Add button disabled state based on KEY input", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("Environment Variables");
    const addRow = rendered.container.querySelector(".env-add-row") as HTMLElement;
    const keyInput = addRow.querySelector(".env-key-input") as HTMLInputElement;
    const addButton = addRow.querySelector("button") as HTMLButtonElement;

    // Initial state: disabled (empty KEY)
    expect(addButton.disabled).toBe(true);

    // After entering KEY: enabled
    await fireEvent.input(keyInput, { target: { value: "MY_VAR" } });
    await waitFor(() => {
      expect(addButton.disabled).toBe(false);
    });

    // Whitespace-only KEY: disabled
    await fireEvent.input(keyInput, { target: { value: "   " } });
    await waitFor(() => {
      expect(addButton.disabled).toBe(true);
    });

    // Re-enter KEY: enabled again
    await fireEvent.input(keyInput, { target: { value: "ANOTHER_VAR" } });
    await waitFor(() => {
      expect(addButton.disabled).toBe(false);
    });
  });

  it("adds, edits, and removes environment variables", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("Environment Variables");
    const addRow = rendered.container.querySelector(".env-add-row") as HTMLElement;
    const keyInput = addRow.querySelector(".env-key-input") as HTMLInputElement;
    const valueInput = addRow.querySelector(".env-value-input") as HTMLInputElement;
    const addButton = addRow.querySelector("button") as HTMLButtonElement;

    await fireEvent.input(keyInput, { target: { value: "NEW_KEY" } });
    await fireEvent.input(valueInput, { target: { value: "v1" } });
    await fireEvent.click(addButton);

    const row = await waitFor(() => {
      const target = Array.from(rendered.container.querySelectorAll(".env-row")).find((node) =>
        (node.textContent ?? "").includes("NEW_KEY")
      ) as HTMLElement | undefined;
      expect(target).toBeTruthy();
      return target as HTMLElement;
    });
    const rowValueInput = row.querySelector(".env-value") as HTMLInputElement;
    await fireEvent.input(rowValueInput, { target: { value: "v2" } });
    await waitFor(() => {
      expect(rowValueInput.value).toBe("v2");
    });

    const removeButton = row.querySelector("button") as HTMLButtonElement;
    await fireEvent.click(removeButton);
    await waitFor(() => {
      expect(rendered.queryByText("NEW_KEY")).toBeNull();
    });

    await fireEvent.input(keyInput, { target: { value: "" } });
    expect(addButton.disabled).toBe(true);
  });

  it("switches and deletes active profile with fallback", async () => {
    const twoProfiles = structuredClone(profilesFixture);
    twoProfiles.profiles.dev = {
      name: "dev",
      description: "",
      env: {},
      disabled_env: [],
      ai_enabled: true,
      ai: {
        endpoint: "https://api.openai.com/v1",
        api_key: "dev-key",
        model: "gpt-4o-mini",
        language: "en",
        summary_enabled: true,
      },
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(twoProfiles);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("Active Profile");

    const activeProfile = rendered.container.querySelector("#active-profile") as HTMLSelectElement;
    await fireEvent.change(activeProfile, { target: { value: "dev" } });
    await waitFor(() => {
      expect(activeProfile.value).toBe("dev");
    });

    const profileEdit = rendered.container.querySelector("#profile-edit") as HTMLSelectElement;
    await fireEvent.change(profileEdit, { target: { value: "dev" } });
    await fireEvent.click(rendered.getByRole("button", { name: "Delete" }));

    await waitFor(() => {
      const editOptions = Array.from(profileEdit.options).map((opt) => opt.value);
      expect(editOptions).not.toContain("dev");
      expect(activeProfile.value).toBe("default");
    });
  });

  it("shows AI model fetch error and recovers on refresh", async () => {
    let listCount = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") {
        listCount += 1;
        if (listCount === 1) throw new Error("network down");
        return [{ id: "gpt-5.1" }];
      }
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");

    await fireEvent.click(rendered.getByRole("button", { name: "Refresh" }));
    await waitFor(() => {
      expect(rendered.getByText("Failed to load models: network down")).toBeTruthy();
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Refresh" }));
    await waitFor(() => {
      expect(rendered.queryByText("Failed to load models: network down")).toBeNull();
    });
    await waitFor(() => {
      const modelOptions = Array.from(
        rendered.container.querySelectorAll(".ai-model-select option")
      ).map((o) => o.textContent?.trim());
      expect(modelOptions).toContain("gpt-5.1");
    });
  });

  it("shows missing-current-model warning when current model is not returned", async () => {
    const modelMissingProfiles = structuredClone(profilesFixture);
    modelMissingProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "test-key",
      model: "custom-model",
      language: "en",
      summary_enabled: true,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(modelMissingProfiles);
      if (command === "list_ai_models") return [{ id: "gpt-4o-mini" }];
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");
    await fireEvent.click(rendered.getByRole("button", { name: "Refresh" }));

    await waitFor(() => {
      expect(rendered.getByText("Current model is not listed in /v1/models.")).toBeTruthy();
    });
  });

  it("shows no-models hint when API returns empty model list", async () => {
    const emptyModelProfiles = structuredClone(profilesFixture);
    emptyModelProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "test-key",
      model: "",
      language: "en",
      summary_enabled: true,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(emptyModelProfiles);
      if (command === "list_ai_models") return [];
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");
    await fireEvent.click(rendered.getByRole("button", { name: "Refresh" }));

    await waitFor(() => {
      expect(rendered.getByText("No models returned from /v1/models.")).toBeTruthy();
    });
  });

  it("normalizes voice input defaults and persists updated values", async () => {
    const invalidVoiceSettings = structuredClone(settingsFixture);
    invalidVoiceSettings.voice_input = {
      enabled: true,
      hotkey: "",
      language: "xx" as "auto",
      model: "",
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(invalidVoiceSettings);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Voice Input");

    const voiceEnabled = rendered.container.querySelector("#voice-input-enabled") as HTMLInputElement;
    const voiceHotkey = rendered.container.querySelector("#voice-hotkey") as HTMLInputElement;
    const voiceLanguage = rendered.container.querySelector("#voice-language") as HTMLSelectElement;
    const voiceModel = rendered.container.querySelector("#voice-model") as HTMLInputElement;

    expect(voiceEnabled.checked).toBe(true);
    expect(voiceHotkey.value).toBe("Mod+Shift+M");
    expect(voiceLanguage.value).toBe("auto");
    expect(voiceModel.value).toBe("base");

    await fireEvent.input(voiceHotkey, { target: { value: "  Ctrl+M  " } });
    await fireEvent.change(voiceLanguage, { target: { value: "ja" } });
    await fireEvent.input(voiceModel, { target: { value: "small" } });
    await fireEvent.click(voiceEnabled);

    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_settings", {
        settings: expect.objectContaining({
          voice_input: expect.objectContaining({
            enabled: false,
            hotkey: "Ctrl+M",
            language: "ja",
            model: "small",
          }),
        }),
      });
    });
  });

  it("shows AI fields when profile AI config is null", async () => {
    const noAiProfiles = structuredClone(profilesFixture);
    noAiProfiles.profiles.default.ai_enabled = false;
    noAiProfiles.profiles.default.ai = null;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(noAiProfiles);
      if (command === "list_ai_models") return [{ id: "gpt-4o-mini" }];
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("AI Settings (per profile)");
    expect(rendered.getByText("Endpoint")).toBeTruthy();

    const endpointInput = rendered.container.querySelector(".ai-field input") as HTMLInputElement;
    expect(endpointInput.value).toBe("");
  });

  it("formats string load errors with toErrorMessage", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") throw "string failure";
      if (command === "get_profiles") return structuredClone(profilesFixture);
      return null;
    });

    const rendered = await renderSettingsPanel();
    await waitFor(() => {
      expect(rendered.getByText("Failed to load settings: string failure")).toBeTruthy();
    });
  });

  it("calls onClose when Close button is clicked", async () => {
    const onClose = vi.fn();
    const rendered = await renderSettingsPanel({ onClose });

    await rendered.findByRole("button", { name: "Close" });
    await fireEvent.click(rendered.getByRole("button", { name: "Close" }));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("hides Terminal tab when no shells are available (macOS/Linux)", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(4);
    });

    const tabButtons = rendered.container.querySelectorAll(".settings-tab-btn");
    const tabNames = Array.from(tabButtons).map((btn) => btn.textContent?.trim());
    expect(tabNames).not.toContain("Terminal");
  });

  it("shows Terminal tab when shells are available (Windows)", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "get_mcp_registration_status_cmd") return structuredClone(skillStatusFixture);
      if (command === "get_available_shells") return structuredClone(shellsFixture);
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      const tabButtons = rendered.container.querySelectorAll(".settings-tab-btn");
      const tabNames = Array.from(tabButtons).map((btn) => btn.textContent?.trim());
      expect(tabNames).toContain("Terminal");
    });
  });

  it("shows shell dropdown with version subtext in Terminal tab", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "get_mcp_registration_status_cmd") return structuredClone(skillStatusFixture);
      if (command === "get_available_shells") return structuredClone(shellsFixture);
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      const tabButtons = rendered.container.querySelectorAll(".settings-tab-btn");
      const tabNames = Array.from(tabButtons).map((btn) => btn.textContent?.trim());
      expect(tabNames).toContain("Terminal");
    });

    await switchToTab(rendered, "Terminal");

    await waitFor(() => {
      expect(rendered.getByText("Default Shell")).toBeTruthy();
    });

    const select = rendered.container.querySelector("#default-shell") as HTMLSelectElement;
    expect(select).toBeTruthy();

    const options = Array.from(select.options).map((o) => o.textContent?.trim());
    expect(options).toContain("System Default");
    expect(options).toContain("PowerShell (7.4.1)");
    expect(options).toContain("Command Prompt");
    expect(options).toContain("WSL (Ubuntu) (2.0)");
  });

  it("saves selected shell via Terminal tab", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "get_mcp_registration_status_cmd") return structuredClone(skillStatusFixture);
      if (command === "get_available_shells") return structuredClone(shellsFixture);
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      const tabButtons = rendered.container.querySelectorAll(".settings-tab-btn");
      const tabNames = Array.from(tabButtons).map((btn) => btn.textContent?.trim());
      expect(tabNames).toContain("Terminal");
    });

    await switchToTab(rendered, "Terminal");

    await waitFor(() => {
      expect(rendered.container.querySelector("#default-shell")).toBeTruthy();
    });

    const select = rendered.container.querySelector("#default-shell") as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: "wsl" } });

    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_settings", {
        settings: expect.objectContaining({
          default_shell: "wsl",
        }),
      });
    });
  });
});
