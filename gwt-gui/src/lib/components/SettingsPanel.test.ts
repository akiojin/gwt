import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

import type {
  ProfilesConfig,
  SettingsData,
  ShellInfo,
} from "../types";


const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
}));

const tauriCoreInvokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: tauriCoreInvokeMock,
}));

const settingsFixture: SettingsData = {
  protected_branches: ["main", "develop"],
  default_base_branch: "main",
  worktree_root: "/tmp/worktrees",
  debug: false,
  log_retention_days: 30,
  agent_default: "codex",
  agent_auto_install_deps: true,
  agent_skill_registration_enabled: true,
  docker_force_host: true,
  ui_font_size: 13,
  terminal_font_size: 13,
  ui_font_family: 'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
  terminal_font_family: '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace',
  app_language: "auto",
  voice_input: {
    enabled: false,
    engine: "qwen3-asr",
    hotkey: "Mod+Shift+M",
    ptt_hotkey: "Mod+Shift+Space",
    language: "auto",
    quality: "balanced",
    model: "Qwen/Qwen3-ASR-1.7B",
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
      },
    },
  },
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

async function pasteText(input: HTMLInputElement, text: string) {
  input.focus();
  const cursor = input.value.length;
  input.setSelectionRange(cursor, cursor);
  const pasteEvent = new Event("paste", {
    bubbles: true,
    cancelable: true,
  }) as ClipboardEvent;
  Object.defineProperty(pasteEvent, "clipboardData", {
    value: {
      getData: (type: string) => (type === "text/plain" ? text : ""),
    },
    configurable: true,
  });
  await fireEvent(input, pasteEvent);
}

