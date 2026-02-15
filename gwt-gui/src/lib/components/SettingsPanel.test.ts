import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

import type { ProfilesConfig, SettingsData } from "../types";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  default: {
    invoke: invokeMock,
  },
}));

const settingsFixture: SettingsData = {
  protected_branches: ["main", "develop"],
  default_base_branch: "main",
  worktree_root: "/tmp/worktrees",
  debug: false,
  log_retention_days: 30,
  agent_default: "codex",
  agent_auto_install_deps: true,
  docker_force_host: true,
  ui_font_size: 13,
  terminal_font_size: 13,
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
        summary_enabled: true,
      },
    },
  },
};

async function renderSettingsPanel(overrides: Record<string, unknown> = {}) {
  const { default: SettingsPanel } = await import("./SettingsPanel.svelte");
  return render(SettingsPanel, {
    props: {
      onClose: vi.fn(),
      ...overrides,
    },
  });
}

describe("SettingsPanel", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });
    vi.spyOn(window, "dispatchEvent");
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("loads settings and profiles on mount", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_settings");
      expect(invokeMock).toHaveBeenCalledWith("get_profiles");
      expect(rendered.getByText("Appearance")).toBeTruthy();
      expect(rendered.getByText("Voice Input")).toBeTruthy();
      expect(rendered.getByText("Profiles")).toBeTruthy();
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

  it("loads AI model options from invoke", async () => {
    const rendered = await renderSettingsPanel();

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

  it("toggles AI settings section by checkbox", async () => {
    const rendered = await renderSettingsPanel();

    await rendered.findByText("AI Settings (per profile)");
    const aiEnabled = rendered.container.querySelector("#ai-enabled") as HTMLInputElement;
    expect(aiEnabled.checked).toBe(true);
    expect(rendered.getByText("Endpoint")).toBeTruthy();

    await fireEvent.click(aiEnabled);
    await waitFor(() => {
      expect(rendered.queryByText("Endpoint")).toBeNull();
    });

    await fireEvent.click(aiEnabled);
    await waitFor(() => {
      expect(rendered.getByText("Endpoint")).toBeTruthy();
    });
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

  it("adds, edits, and removes environment variables", async () => {
    const rendered = await renderSettingsPanel();

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
      expect(rendered.getByText("Current model is not listed in /v1/models.")).toBeTruthy();
    });
  });

  it("shows no-models hint when API returns empty model list", async () => {
    const emptyModelProfiles = structuredClone(profilesFixture);
    emptyModelProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "test-key",
      model: "",
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
    await rendered.findByText("Voice Input");

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

  it("enables AI settings from null profile config", async () => {
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
    await rendered.findByText("AI Settings (per profile)");
    expect(rendered.queryByText("Endpoint")).toBeNull();

    const aiEnabled = rendered.container.querySelector("#ai-enabled") as HTMLInputElement;
    expect(aiEnabled.checked).toBe(false);
    await fireEvent.click(aiEnabled);

    await waitFor(() => {
      expect(rendered.getByText("Endpoint")).toBeTruthy();
    });
    const endpointInput = rendered.container.querySelector(".ai-field input") as HTMLInputElement;
    expect(endpointInput.value).toBe("https://api.openai.com/v1");
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
});
