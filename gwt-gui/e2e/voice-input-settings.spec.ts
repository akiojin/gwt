import { expect, test, type Page } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";

const defaultRecentProject = {
  path: "/tmp/gwt-playwright",
  lastOpened: "2026-02-13T00:00:00.000Z",
};

const branchFeature = {
  name: "feature/voice-input",
  commit: "abc1234",
  is_current: false,
  ahead: 1,
  behind: 0,
  divergence_status: "Ahead",
  last_tool_usage: null,
  is_agent_running: false,
  commit_timestamp: 1_700_000_200,
};

const branchMain = {
  ...branchFeature,
  name: "main",
  is_current: true,
  divergence_status: "UpToDate",
  ahead: 0,
  commit_timestamp: 1_700_000_250,
};

const settingsFixture = {
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
  ui_font_family:
    'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
  terminal_font_family:
    '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace',
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
  default_shell: null,
};

const profilesFixture = {
  version: 1,
  active: "default",
  profiles: {
    default: {
      name: "default",
      description: "",
      env: {},
      disabled_env: [],
      ai_enabled: false,
      ai: null,
    },
  },
};

async function setMockCommandResponses(
  page: Page,
  commandResponses: Record<string, unknown>,
) {
  await page.evaluate((responses) => {
    (
      window as unknown as {
        __GWT_MOCK_COMMAND_RESPONSES__?: Record<string, unknown>;
      }
    ).__GWT_MOCK_COMMAND_RESPONSES__ = responses;
  }, commandResponses);
}

async function dismissSkillRegistrationScopeDialogIfPresent(_page: Page) {
  // No-op: scope dialog removed after scope simplification.
}

async function openProjectAndSelectBranch(
  page: Page,
  commandResponses: Record<string, unknown>,
) {
  await page.goto("/");
  await setMockCommandResponses(page, commandResponses);

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await page.locator("button.recent-item").first().click();
  await expect(
    page.getByPlaceholder("Type a message..."),
  ).toBeVisible();

  const branchButton = page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name });
  await expect(branchButton).toBeVisible();
  await branchButton.click();

  await expect(page.locator(".branch-detail h2")).toContainText(
    branchFeature.name,
  );
}

async function waitForMenuActionListener(page: Page) {
  await expect
    .poll(async () => {
      return page.evaluate(() => {
        const globalWindow = window as unknown as {
          __GWT_TAURI_INVOKE_LOG__?: Array<{
            cmd: string;
            args?: { event?: string };
          }>;
        };
        return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).some(
          (entry) =>
            entry.cmd === "plugin:event|listen" &&
            entry.args?.event === "menu-action",
        );
      });
    })
    .toBe(true);
}

async function waitForInvokeCommand(page: Page, cmd: string) {
  await expect
    .poll(async () => {
      return page.evaluate((targetCmd) => {
        const globalWindow = window as unknown as {
          __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
        };
        return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).some(
          (entry) => entry.cmd === targetCmd,
        );
      }, cmd);
    })
    .toBe(true);
}

async function openSettings(
  page: Page,
  commandResponses: Record<string, unknown>,
) {
  await openProjectAndSelectBranch(page, commandResponses);

  await waitForMenuActionListener(page);
  await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_MOCK_EMIT_EVENT__?: (event: string, payload: unknown) => void;
    };
    globalWindow.__GWT_MOCK_EMIT_EVENT__?.("menu-action", {
      action: "open-settings",
    });
  });

  await expect(
    page.getByRole("heading", { name: "Settings" }),
  ).toBeVisible();
}

async function openVoiceInputTab(page: Page) {
  await page.getByRole("button", { name: "Voice Input", exact: true }).click();
}

async function enableVoiceInput(page: Page) {
  const enabledCheckbox = page.locator("#voice-input-enabled");
  await expect(enabledCheckbox).toBeVisible();
  if (!(await enabledCheckbox.isChecked())) {
    await enabledCheckbox.check();
  }
}

const sharedCommandResponses = {
  list_worktree_branches: [branchMain, branchFeature],
  list_remote_branches: [],
  list_worktrees: [],
  get_settings: settingsFixture,
  get_profiles: profilesFixture,
  get_voice_capability: {
    available: false,
    reason: "GPU acceleration is not available",
  },
};

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("voice input fields are enabled when capability is unavailable", async ({
  page,
}) => {
  await openSettings(page, sharedCommandResponses);

  await openVoiceInputTab(page);
  await enableVoiceInput(page);

  const enabledCheckbox = page.locator("#voice-input-enabled");
  const hotkeyInput = page.locator("#voice-hotkey");
  const pttHotkeyInput = page.locator("#voice-ptt-hotkey");
  const languageSelect = page.locator("#voice-language");
  const qualitySelect = page.locator("#voice-quality");

  await expect(enabledCheckbox).toBeEnabled();
  await expect(hotkeyInput).toBeEnabled();
  await expect(pttHotkeyInput).toBeEnabled();
  await expect(languageSelect).toBeEnabled();
  await expect(qualitySelect).toBeEnabled();
});

test("voice input settings can be changed and saved when capability is unavailable", async ({
  page,
}) => {
  await openSettings(page, sharedCommandResponses);

  await openVoiceInputTab(page);
  await enableVoiceInput(page);

  await expect(page.locator("#voice-hotkey")).toBeEnabled();

  await page.locator("#voice-hotkey").fill("Ctrl+Shift+V");
  await page.locator("#voice-language").selectOption("ja");

  await page.getByRole("button", { name: "Save" }).click();
  await waitForInvokeCommand(page, "save_settings");

  const savedSettings = await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{
        cmd: string;
        args?: { settings?: Record<string, unknown> };
      }>;
    };
    const log = globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? [];
    const entry = [...log]
      .reverse()
      .find((item) => item.cmd === "save_settings");
    return entry?.args?.settings ?? null;
  });

  expect(savedSettings).not.toBeNull();
  const voiceInput = savedSettings?.voice_input as Record<string, unknown>;
  expect(voiceInput?.hotkey).toBe("Ctrl+Shift+V");
  expect(voiceInput?.language).toBe("ja");
});

test("shows unavailable reason banner with settings-still-configurable note", async ({
  page,
}) => {
  await openSettings(page, sharedCommandResponses);

  await openVoiceInputTab(page);
  await enableVoiceInput(page);

  await expect(
    page.getByText("GPU acceleration is not available"),
  ).toBeVisible();
  await expect(
    page.getByText(/Settings can still be configured/),
  ).toBeVisible();
});