describe("SettingsPanel", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
    tauriCoreInvokeMock.mockReset();
    tauriCoreInvokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: true, reason: null };
      }
      return null;
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
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
      const tabButtons = rendered.container.querySelectorAll(".settings-tab-btn");
      const tabNames = Array.from(tabButtons).map((btn) => btn.textContent?.trim());
      expect(tabNames).toEqual(["General", "Profiles", "Terminal", "Voice Input", "Agent"]);
    });
  });

  it("shows General tab content by default", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.getByText("UI font size")).toBeTruthy();
    });

    const activeTab = rendered.container.querySelector(".settings-tab-btn.active");
    expect(activeTab?.textContent?.trim()).toBe("General");
  });

  it("switches tabs and shows only active content", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.getByText("UI font size")).toBeTruthy();
    });

    // Switch to Voice Input
    await switchToTab(rendered, "Voice Input");
    await waitFor(() => {
      expect(rendered.getByText("Enable Voice Input")).toBeTruthy();
      expect(rendered.queryByText("UI font size")).toBeNull();
    });

    // Switch to Profiles
    await switchToTab(rendered, "Profiles");
    await waitFor(() => {
      expect(rendered.container.querySelector(".profile-select")).toBeTruthy();
    });

    // Switch back to General
    await switchToTab(rendered, "General");
    await waitFor(() => {
      expect(rendered.getByText("UI font size")).toBeTruthy();
      expect(rendered.queryByText("Environment Variables")).toBeNull();
    });
  });

  it("adds a protected branch via Enter key", async () => {
    const rendered = await renderSettingsPanel();

    await rendered.findByText("Protected branches");
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
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    // Click "+ New" to open CreateProfileDialog
    const newBtn = rendered.getByRole("button", { name: "+ New" });
    await fireEvent.click(newBtn);

    // Fill in the dialog
    await waitFor(() => {
      expect(rendered.container.querySelector(".modal-overlay")).toBeTruthy();
    });
    const dialogInput = rendered.container.querySelector("#profile-name-input") as HTMLInputElement;
    await fireEvent.input(dialogInput, { target: { value: "staging" } });
    const createButton = rendered.container.querySelector(".modal-overlay .btn-save") as HTMLButtonElement;
    await fireEvent.click(createButton);

    await waitFor(() => {
      const activeProfile = rendered.container.querySelector(".profile-select") as HTMLSelectElement;
      const options = Array.from(
        rendered.container.querySelectorAll(".profile-select option")
      ).map((o) => o.textContent?.trim());
      expect(options).toContain("staging");
      expect(activeProfile.value).toBe("staging");
    });

    // Click "Delete" to open ConfirmDialog
    const deleteButton = rendered.getByRole("button", { name: "Delete" });
    await fireEvent.click(deleteButton);

    // Confirm deletion in dialog
    await waitFor(() => {
      expect(rendered.container.querySelector(".modal-overlay")).toBeTruthy();
    });
    const cancelBtn = rendered.container.querySelector(".modal-overlay .btn-cancel") as HTMLButtonElement;
    await waitFor(() => {
      expect(document.activeElement).toBe(cancelBtn);
    });
    const confirmBtn = rendered.container.querySelector(".modal-overlay .btn-danger") as HTMLButtonElement;
    await fireEvent.click(confirmBtn);

    await waitFor(() => {
      const activeProfile = rendered.container.querySelector(".profile-select") as HTMLSelectElement;
      const options = Array.from(
        rendered.container.querySelectorAll(".profile-select option")
      ).map((o) => o.textContent?.trim());
      expect(options).not.toContain("staging");
      expect(activeProfile.value).toBe("default");
    });
  });

  it("disables deleting the default profile", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    const activeProfile = rendered.container.querySelector(".profile-select") as HTMLSelectElement;
    await waitFor(() => {
      expect(activeProfile).toBeTruthy();
    });
    const deleteButton = rendered.getByRole("button", {
      name: "Delete",
    }) as HTMLButtonElement;

    expect(activeProfile.value).toBe("default");
    expect(deleteButton.disabled).toBe(true);

    await fireEvent.click(deleteButton);

    await waitFor(() => {
      const options = Array.from(activeProfile.options).map((opt) => opt.value);
      expect(options).toContain("default");
      expect(activeProfile.value).toBe("default");
    });
  });

  it("allows deleting a malformed default-like profile key", async () => {
    const malformedProfiles = structuredClone(profilesFixture);
    malformedProfiles.active = "default ";
    malformedProfiles.profiles["default "] = {
      name: "default ",
      description: "",
      env: { BROKEN: "1" },
      disabled_env: [],
      ai_enabled: false,
      ai: null,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(malformedProfiles);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    const activeProfile = rendered.container.querySelector(".profile-select") as HTMLSelectElement;
    await waitFor(() => {
      expect(activeProfile).toBeTruthy();
    });
    const deleteButton = rendered.getByRole("button", {
      name: "Delete",
    }) as HTMLButtonElement;

    expect(activeProfile.value).toBe("default ");
    expect(deleteButton.disabled).toBe(false);

    await fireEvent.click(deleteButton);

    // Confirm deletion in dialog
    await waitFor(() => {
      expect(rendered.container.querySelector(".modal-overlay")).toBeTruthy();
    });
    const confirmBtn = rendered.container.querySelector(".modal-overlay .btn-danger") as HTMLButtonElement;
    await fireEvent.click(confirmBtn);

    await waitFor(() => {
      const options = Array.from(activeProfile.options).map((opt) => opt.value);
      expect(options).not.toContain("default ");
      expect(activeProfile.value).toBe("default");
    });
  });

  it("re-disables deleting after switching back to the default profile", async () => {
    const twoProfiles = structuredClone(profilesFixture);
    twoProfiles.profiles.dev = {
      name: "dev",
      description: "",
      env: { DEV_KEY: "dev-value" },
      disabled_env: [],
      ai_enabled: true,
      ai: {
        endpoint: "https://api.openai.com/v1",
        api_key: "dev-key",
        model: "gpt-4o-mini",
        language: "en",
      },
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(twoProfiles);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    const activeProfile = rendered.container.querySelector(".profile-select") as HTMLSelectElement;
    await waitFor(() => {
      expect(activeProfile).toBeTruthy();
    });
    const deleteButton = rendered.getByRole("button", {
      name: "Delete",
    }) as HTMLButtonElement;

    expect(activeProfile.value).toBe("default");
    expect(deleteButton.disabled).toBe(true);

    await fireEvent.change(activeProfile, { target: { value: "dev" } });
    await waitFor(() => {
      expect(activeProfile.value).toBe("dev");
      expect(deleteButton.disabled).toBe(false);
    });

    await fireEvent.change(activeProfile, { target: { value: "default" } });
    await waitFor(() => {
      expect(activeProfile.value).toBe("default");
      expect(deleteButton.disabled).toBe(true);
    });
  });

  it("loads AI model options on manual refresh", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
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
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
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
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    const apiKeyLabel = rendered.getByText("API key");
    const apiKeyInput = apiKeyLabel.parentElement?.querySelector("input") as HTMLInputElement;
    await fireEvent.input(apiKeyInput, { target: { value: "new-key-1" } });
    await fireEvent.input(apiKeyInput, { target: { value: "new-key-2" } });

    expect(invokeMock.mock.calls.some(([command]) => command === "list_ai_models")).toBe(false);
  });

  it("saves settings and profiles with correct AI payload", async () => {
    const onClose = vi.fn();
    const rendered = await renderSettingsPanel({ onClose });

    await rendered.findByRole("button", { name: "Save" });
    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_settings", {
        settings: expect.any(Object),
      });
      const saveCall = invokeMock.mock.calls.find((args: unknown[]) => args[0] === "save_profiles");
      expect(saveCall).toBeTruthy();
      const savedConfig = saveCall![1].config as ProfilesConfig;
      expect(savedConfig.profiles.default.ai?.api_key).toBe("test-key");
      expect(savedConfig.profiles.default.ai?.model).toBe("gpt-4o-mini");
      expect(rendered.getByText("Settings saved.")).toBeTruthy();
    });
  });

  it("clears save message after timeout", async () => {
    vi.useFakeTimers();
    try {
      const rendered = await renderSettingsPanel();
      await rendered.findByRole("button", { name: "Save" });

      await fireEvent.click(rendered.getByRole("button", { name: "Save" }));
      await waitFor(() => {
        expect(rendered.getByText("Settings saved.")).toBeTruthy();
      });

      await vi.advanceTimersByTimeAsync(2000);
      await waitFor(() => {
        expect(rendered.queryByText("Settings saved.")).toBeNull();
      });
    } finally {
      vi.useRealTimers();
    }
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
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    // Click "+ New" to open CreateProfileDialog
    const newBtn = rendered.getByRole("button", { name: "+ New" });
    await fireEvent.click(newBtn);

    await waitFor(() => {
      expect(rendered.container.querySelector(".modal-overlay")).toBeTruthy();
    });
    const dialogInput = rendered.container.querySelector("#profile-name-input") as HTMLInputElement;
    await fireEvent.input(dialogInput, { target: { value: "Invalid Name" } });

    // The Create button should be disabled for invalid names in CreateProfileDialog
    const createButton = rendered.container.querySelector(".modal-overlay .btn-save") as HTMLButtonElement;
    expect(createButton.disabled).toBe(true);
  });

  it("shows validation error when creating duplicate profile", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    // Click "+ New" to open CreateProfileDialog
    const newBtn = rendered.getByRole("button", { name: "+ New" });
    await fireEvent.click(newBtn);

    await waitFor(() => {
      expect(rendered.container.querySelector(".modal-overlay")).toBeTruthy();
    });
    const dialogInput = rendered.container.querySelector("#profile-name-input") as HTMLInputElement;
    await fireEvent.input(dialogInput, { target: { value: "default" } });
    const createButton = rendered.container.querySelector(".modal-overlay .btn-save") as HTMLButtonElement;
    await fireEvent.click(createButton);

    await waitFor(() => {
      expect(rendered.getByText("Profile already exists.")).toBeTruthy();
    });
    expect(dialogInput.value).toBe("default");
  });

  it("removes protected branch from tags", async () => {
    const rendered = await renderSettingsPanel();

    await rendered.findByText("Protected branches");
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
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("AI Configuration");
    expect(rendered.getByText("Endpoint")).toBeTruthy();
    expect(rendered.getByText("API key")).toBeTruthy();
    expect(rendered.getByText("Model")).toBeTruthy();
    expect(rendered.getByText("AI response language")).toBeTruthy();
    expect(rendered.container.querySelector("#ai-enabled")).toBeNull();

    const apiKeyLabel = rendered.getByText("API key");
    const apiKeyInput = apiKeyLabel.parentElement?.querySelector("input") as HTMLInputElement;
    expect(apiKeyInput.type).toBe("text");
  });

  it("adjusts font sizes and clamps numeric inputs", async () => {
    const rendered = await renderSettingsPanel();

    // General tab has UI font size
    await rendered.findByText("General");
    const uiControls = Array.from(rendered.container.querySelectorAll(".font-size-control"));
    const uiControl = uiControls[0] as HTMLElement;
    const uiInput = uiControl.querySelector("input") as HTMLInputElement;
    const uiButtons = uiControl.querySelectorAll("button");

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

    // Terminal tab has terminal font size
    await switchToTab(rendered, "Terminal");
    await waitFor(() => {
      expect(rendered.getByText("Terminal font size")).toBeTruthy();
    });
    const terminalControls = Array.from(rendered.container.querySelectorAll(".font-size-control"));
    const terminalControl = terminalControls[0] as HTMLElement;
    const terminalInput = terminalControl.querySelector("input") as HTMLInputElement;
    const terminalButtons = terminalControl.querySelectorAll("button");

    await fireEvent.input(terminalInput, { target: { value: "100" } });
    await fireEvent.change(terminalInput);
    await waitFor(() => {
      expect(terminalInput.value).toBe("24");
    });
    expect((terminalButtons[1] as HTMLButtonElement).disabled).toBe(true);

    await fireEvent.click(terminalButtons[0] as HTMLButtonElement);
    await waitFor(() => {
      expect(terminalInput.value).toBe("23");
    });
  });

  it("updates font family selects and includes them in saved settings", async () => {
    const rendered = await renderSettingsPanel();

    await rendered.findByText("General");
    const uiFontFamily = rendered.container.querySelector(
      "#ui-font-family"
    ) as HTMLSelectElement;

    await fireEvent.change(uiFontFamily, {
      target: {
        value: '"Inter", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
      },
    });

    // Terminal font family is now in Terminal tab
    await switchToTab(rendered, "Terminal");
    await waitFor(() => {
      expect(rendered.container.querySelector("#terminal-font-family")).toBeTruthy();
    });
    const terminalFontFamily = rendered.container.querySelector(
      "#terminal-font-family"
    ) as HTMLSelectElement;
    await fireEvent.change(terminalFontFamily, {
      target: { value: '"Cascadia Mono", "Cascadia Code", Consolas, monospace' },
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

  it("updates app language and persists it to saved settings", async () => {
    const rendered = await renderSettingsPanel();

    await rendered.findByText("General");
    const appLanguage = rendered.container.querySelector("#app-language") as HTMLSelectElement;
    expect(appLanguage).toBeTruthy();

    await fireEvent.change(appLanguage, { target: { value: "ja" } });
    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_settings", {
        settings: expect.objectContaining({
          app_language: "ja",
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
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();
    await rendered.findByText("General");

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

    await rendered.findByText("General");
    const uiFontFamily = rendered.container.querySelector(
      "#ui-font-family"
    ) as HTMLSelectElement;

    await fireEvent.change(uiFontFamily, {
      target: {
        value:
          '"Source Sans 3", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
      },
    });

    // Terminal font family is now in Terminal tab
    await switchToTab(rendered, "Terminal");
    await waitFor(() => {
      expect(rendered.container.querySelector("#terminal-font-family")).toBeTruthy();
    });
    const terminalFontFamily = rendered.container.querySelector(
      "#terminal-font-family"
    ) as HTMLSelectElement;
    await fireEvent.change(terminalFontFamily, {
      target: { value: '"SF Mono", Menlo, Monaco, Consolas, monospace' },
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
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
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
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
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
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    const activeProfile = rendered.container.querySelector(".profile-select") as HTMLSelectElement;
    await waitFor(() => {
      expect(activeProfile).toBeTruthy();
    });
    await fireEvent.change(activeProfile, { target: { value: "dev" } });
    await waitFor(() => {
      expect(activeProfile.value).toBe("dev");
    });

    await fireEvent.click(rendered.getByRole("button", { name: "Delete" }));

    // Confirm deletion in dialog
    await waitFor(() => {
      expect(rendered.container.querySelector(".modal-overlay")).toBeTruthy();
    });
    const confirmBtn = rendered.container.querySelector(".modal-overlay .btn-danger") as HTMLButtonElement;
    await fireEvent.click(confirmBtn);

    await waitFor(() => {
      const activeOptions = Array.from(activeProfile.options).map((opt) => opt.value);
      expect(activeOptions).not.toContain("dev");
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
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
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
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(modelMissingProfiles);
      if (command === "list_ai_models") return [{ id: "gpt-4o-mini" }];
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
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
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(emptyModelProfiles);
      if (command === "list_ai_models") return [];
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
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
      engine: "whisper",
      hotkey: "",
      ptt_hotkey: "",
      language: "xx" as "auto",
      quality: "bad" as "balanced",
      model: "",
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(invalidVoiceSettings);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "get_voice_capability") {
        return { available: true, reason: null };
      }
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Voice Input");

    const voiceEnabled = rendered.container.querySelector("#voice-input-enabled") as HTMLInputElement;
    const voiceHotkey = rendered.container.querySelector("#voice-hotkey") as HTMLInputElement;
    const voicePttHotkey = rendered.container.querySelector("#voice-ptt-hotkey") as HTMLInputElement;
    const voiceLanguage = rendered.container.querySelector("#voice-language") as HTMLSelectElement;
    const voiceQuality = rendered.container.querySelector("#voice-quality") as HTMLSelectElement;
    const voiceModel = rendered.container.querySelector("#voice-model") as HTMLInputElement;

    expect(voiceEnabled.checked).toBe(true);
    expect(voiceHotkey.value).toBe("Mod+Shift+M");
    expect(voicePttHotkey.value).toBe("Mod+Shift+Space");
    expect(voiceLanguage.value).toBe("auto");
    expect(voiceQuality.value).toBe("balanced");
    expect(voiceModel.value).toBe("Qwen/Qwen3-ASR-1.7B");

    await fireEvent.input(voiceHotkey, { target: { value: "  Ctrl+M  " } });
    await fireEvent.input(voicePttHotkey, { target: { value: "Ctrl+Shift+Space" } });
    await fireEvent.change(voiceLanguage, { target: { value: "ja" } });
    await fireEvent.change(voiceQuality, { target: { value: "fast" } });
    await fireEvent.click(voiceEnabled);

    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_settings", {
        settings: expect.objectContaining({
          voice_input: expect.objectContaining({
            enabled: false,
            engine: "qwen3-asr",
            hotkey: "Ctrl+M",
            ptt_hotkey: "Ctrl+Shift+Space",
            language: "ja",
            quality: "fast",
            model: "Qwen/Qwen3-ASR-0.6B",
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
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("AI Configuration");
    expect(rendered.getByText("Endpoint")).toBeTruthy();

    const endpointInput = rendered.container.querySelector(".settings-section-body .field input") as HTMLInputElement;
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

  it("always shows Terminal tab even when no shells are available", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    const tabButtons = rendered.container.querySelectorAll(".settings-tab-btn");
    const tabNames = Array.from(tabButtons).map((btn) => btn.textContent?.trim());
    expect(tabNames).toContain("Terminal");

    // Shell section should not appear when no shells
    await switchToTab(rendered, "Terminal");
    await waitFor(() => {
      expect(rendered.getByText("Terminal font size")).toBeTruthy();
    });
    expect(rendered.container.querySelector("#default-shell")).toBeNull();
  });

  it("shows shell dropdown in Terminal tab when shells are available", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
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
  });

  it("falls back to no shells when shell discovery fails", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "get_available_shells") throw new Error("shell list failed");
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    // Terminal tab is still shown but without shell section
    await switchToTab(rendered, "Terminal");
    await waitFor(() => {
      expect(rendered.getByText("Terminal font size")).toBeTruthy();
    });
    expect(rendered.container.querySelector("#default-shell")).toBeNull();
  });

  it("shows shell dropdown with version subtext in Terminal tab", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
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
      expect(rendered.getByText("Default shell")).toBeTruthy();
    });

    const select = rendered.container.querySelector("#default-shell") as HTMLSelectElement;
    expect(select).toBeTruthy();

    const options = Array.from(select.options).map((o) => o.textContent?.trim());
    expect(options).toContain("System Default");
    expect(options).toContain("PowerShell (7.4.1)");
    expect(options).toContain("Command Prompt");
    expect(options).toContain("WSL (Ubuntu) (2.0)");
  });

  it("keeps voice input fields enabled when voice capability is unavailable", async () => {
    const enabledVoiceSettings = structuredClone(settingsFixture);
    enabledVoiceSettings.voice_input.enabled = true;
    tauriCoreInvokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: false, reason: "GPU acceleration is not available" };
      }
      return null;
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(enabledVoiceSettings);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Voice Input");

    await waitFor(() => {
      const voiceEnabled = rendered.container.querySelector("#voice-input-enabled") as HTMLInputElement;
      expect(voiceEnabled).toBeTruthy();
      expect(voiceEnabled.disabled).toBe(false);
    });

    const voiceHotkey = rendered.container.querySelector("#voice-hotkey") as HTMLInputElement;
    const voicePttHotkey = rendered.container.querySelector("#voice-ptt-hotkey") as HTMLInputElement;
    const voiceLanguage = rendered.container.querySelector("#voice-language") as HTMLSelectElement;
    const voiceQuality = rendered.container.querySelector("#voice-quality") as HTMLSelectElement;

    expect(voiceHotkey.disabled).toBe(false);
    expect(voicePttHotkey.disabled).toBe(false);
    expect(voiceLanguage.disabled).toBe(false);
    expect(voiceQuality.disabled).toBe(false);
  });

  it("saves voice settings when capability is unavailable", async () => {
    const enabledVoiceSettings = structuredClone(settingsFixture);
    enabledVoiceSettings.voice_input.enabled = true;
    tauriCoreInvokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: false, reason: "GPU acceleration is not available" };
      }
      return null;
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(enabledVoiceSettings);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Voice Input");

    await waitFor(() => {
      expect(rendered.container.querySelector("#voice-input-enabled")).toBeTruthy();
      expect((rendered.container.querySelector("#voice-input-enabled") as HTMLInputElement).disabled).toBe(false);
    });

    const voiceHotkey = rendered.container.querySelector("#voice-hotkey") as HTMLInputElement;
    await fireEvent.input(voiceHotkey, { target: { value: "Ctrl+Shift+V" } });

    const voiceLanguage = rendered.container.querySelector("#voice-language") as HTMLSelectElement;
    await fireEvent.change(voiceLanguage, { target: { value: "ja" } });

    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_settings", {
        settings: expect.objectContaining({
          voice_input: expect.objectContaining({
            hotkey: "Ctrl+Shift+V",
            language: "ja",
          }),
        }),
      });
    });
  });

  it("shows unavailable reason banner when voice capability is unavailable", async () => {
    const enabledVoiceSettings = structuredClone(settingsFixture);
    enabledVoiceSettings.voice_input.enabled = true;
    tauriCoreInvokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: false, reason: "GPU acceleration is not available" };
      }
      return null;
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(enabledVoiceSettings);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Voice Input");

    await waitFor(() => {
      expect(rendered.getByText(/GPU acceleration is not available/)).toBeTruthy();
      expect(rendered.getByText(/Settings can still be configured/)).toBeTruthy();
    });
  });

  it("edits environment variable value inline", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("Environment Variables");

    // The default profile has API_KEY=xxx
    const envRows = rendered.container.querySelectorAll(".env-row");
    expect(envRows.length).toBeGreaterThan(0);

    const apiKeyRow = Array.from(envRows).find((r) =>
      (r.textContent ?? "").includes("API_KEY")
    ) as HTMLElement;
    expect(apiKeyRow).toBeTruthy();

    const valueInput = apiKeyRow.querySelector(".env-value") as HTMLInputElement;
    await fireEvent.input(valueInput, { target: { value: "new-secret" } });

    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_profiles", {
        config: expect.objectContaining({
          profiles: expect.objectContaining({
            default: expect.objectContaining({
              env: expect.objectContaining({ API_KEY: "new-secret" }),
            }),
          }),
        }),
      });
    });
  });

  it("switches active profile dropdown and shows different profile's env", async () => {
    const twoProfiles = structuredClone(profilesFixture);
    twoProfiles.profiles.staging = {
      name: "staging",
      description: "",
      env: { STAGE_KEY: "stage-val" },
      disabled_env: [],
      ai_enabled: true,
      ai: {
        endpoint: "https://api.openai.com/v1",
        api_key: "stage-key",
        model: "gpt-4o-mini",
        language: "en",
      },
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(twoProfiles);
      if (command === "list_ai_models") return [{ id: "gpt-5" }];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("Environment Variables");

    const activeProfile = rendered.container.querySelector(".profile-select") as HTMLSelectElement;
    await fireEvent.change(activeProfile, { target: { value: "staging" } });

    await waitFor(() => {
      expect(activeProfile.value).toBe("staging");
      expect(rendered.container.textContent).toContain("STAGE_KEY");
    });
  });

  it("does not prepend current model to list when already present", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-4o-mini" }, { id: "gpt-5" }];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();
    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await fireEvent.click(rendered.getByRole("button", { name: "Refresh" }));

    await waitFor(() => {
      const options = Array.from(
        rendered.container.querySelectorAll(".ai-model-select option")
      ).map((o) => o.textContent?.trim());
      // gpt-4o-mini is both in the API list and the current model; should not be duplicated
      const occurrences = options.filter((o) => o === "gpt-4o-mini");
      expect(occurrences.length).toBe(1);
    });
  });

  it("shows model dropdown with empty current when profile has no model set", async () => {
    const noModelProfiles = structuredClone(profilesFixture);
    noModelProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "test-key",
      model: "",
      language: "en",
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(noModelProfiles);
      if (command === "list_ai_models") return [{ id: "gpt-5" }];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();
    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await fireEvent.click(rendered.getByRole("button", { name: "Refresh" }));

    await waitFor(() => {
      const options = Array.from(
        rendered.container.querySelectorAll(".ai-model-select option")
      ).map((o) => o.textContent?.trim());
      expect(options).toContain("gpt-5");
      // No missing model warning since current is empty
      expect(rendered.queryByText("Current model is not listed")).toBeNull();
    });
  });

  it("updates selected AI model and persists it in profile config", async () => {
    const rendered = await renderSettingsPanel();
    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await fireEvent.click(rendered.getByRole("button", { name: "Refresh" }));

    await waitFor(() => {
      const options = Array.from(
        rendered.container.querySelectorAll(".ai-model-select option")
      ).map((o) => o.textContent?.trim());
      expect(options).toContain("gpt-5");
    });

    const modelSelect = rendered.container.querySelector(".ai-model-select") as HTMLSelectElement;
    await fireEvent.change(modelSelect, { target: { value: "gpt-5" } });
    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_profiles", {
        config: expect.objectContaining({
          profiles: expect.objectContaining({
            default: expect.objectContaining({
              ai: expect.objectContaining({ model: "gpt-5" }),
            }),
          }),
        }),
      });
    });
  });

  it("shows model options without current model before refresh", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();
    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    // Before refresh, aiModelsLoadedKey !== currentAiRequestKey
    // So current model from profile should not be shown in dropdown options
    const options = Array.from(
      rendered.container.querySelectorAll(".ai-model-select option")
    ).map((o) => o.textContent?.trim());
    // Options should be empty or just have placeholder since no models loaded yet
    expect(options.length).toBeLessThanOrEqual(1);
  });

  it("shows refresh hint when endpoint changes after models are loaded", async () => {
    const rendered = await renderSettingsPanel();
    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await fireEvent.click(rendered.getByRole("button", { name: "Refresh" }));

    await waitFor(() => {
      const options = Array.from(
        rendered.container.querySelectorAll(".ai-model-select option")
      ).map((o) => o.textContent?.trim());
      expect(options).toContain("gpt-5");
    });

    const endpointField = Array.from(rendered.container.querySelectorAll(".settings-section-body .field")).find((field) =>
      (field.textContent ?? "").includes("Endpoint")
    ) as HTMLElement | undefined;
    const endpointInput = endpointField?.querySelector("input") as HTMLInputElement;
    expect(endpointInput).toBeTruthy();
    await fireEvent.input(endpointInput, { target: { value: "https://example.local/v1" } });

    await waitFor(() => {
      expect(rendered.getByText("Click Refresh to load models from /v1/models.")).toBeTruthy();
    });
  });

  it("saves selected shell via Terminal tab", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
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

  it("shows 'Create a profile' hint in AI Settings when no profile is selected", async () => {
    // Use a profiles config with no active profile and no profile keys
    const emptyProfiles: ProfilesConfig = {
      version: 1,
      active: null,
      profiles: {},
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(emptyProfiles);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    await waitFor(() => {
      expect(rendered.getByText("Create a profile to configure settings.")).toBeTruthy();
    });
  });

  it("updates AI language field via dropdown selection", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("AI Configuration");

    // Find the AI response language select - it's inside a .field with "AI response language" label
    const aiFields = rendered.container.querySelectorAll(".settings-section-body .field");
    const languageField = Array.from(aiFields).find((f) =>
      (f.textContent ?? "").includes("AI response language")
    ) as HTMLElement | undefined;
    expect(languageField).toBeTruthy();

    const languageSelect = languageField!.querySelector("select") as HTMLSelectElement;
    expect(languageSelect).toBeTruthy();

    await fireEvent.change(languageSelect, { target: { value: "ja" } });

    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_profiles", {
        config: expect.objectContaining({
          profiles: expect.objectContaining({
            default: expect.objectContaining({
              ai: expect.objectContaining({ language: "ja" }),
            }),
          }),
        }),
      });
    });
  });

  it("keeps Refresh button disabled when endpoint is empty", async () => {
    const noEndpointProfiles: ProfilesConfig = {
      version: 1,
      active: "default",
      profiles: {
        default: {
          name: "default",
          description: "",
          env: {},
          disabled_env: [],
          ai: {
            endpoint: "",
            api_key: "",
            model: "",
            language: "en",
              },
        },
      },
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(noEndpointProfiles);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("AI Configuration");

    const refreshBtn = rendered.getByRole("button", { name: "Refresh" }) as HTMLButtonElement;
    expect(refreshBtn.disabled).toBe(true);
    expect(rendered.queryByText("Endpoint is required.")).toBeNull();
  });

  it("renders AI language ?? 'en' fallback when language field is undefined", async () => {
    const noLangProfiles: ProfilesConfig = {
      version: 1,
      active: "default",
      profiles: {
        default: {
          name: "default",
          description: "",
          env: {},
          disabled_env: [],
          ai: {
            endpoint: "https://api.example.com/v1",
            api_key: "key",
            model: "gpt-4",
            // language intentionally omitted to trigger ?? 'en' fallback
            language: undefined as any,
          },
        },
      },
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(noLangProfiles);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");

    await rendered.findByText("AI Configuration");

    // Profile Language select should fall back to "en" value
    const aiFields = rendered.container.querySelectorAll(".settings-section-body .field");
    const languageField = Array.from(aiFields).find((f) =>
      (f.textContent ?? "").includes("AI response language")
    ) as HTMLElement | undefined;
    expect(languageField).toBeTruthy();

    const languageSelect = languageField!.querySelector("select") as HTMLSelectElement;
    expect(languageSelect).toBeTruthy();
    expect(languageSelect.value).toBe("en");
  });

  it("defaults shell selection to empty string when default_shell is null", async () => {
    const noShellSettings = structuredClone(settingsFixture);
    (noShellSettings as any).default_shell = null;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(noShellSettings);
      if (command === "get_profiles") return structuredClone(profilesFixture);
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
      const select = rendered.container.querySelector("#default-shell") as HTMLSelectElement;
      expect(select).toBeTruthy();
      expect(select.value).toBe("");
    });

    // Selecting empty string (System Default) should set shell to null
    const select = rendered.container.querySelector("#default-shell") as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: "" } });

    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_settings", {
        settings: expect.objectContaining({
          default_shell: null,
        }),
      });
    });
  });

  it("shows voice capability loading message when checking capability", async () => {
    const enabledVoiceSettings = structuredClone(settingsFixture);
    enabledVoiceSettings.voice_input.enabled = true;
    let resolveCapability!: (v: any) => void;
    const pendingCapability = new Promise<any>((r) => { resolveCapability = r; });
    tauriCoreInvokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return pendingCapability;
      }
      return null;
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(enabledVoiceSettings);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Voice Input");

    await waitFor(() => {
      expect(rendered.getByText("Checking voice runtime capability...")).toBeTruthy();
    });

    resolveCapability({ available: true, reason: null });
  });

  it("handles voice capability check error gracefully and keeps fields enabled", async () => {
    tauriCoreInvokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        throw new Error("tauri not available");
      }
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Voice Input");

    await waitFor(() => {
      // When capability check throws, voiceAvailable defaults to true
      // so no unavailable banner should be shown
      const voiceEnabled = rendered.container.querySelector("#voice-input-enabled") as HTMLInputElement;
      expect(voiceEnabled).toBeTruthy();
      expect(voiceEnabled.disabled).toBe(false);
    });

    // Should not show the unavailable reason banner
    expect(
      rendered.queryByText(/GPU acceleration and Qwen runtime are required/)
    ).toBeNull();
  });

  it("passes gpuAvailable=false when WebGL renderer is software", async () => {
    const realCreateElement = document.createElement.bind(document);
    const fakeGl = {
      getExtension: vi.fn(() => ({ UNMASKED_RENDERER_WEBGL: 0x9246 })),
      getParameter: vi.fn(() => "Google SwiftShader"),
    };
    const createSpy = vi.spyOn(document, "createElement").mockImplementation(((tagName: string) => {
      if (String(tagName).toLowerCase() === "canvas") {
        return {
          getContext: vi.fn((kind: string) => (kind === "webgl2" ? fakeGl : null)),
        } as unknown as HTMLCanvasElement;
      }
      return realCreateElement(tagName);
    }) as typeof document.createElement);

    try {
      await renderSettingsPanel();
      await waitFor(() => {
        expect(tauriCoreInvokeMock).toHaveBeenCalledWith(
          "get_voice_capability",
          expect.objectContaining({ gpuAvailable: false }),
        );
      });
    } finally {
      createSpy.mockRestore();
    }
  });

  it("passes gpuAvailable=true when WebGL renderer is hardware accelerated", async () => {
    const realCreateElement = document.createElement.bind(document);
    const fakeGl = {
      getExtension: vi.fn(() => ({ UNMASKED_RENDERER_WEBGL: 0x9246 })),
      getParameter: vi.fn(() => "NVIDIA GeForce RTX"),
    };
    const createSpy = vi.spyOn(document, "createElement").mockImplementation(((tagName: string) => {
      if (String(tagName).toLowerCase() === "canvas") {
        return {
          getContext: vi.fn((kind: string) => (kind === "webgl2" ? fakeGl : null)),
        } as unknown as HTMLCanvasElement;
      }
      return realCreateElement(tagName);
    }) as typeof document.createElement);

    try {
      await renderSettingsPanel();
      await waitFor(() => {
        expect(tauriCoreInvokeMock).toHaveBeenCalledWith(
          "get_voice_capability",
          expect.objectContaining({ gpuAvailable: true }),
        );
      });
    } finally {
      createSpy.mockRestore();
    }
  });

  it("passes gpuAvailable=true when renderer extension is unavailable", async () => {
    const realCreateElement = document.createElement.bind(document);
    const fakeGl = {
      getExtension: vi.fn(() => null),
      getParameter: vi.fn(),
    };
    const createSpy = vi.spyOn(document, "createElement").mockImplementation(((tagName: string) => {
      if (String(tagName).toLowerCase() === "canvas") {
        return {
          getContext: vi.fn((kind: string) => (kind === "webgl2" ? fakeGl : null)),
        } as unknown as HTMLCanvasElement;
      }
      return realCreateElement(tagName);
    }) as typeof document.createElement);

    try {
      await renderSettingsPanel();
      await waitFor(() => {
        expect(tauriCoreInvokeMock).toHaveBeenCalledWith(
          "get_voice_capability",
          expect.objectContaining({ gpuAvailable: true }),
        );
      });
    } finally {
      createSpy.mockRestore();
    }
  });

  it("clears AI model state when endpoint is removed after model refresh", async () => {
    const noModelProfiles = structuredClone(profilesFixture);
    noModelProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "test-key",
      model: "",
      language: "en",
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(noModelProfiles);
      if (command === "list_ai_models") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();
    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await fireEvent.click(rendered.getByRole("button", { name: "Refresh" }));

    await waitFor(() => {
      expect(rendered.getByText("No models returned from /v1/models.")).toBeTruthy();
    });

    const endpointField = Array.from(
      rendered.container.querySelectorAll(".settings-section-body .field"),
    ).find((field) => (field.textContent ?? "").includes("Endpoint")) as HTMLElement | undefined;
    expect(endpointField).toBeTruthy();
    const endpointInput = endpointField?.querySelector("input") as HTMLInputElement;
    expect(endpointInput).toBeTruthy();
    await fireEvent.input(endpointInput, { target: { value: "" } });

    await waitFor(() => {
      expect(rendered.queryByText("No models returned from /v1/models.")).toBeNull();
      const refreshBtn = rendered.getByRole("button", { name: "Refresh" }) as HTMLButtonElement;
      expect(refreshBtn.disabled).toBe(true);
    });
  });

  it("shows default unavailable reason when voice capability reason is null", async () => {
    const enabledVoiceSettings = structuredClone(settingsFixture);
    enabledVoiceSettings.voice_input.enabled = true;
    tauriCoreInvokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: false, reason: null };
      }
      return null;
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(enabledVoiceSettings);
      if (command === "get_profiles") return structuredClone(profilesFixture);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Voice Input");

    await waitFor(() => {
      expect(
        rendered.getByText(/GPU acceleration and Qwen runtime are required/)
      ).toBeTruthy();
    });
  });

  it("shows error state when settings are null after load error", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return null as any;
      if (command === "get_profiles") return structuredClone(profilesFixture);
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      const loading = rendered.container.querySelector(".loading");
      expect((loading?.textContent ?? "").startsWith("Failed to load settings:")).toBe(true);
    });
  });

  it("normalizes malformed loaded data and keeps tabs usable", async () => {
    const malformedSettings = structuredClone(settingsFixture) as SettingsData & Record<string, unknown>;
    malformedSettings.ui_font_size = null as unknown as number;
    malformedSettings.terminal_font_size = undefined as unknown as number;
    malformedSettings.ui_font_family = "";
    malformedSettings.terminal_font_family = "";
    malformedSettings.app_language = "xx" as SettingsData["app_language"];
    malformedSettings.voice_input = {
      enabled: true,
      engine: "whisper" as any,
      hotkey: "",
      ptt_hotkey: "",
      language: "fr" as any,
      quality: "ultra" as any,
      model: "",
    } as SettingsData["voice_input"];
    malformedSettings.agent_skill_registration_enabled = true;

    const malformedProfiles: ProfilesConfig = {
      version: 1,
      active: null,
      profiles: {},
    };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(malformedSettings);
      if (command === "get_profiles") return structuredClone(malformedProfiles);
      if (command === "get_available_shells") {
        return [{ id: "pwsh", name: "PowerShell", version: null }];
      }
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      const tabNames = Array.from(
        rendered.container.querySelectorAll(".settings-tab-btn"),
      ).map((btn) => btn.textContent?.trim());
      expect(tabNames).toEqual([
        "General",
        "Profiles",
        "Terminal",
        "Voice Input",
        "Agent",
      ]);
    });

    const appLanguageSelect = rendered.container.querySelector(
      "#app-language",
    ) as HTMLSelectElement | null;
    expect(appLanguageSelect?.value).toBe("auto");

    await switchToTab(rendered, "Voice Input");
    await waitFor(() => {
      const hotkey = rendered.container.querySelector("#voice-hotkey") as HTMLInputElement | null;
      const pttHotkey = rendered.container.querySelector(
        "#voice-ptt-hotkey",
      ) as HTMLInputElement | null;
      const language = rendered.container.querySelector(
        "#voice-language",
      ) as HTMLSelectElement | null;
      const quality = rendered.container.querySelector(
        "#voice-quality",
      ) as HTMLSelectElement | null;
      const model = rendered.container.querySelector("#voice-model") as HTMLInputElement | null;
      expect(hotkey?.value).toBe("Mod+Shift+M");
      expect(pttHotkey?.value).toBe("Mod+Shift+Space");
      expect(language?.value).toBe("auto");
      expect(quality?.value).toBe("balanced");
      expect(model?.value).toBe("Qwen/Qwen3-ASR-1.7B");
    });

    await switchToTab(rendered, "Terminal");
    await waitFor(() => {
      const shellSelect = rendered.container.querySelector(
        "#default-shell",
      ) as HTMLSelectElement | null;
      expect(shellSelect).toBeTruthy();
      expect(shellSelect?.options[1]?.textContent?.trim()).toBe("PowerShell");
    });

    await switchToTab(rendered, "Profiles");
    await waitFor(() => {
      expect(rendered.getByText("Create a profile to configure settings.")).toBeTruthy();
    });
  });

  // --- API Key peek / copy (Issue #1433) ---

  it("hides peek and copy buttons when API key is empty", async () => {
    const emptyKeyProfiles = structuredClone(profilesFixture);
    emptyKeyProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "",
      model: "gpt-4o-mini",
      language: "en",
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(emptyKeyProfiles);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const peekBtn = rendered.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;
    const copyBtn = rendered.container.querySelector(".btn-copy-apikey") as HTMLButtonElement;

    expect(peekBtn.disabled).toBe(true);
    expect(copyBtn.disabled).toBe(true);
    expect(peekBtn.parentElement?.classList.contains("hidden")).toBe(true);
  });

  it("shows peek and copy buttons when API key is typed", async () => {
    const emptyKeyProfiles = structuredClone(profilesFixture);
    emptyKeyProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "",
      model: "gpt-4o-mini",
      language: "en",
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(emptyKeyProfiles);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const apiKeyField = Array.from(rendered.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const apiKeyInput = apiKeyField.querySelector("input") as HTMLInputElement;

    await fireEvent.input(apiKeyInput, { target: { value: "sk-draft-key" } });

    await waitFor(() => {
      const peekBtn = rendered.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;
      const copyBtn = rendered.container.querySelector(".btn-copy-apikey") as HTMLButtonElement;
      expect(peekBtn.disabled).toBe(false);
      expect(copyBtn.disabled).toBe(false);
      expect(peekBtn.parentElement?.classList.contains("hidden")).toBe(false);
    });
  });

  it("shows peek and copy buttons when API key is pasted", async () => {
    const emptyKeyProfiles = structuredClone(profilesFixture);
    emptyKeyProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "",
      model: "gpt-4o-mini",
      language: "en",
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(emptyKeyProfiles);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const apiKeyField = Array.from(rendered.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const apiKeyInput = apiKeyField.querySelector("input") as HTMLInputElement;

    await pasteText(apiKeyInput, "sk-pasted-key");

    await waitFor(() => {
      expect(apiKeyInput.value).toBe("sk-pasted-key");
      const peekBtn = rendered.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;
      const copyBtn = rendered.container.querySelector(".btn-copy-apikey") as HTMLButtonElement;
      expect(peekBtn.disabled).toBe(false);
      expect(copyBtn.disabled).toBe(false);
      expect(peekBtn.parentElement?.classList.contains("hidden")).toBe(false);
    });
  });

  it("shows peek and copy buttons when API key is non-empty", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const peekBtn = rendered.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;
    const copyBtn = rendered.container.querySelector(".btn-copy-apikey") as HTMLButtonElement;
    expect(peekBtn.disabled).toBe(false);
    expect(copyBtn.disabled).toBe(false);
    expect(peekBtn.parentElement?.classList.contains("hidden")).toBe(false);
  });

  it("renders SVG icons inside API key peek and copy buttons", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const peekBtn = rendered.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;
    const copyBtn = rendered.container.querySelector(".btn-copy-apikey") as HTMLButtonElement;

    expect(peekBtn.querySelector("svg")).not.toBeNull();
    expect(copyBtn.querySelector("svg")).not.toBeNull();
  });

  it("reveals API key on mousedown and hides on mouseup", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const apiKeyField = Array.from(rendered.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const apiKeyInput = apiKeyField.querySelector("input") as HTMLInputElement;
    const peekBtn = rendered.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;

    // Initial state: always type="text", masked via CSS class
    expect(apiKeyInput.type).toBe("text");
    expect(apiKeyInput.classList.contains("api-key-masked")).toBe(true);

    // mousedown → peek (unmask)
    await fireEvent.mouseDown(peekBtn);
    expect(apiKeyInput.type).toBe("text");
    expect(apiKeyInput.classList.contains("api-key-masked")).toBe(false);

    // mouseup → hide (mask again)
    await fireEvent.mouseUp(peekBtn);
    expect(apiKeyInput.type).toBe("text");
    expect(apiKeyInput.classList.contains("api-key-masked")).toBe(true);
  });

  it("toggles API key visibility on keyboard/assistive click activation", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const apiKeyField = Array.from(rendered.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const apiKeyInput = apiKeyField.querySelector("input") as HTMLInputElement;
    const peekBtn = rendered.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;

    expect(apiKeyInput.type).toBe("text");
    expect(apiKeyInput.classList.contains("api-key-masked")).toBe(true);

    await fireEvent.click(peekBtn, { detail: 0 });
    expect(apiKeyInput.type).toBe("text");
    expect(apiKeyInput.classList.contains("api-key-masked")).toBe(false);

    await fireEvent.click(peekBtn, { detail: 0 });
    expect(apiKeyInput.type).toBe("text");
    expect(apiKeyInput.classList.contains("api-key-masked")).toBe(true);
  });

  it("hides API key on mouseleave from peek button", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const apiKeyField = Array.from(rendered.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const apiKeyInput = apiKeyField.querySelector("input") as HTMLInputElement;
    const peekBtn = rendered.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;

    await fireEvent.mouseDown(peekBtn);
    expect(apiKeyInput.classList.contains("api-key-masked")).toBe(false);

    await fireEvent.mouseLeave(peekBtn);
    expect(apiKeyInput.type).toBe("text");
    expect(apiKeyInput.classList.contains("api-key-masked")).toBe(true);
  });

  it("copies API key to clipboard on copy button click", async () => {
    const writeTextMock = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: writeTextMock },
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const copyBtn = rendered.container.querySelector(".btn-copy-apikey") as HTMLButtonElement;
    await fireEvent.click(copyBtn);

    expect(writeTextMock).toHaveBeenCalledWith("test-key");
  });

  it("shows Copied! feedback and reverts after timeout", async () => {
    vi.useFakeTimers();
    try {
      const writeTextMock = vi.fn().mockResolvedValue(undefined);
      Object.defineProperty(navigator, "clipboard", {
        configurable: true,
        value: { writeText: writeTextMock },
      });

      const rendered = await renderSettingsPanel();

      await waitFor(() => {
        expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
      });

      await switchToTab(rendered, "Profiles");
      await rendered.findByText("API key");

      const copyBtn = rendered.container.querySelector(".btn-copy-apikey") as HTMLButtonElement;
      await fireEvent.click(copyBtn);

      await waitFor(() => {
        expect(copyBtn.getAttribute("title")).toBe("Copied!");
        expect(copyBtn.classList.contains("copied")).toBe(true);
      });

      await vi.advanceTimersByTimeAsync(1500);

      await waitFor(() => {
        expect(copyBtn.getAttribute("title")).toBe("Copy API Key");
        expect(copyBtn.classList.contains("copied")).toBe(false);
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("copies plaintext API key even when masked", async () => {
    const writeTextMock = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: writeTextMock },
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const apiKeyField = Array.from(rendered.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const apiKeyInput = apiKeyField.querySelector("input") as HTMLInputElement;

    // Confirm it's masked via CSS class (type is always "text")
    expect(apiKeyInput.type).toBe("text");
    expect(apiKeyInput.classList.contains("api-key-masked")).toBe(true);

    const copyBtn = rendered.container.querySelector(".btn-copy-apikey") as HTMLButtonElement;
    await fireEvent.click(copyBtn);

    // Should copy the plaintext value despite being masked
    expect(writeTextMock).toHaveBeenCalledWith("test-key");
  });

  it("updates profile state when API key is typed (issue #1480)", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const apiKeyField = Array.from(rendered.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const apiKeyInput = apiKeyField.querySelector("input") as HTMLInputElement;

    // type is always "text" (never "password") to avoid WKWebView issues
    expect(apiKeyInput.type).toBe("text");

    await fireEvent.input(apiKeyInput, { target: { value: "sk-new-key-123" } });

    // Refresh should send the updated key to list_ai_models
    const refreshBtn = rendered.getByRole("button", { name: "Refresh" }) as HTMLButtonElement;
    await fireEvent.click(refreshBtn);

    await waitFor(() => {
      const listCall = invokeMock.mock.calls.find(([cmd]) => cmd === "list_ai_models");
      expect(listCall).toBeTruthy();
      expect(listCall![1]).toMatchObject({ apiKey: "sk-new-key-123" });
    });
  });

  it("uses pasted API key when Refresh is clicked", async () => {
    const emptyKeyProfiles = structuredClone(profilesFixture);
    emptyKeyProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "",
      model: "",
      language: "en",
    };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(emptyKeyProfiles);
      if (command === "list_ai_models") return [{ id: "gpt-5" }, { id: "gpt-4o-mini" }];
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const apiKeyField = Array.from(rendered.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const apiKeyInput = apiKeyField.querySelector("input") as HTMLInputElement;

    await pasteText(apiKeyInput, "sk-pasted-refresh");

    const refreshBtn = rendered.getByRole("button", { name: "Refresh" }) as HTMLButtonElement;
    await fireEvent.click(refreshBtn);

    await waitFor(() => {
      const listCall = invokeMock.mock.calls.find(([cmd]) => cmd === "list_ai_models");
      expect(listCall).toBeTruthy();
      expect(listCall![1]).toMatchObject({ apiKey: "sk-pasted-refresh" });
    });
  });

  it("persists API key to save_profiles on Save (issue #1480)", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const apiKeyField = Array.from(rendered.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const apiKeyInput = apiKeyField.querySelector("input") as HTMLInputElement;

    await fireEvent.input(apiKeyInput, { target: { value: "sk-saved-key" } });

    // Click Save
    const saveBtn = rendered.getByRole("button", { name: "Save" }) as HTMLButtonElement;
    await fireEvent.click(saveBtn);

    await waitFor(() => {
      const saveCall = invokeMock.mock.calls.find(([cmd]) => cmd === "save_profiles");
      expect(saveCall).toBeTruthy();
      const savedConfig = saveCall![1].config as ProfilesConfig;
      expect(savedConfig.profiles.default.ai?.api_key).toBe("sk-saved-key");
    });
  });

  it("reloads saved default profile API key after closing and reopening settings", async () => {
    let persistedSettings = structuredClone(settingsFixture);
    let persistedProfiles = structuredClone(profilesFixture);
    persistedProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "",
      model: "",
      language: "ja",
    };

    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "get_settings") return structuredClone(persistedSettings);
      if (command === "get_profiles") return structuredClone(persistedProfiles);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") {
        persistedSettings = structuredClone(args?.settings as SettingsData);
        return null;
      }
      if (command === "save_profiles") {
        persistedProfiles = structuredClone(args?.config as ProfilesConfig);
        return null;
      }
      return null;
    });

    const first = await renderSettingsPanel();

    await waitFor(() => {
      expect(first.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(first, "Profiles");
    await first.findByText("API key");

    const firstApiKeyField = Array.from(first.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const firstApiKeyInput = firstApiKeyField.querySelector("input") as HTMLInputElement;

    await fireEvent.input(firstApiKeyInput, { target: { value: "sk-reopen-check" } });
    await fireEvent.click(first.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(persistedProfiles.profiles.default.ai?.api_key).toBe("sk-reopen-check");
    });

    first.unmount();

    const reopened = await renderSettingsPanel();

    await waitFor(() => {
      expect(reopened.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(reopened, "Profiles");
    await reopened.findByText("API key");

    const reopenedApiKeyField = Array.from(reopened.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const reopenedApiKeyInput = reopenedApiKeyField.querySelector("input") as HTMLInputElement;

    await waitFor(() => {
      expect(reopenedApiKeyInput.value).toBe("sk-reopen-check");
      const peekBtn = reopened.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;
      const copyBtn = reopened.container.querySelector(".btn-copy-apikey") as HTMLButtonElement;
      expect(peekBtn.disabled).toBe(false);
      expect(copyBtn.disabled).toBe(false);
      expect(peekBtn.parentElement?.classList.contains("hidden")).toBe(false);
    });
  });

  it("reloads pasted default profile API key after closing and reopening settings", async () => {
    let persistedSettings = structuredClone(settingsFixture);
    let persistedProfiles = structuredClone(profilesFixture);
    persistedProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "",
      model: "",
      language: "ja",
    };

    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "get_settings") return structuredClone(persistedSettings);
      if (command === "get_profiles") return structuredClone(persistedProfiles);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") {
        persistedSettings = structuredClone(args?.settings as SettingsData);
        return null;
      }
      if (command === "save_profiles") {
        persistedProfiles = structuredClone(args?.config as ProfilesConfig);
        return null;
      }
      return null;
    });

    const first = await renderSettingsPanel();

    await waitFor(() => {
      expect(first.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(first, "Profiles");
    await first.findByText("API key");

    const firstApiKeyField = Array.from(first.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const firstApiKeyInput = firstApiKeyField.querySelector("input") as HTMLInputElement;

    await pasteText(firstApiKeyInput, "sk-pasted-reopen");
    await fireEvent.click(first.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(persistedProfiles.profiles.default.ai?.api_key).toBe("sk-pasted-reopen");
    });

    first.unmount();

    const reopened = await renderSettingsPanel();

    await waitFor(() => {
      expect(reopened.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(reopened, "Profiles");
    await reopened.findByText("API key");

    const reopenedApiKeyField = Array.from(reopened.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const reopenedApiKeyInput = reopenedApiKeyField.querySelector("input") as HTMLInputElement;

    await waitFor(() => {
      expect(reopenedApiKeyInput.value).toBe("sk-pasted-reopen");
      const peekBtn = reopened.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;
      const copyBtn = reopened.container.querySelector(".btn-copy-apikey") as HTMLButtonElement;
      expect(peekBtn.disabled).toBe(false);
      expect(copyBtn.disabled).toBe(false);
      expect(peekBtn.parentElement?.classList.contains("hidden")).toBe(false);
    });
  });

  it("resets API key draft and button visibility when switching profiles", async () => {
    const twoProfiles = structuredClone(profilesFixture);
    twoProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "",
      model: "gpt-4o-mini",
      language: "en",
    };
    twoProfiles.profiles.dev = {
      name: "dev",
      description: "",
      env: {},
      disabled_env: [],
      ai_enabled: true,
      ai: {
        endpoint: "https://api.openai.com/v1",
        api_key: "",
        model: "gpt-4o-mini",
        language: "en",
      },
    };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(twoProfiles);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const apiKeyField = Array.from(rendered.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const apiKeyInput = apiKeyField.querySelector("input") as HTMLInputElement;
    const activeProfile = rendered.container.querySelector(".profile-select") as HTMLSelectElement;

    await fireEvent.input(apiKeyInput, { target: { value: "sk-unsaved-key" } });

    await waitFor(() => {
      const peekBtn = rendered.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;
      const copyBtn = rendered.container.querySelector(".btn-copy-apikey") as HTMLButtonElement;
      expect(peekBtn.disabled).toBe(false);
      expect(copyBtn.disabled).toBe(false);
      expect(peekBtn.parentElement?.classList.contains("hidden")).toBe(false);
    });

    await fireEvent.change(activeProfile, { target: { value: "dev" } });

    await waitFor(() => {
      expect(apiKeyInput.value).toBe("");
      const peekBtn = rendered.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;
      const copyBtn = rendered.container.querySelector(".btn-copy-apikey") as HTMLButtonElement;
      expect(peekBtn.disabled).toBe(true);
      expect(copyBtn.disabled).toBe(true);
      expect(peekBtn.parentElement?.classList.contains("hidden")).toBe(true);
    });
  });

  it("preserves unsaved API key edits across profile switches on Save", async () => {
    const twoProfiles = structuredClone(profilesFixture);
    twoProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "",
      model: "gpt-4o-mini",
      language: "en",
    };
    twoProfiles.profiles.dev = {
      name: "dev",
      description: "",
      env: {},
      disabled_env: [],
      ai_enabled: true,
      ai: {
        endpoint: "https://api.openai.com/v1",
        api_key: "",
        model: "gpt-4o-mini",
        language: "en",
      },
    };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(twoProfiles);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const apiKeyField = Array.from(rendered.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const apiKeyInput = apiKeyField.querySelector("input") as HTMLInputElement;
    const activeProfile = rendered.container.querySelector(".profile-select") as HTMLSelectElement;

    await fireEvent.input(apiKeyInput, { target: { value: "sk-profile-a" } });
    await fireEvent.change(activeProfile, { target: { value: "dev" } });

    const saveBtn = rendered.getByRole("button", { name: "Save" }) as HTMLButtonElement;
    await fireEvent.click(saveBtn);

    await waitFor(() => {
      const saveCall = invokeMock.mock.calls.findLast(([cmd]) => cmd === "save_profiles");
      expect(saveCall).toBeTruthy();
      const savedConfig = saveCall![1].config as ProfilesConfig;
      expect(savedConfig.profiles.default.ai?.api_key).toBe("sk-profile-a");
      expect(savedConfig.profiles.dev.ai?.api_key).toBe("");
    });

    await fireEvent.change(activeProfile, { target: { value: "default" } });

    await waitFor(() => {
      expect(apiKeyInput.value).toBe("sk-profile-a");
      const peekBtn = rendered.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;
      const copyBtn = rendered.container.querySelector(".btn-copy-apikey") as HTMLButtonElement;
      expect(peekBtn.disabled).toBe(false);
      expect(copyBtn.disabled).toBe(false);
      expect(peekBtn.parentElement?.classList.contains("hidden")).toBe(false);
    });
  });

  it("keeps API key value with underscores while peeking", async () => {
    const underscoreProfiles = structuredClone(profilesFixture);
    underscoreProfiles.profiles.default.ai = {
      endpoint: "https://api.openai.com/v1",
      api_key: "sk_test_ab_cd",
      model: "gpt-4o-mini",
      language: "en",
    };

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_settings") return structuredClone(settingsFixture);
      if (command === "get_profiles") return structuredClone(underscoreProfiles);
      if (command === "get_available_shells") return [];
      if (command === "save_settings") return null;
      if (command === "save_profiles") return null;
      return null;
    });

    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Profiles");
    await rendered.findByText("API key");

    const apiKeyField = Array.from(rendered.container.querySelectorAll(".settings-section-body .field")).find((f) =>
      (f.textContent ?? "").includes("API key")
    ) as HTMLElement;
    const apiKeyInput = apiKeyField.querySelector("input") as HTMLInputElement;
    const peekBtn = rendered.container.querySelector(".btn-peek-apikey") as HTMLButtonElement;

    await fireEvent.mouseDown(peekBtn);
    expect(apiKeyInput.type).toBe("text");
    expect(apiKeyInput.value).toBe("sk_test_ab_cd");
    expect(apiKeyInput.value.includes("_")).toBe(true);
  });

  // --- Agent tab: Docs Injection (Phase 2) ---

  it("agent_tab_renders_docs_injection_checkboxes", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Agent");

    await waitFor(() => {
      expect(rendered.getByText("Docs Injection")).toBeTruthy();
    });

    const claudeMdCheckbox = rendered.container.querySelector("#agent-inject-claude-md") as HTMLInputElement;
    const agentsMdCheckbox = rendered.container.querySelector("#agent-inject-agents-md") as HTMLInputElement;
    const geminiMdCheckbox = rendered.container.querySelector("#agent-inject-gemini-md") as HTMLInputElement;

    expect(claudeMdCheckbox).toBeTruthy();
    expect(agentsMdCheckbox).toBeTruthy();
    expect(geminiMdCheckbox).toBeTruthy();
    expect(claudeMdCheckbox.type).toBe("checkbox");
    expect(agentsMdCheckbox.type).toBe("checkbox");
    expect(geminiMdCheckbox.type).toBe("checkbox");
  });

  it("agent_tab_claude_md_checked_by_default", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Agent");

    await waitFor(() => {
      expect(rendered.getByText("Docs Injection")).toBeTruthy();
    });

    const claudeMdCheckbox = rendered.container.querySelector("#agent-inject-claude-md") as HTMLInputElement;
    const agentsMdCheckbox = rendered.container.querySelector("#agent-inject-agents-md") as HTMLInputElement;
    const geminiMdCheckbox = rendered.container.querySelector("#agent-inject-gemini-md") as HTMLInputElement;

    expect(claudeMdCheckbox.checked).toBe(true);
    expect(agentsMdCheckbox.checked).toBe(false);
    expect(geminiMdCheckbox.checked).toBe(false);
  });

  it("agent_tab_saves_injection_settings", async () => {
    const rendered = await renderSettingsPanel();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".settings-tab-btn").length).toBe(5);
    });

    await switchToTab(rendered, "Agent");

    await waitFor(() => {
      expect(rendered.getByText("Docs Injection")).toBeTruthy();
    });

    const claudeMdCheckbox = rendered.container.querySelector("#agent-inject-claude-md") as HTMLInputElement;
    const agentsMdCheckbox = rendered.container.querySelector("#agent-inject-agents-md") as HTMLInputElement;

    // Uncheck CLAUDE.md
    await fireEvent.click(claudeMdCheckbox);
    // Check AGENTS.md
    await fireEvent.click(agentsMdCheckbox);

    await fireEvent.click(rendered.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      const saveCall = invokeMock.mock.calls.find(([cmd]) => cmd === "save_settings");
      expect(saveCall).toBeTruthy();
      const savedSettings = saveCall![1].settings as SettingsData;
      expect(savedSettings.agent_inject_claude_md).toBe(false);
      expect(savedSettings.agent_inject_agents_md).toBe(true);
    });
  });
});
