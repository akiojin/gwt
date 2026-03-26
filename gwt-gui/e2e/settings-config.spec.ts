import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  captureUxSnapshot,
  defaultRecentProject,
  settingsFixture,
  profilesFixture,
  getInvokeLog,
  openRecentProject,
  setMockCommandResponses,
  waitForInvokeCommand,
  openSettings,
  standardSettingsResponses,
  getInvokeArgs,
  saveE2ECoverage,
} from "./support/helpers";

function generalTab(page: Parameters<typeof test>[0]["page"]) {
  return page.getByRole("button", { name: "General", exact: true });
}

function profilesTab(page: Parameters<typeof test>[0]["page"]) {
  return page.getByRole("button", { name: "Profiles", exact: true });
}

function terminalTab(page: Parameters<typeof test>[0]["page"]) {
  return page.getByRole("button", { name: "Terminal", exact: true });
}

function voiceInputTab(page: Parameters<typeof test>[0]["page"]) {
  return page.getByRole("button", { name: "Voice Input", exact: true });
}

function fieldByLabel(page: Parameters<typeof test>[0]["page"], label: RegExp) {
  return page
    .locator(".field")
    .filter({ has: page.locator("label", { hasText: label }) })
    .first();
}

function uiFontSizeInput(page: Parameters<typeof test>[0]["page"]) {
  return fieldByLabel(page, /^UI font size$/i).locator('input[type="number"]');
}

function terminalFontSizeInput(page: Parameters<typeof test>[0]["page"]) {
  return fieldByLabel(page, /^Terminal font size$/i).locator(
    'input[type="number"]',
  );
}

function profileSelect(page: Parameters<typeof test>[0]["page"]) {
  return page.locator(".profile-select");
}

function profileDeleteButton(page: Parameters<typeof test>[0]["page"]) {
  return page
    .locator(".profile-header")
    .getByRole("button", { name: "Delete", exact: true });
}

function profileNewButton(page: Parameters<typeof test>[0]["page"]) {
  return page
    .locator(".profile-header")
    .getByRole("button", { name: "+ New", exact: true });
}

function apiKeyField(page: Parameters<typeof test>[0]["page"]) {
  return fieldByLabel(page, /^API key$/i);
}

function apiKeyInput(page: Parameters<typeof test>[0]["page"]) {
  return apiKeyField(page).locator(".ai-apikey-row input");
}

function voiceEnabledSettings() {
  return {
    ...settingsFixture,
    voice_input: {
      ...settingsFixture.voice_input,
      enabled: true,
    },
  };
}

async function enableVoiceInput(page: Parameters<typeof test>[0]["page"]) {
  const checkbox = page.locator("#voice-input-enabled");
  await expect(checkbox).toBeVisible();
  if (!(await checkbox.isChecked())) {
    await checkbox.check();
  }
}

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test.afterEach(async ({ page }, testInfo) => {
  await saveE2ECoverage(page, testInfo);
});

test("opens Settings panel from menu action", async ({ page }, testInfo) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await captureUxSnapshot(page, testInfo, "settings-general-tab");
});

test("Settings General tab is active by default", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());
  await expect(generalTab(page)).toHaveClass(/active/);
});

test("Settings shows UI Font Size field", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());
  await expect(fieldByLabel(page, /^UI font size$/i)).toBeVisible();
});

test("Settings shows Terminal Font Size field", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());
  await terminalTab(page).click();
  await expect(fieldByLabel(page, /^Terminal font size$/i)).toBeVisible();
});

test("changes UI font size via spinbutton and saves", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await expect(uiFontSizeInput(page)).toBeVisible();
  await uiFontSizeInput(page).fill("16");

  await page.getByRole("button", { name: "Save" }).click();
  await waitForInvokeCommand(page, "save_settings");

  const args = await getInvokeArgs(page, "save_settings");
  const settings = (args as Record<string, unknown>)?.settings as Record<
    string,
    unknown
  >;
  expect(settings?.ui_font_size).toBe(16);
});

test("changes terminal font size via spinbutton and saves", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await terminalTab(page).click();
  await expect(terminalFontSizeInput(page)).toBeVisible();
  await terminalFontSizeInput(page).fill("18");

  await page.getByRole("button", { name: "Save" }).click();
  await waitForInvokeCommand(page, "save_settings");

  const args = await getInvokeArgs(page, "save_settings");
  const settings = (args as Record<string, unknown>)?.settings as Record<
    string,
    unknown
  >;
  expect(settings?.terminal_font_size).toBe(18);
});

test("Voice Input tab shows fields", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await voiceInputTab(page).click();

  await expect(page.locator("#voice-input-enabled")).toBeVisible();
  await enableVoiceInput(page);
  await expect(page.getByText(/Cmd\+Shift\+Space/)).toBeVisible();
  await expect(page.locator("#voice-language")).toBeVisible();
});

test("Voice Input language can be changed", async ({ page }) => {
  await page.goto("/");
  await openSettings(
    page,
    standardSettingsResponses({ get_settings: voiceEnabledSettings() }),
  );

  await voiceInputTab(page).click();

  await page.locator("#voice-language").selectOption("ja");
  await page.getByRole("button", { name: "Save" }).click();

  await waitForInvokeCommand(page, "save_settings");

  const args = await getInvokeArgs(page, "save_settings");
  const settings = (args as Record<string, unknown>)?.settings as Record<
    string,
    unknown
  >;
  const voiceInput = settings?.voice_input as Record<string, unknown>;
  expect(voiceInput?.language).toBe("ja");
});

test("Settings close button returns to branch view", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await page
    .locator(".settings-footer .btn-cancel", { hasText: "Close" })
    .click();

  await expect(page.getByRole("heading", { name: "Settings" })).toBeHidden();
});

test("Profiles tab shows default profile", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await profilesTab(page).click();

  await expect(profileSelect(page)).toHaveValue("default");
});

test("default profile delete button is disabled", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await profilesTab(page).click();

  const deleteButton = profileDeleteButton(page);
  await expect(profileSelect(page)).toHaveValue("default");
  await expect(deleteButton).toBeDisabled();

  const invokeLog = await getInvokeLog(page);
  expect(invokeLog).not.toContain("save_profiles");
});

test("Profiles tab shows selector and profile actions from the refactored header", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await profilesTab(page).click();

  await expect(profileSelect(page)).toBeVisible();
  await expect(page.locator("#profile-edit")).toHaveCount(0);
  await expect(profileDeleteButton(page)).toBeVisible();
  await expect(profileNewButton(page)).toBeVisible();
  await captureUxSnapshot(page, testInfo, "settings-profiles-tab");
});

test("creating a profile makes it active and save_profiles persists active profile edits", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await profilesTab(page).click();

  await profileNewButton(page).click();
  await page
    .getByRole("dialog", { name: "Create Profile" })
    .getByLabel("Profile Name")
    .fill("staging");
  await page
    .getByRole("dialog", { name: "Create Profile" })
    .getByRole("button", { name: "Create", exact: true })
    .click();
  await expect(profileSelect(page)).toHaveValue("staging");

  await page.locator(".env-add-row .env-key-input").fill("STAGE_KEY");
  await page.locator(".env-add-row .env-value-input").fill("stage-value");
  await page.locator(".env-add-row .btn-add").click();

  await expect(
    page.locator(".env-row").filter({ hasText: "STAGE_KEY" }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Save" }).click();
  await waitForInvokeCommand(page, "save_profiles");

  const args = await getInvokeArgs(page, "save_profiles");
  const config = (args as Record<string, unknown>)?.config as Record<
    string,
    unknown
  >;
  const profiles = config?.profiles as Record<string, unknown>;
  const staging = profiles?.staging as Record<string, unknown>;
  const stagingEnv = staging?.env as Record<string, unknown>;

  expect(config?.active).toBe("staging");
  expect(profiles?.default).toBeTruthy();
  expect(stagingEnv?.STAGE_KEY).toBe("stage-value");
});

test("deleting active non-default profile falls back to default", async ({
  page,
}) => {
  const twoProfiles = {
    ...profilesFixture,
    active: "dev",
    profiles: {
      ...profilesFixture.profiles,
      dev: {
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
      },
    },
  };

  await page.goto("/");
  await openSettings(
    page,
    standardSettingsResponses({ get_profiles: twoProfiles }),
  );

  await profilesTab(page).click();

  await expect(profileSelect(page)).toHaveValue("dev");
  await profileDeleteButton(page).click();
  await page
    .getByRole("dialog", { name: "Delete Profile" })
    .getByRole("button", { name: "Delete", exact: true })
    .click();

  await expect(profileSelect(page)).toHaveValue("default");
  await expect(profileSelect(page).locator("option[value='dev']")).toHaveCount(
    0,
  );
});

test("Profiles tab shows API key peek/copy controls and preserves underscore key on peek", async ({
  page,
}) => {
  const profilesWithApiKey = {
    ...profilesFixture,
    profiles: {
      ...profilesFixture.profiles,
      default: {
        ...profilesFixture.profiles.default,
        ai_enabled: true,
        ai: {
          endpoint: "https://api.openai.com/v1",
          api_key: "sk_test_ab_cd",
          model: "gpt-4o-mini",
          language: "en",
        },
      },
    },
  };

  await page.goto("/");
  await openSettings(
    page,
    standardSettingsResponses({ get_profiles: profilesWithApiKey }),
  );

  await profilesTab(page).click();

  const keyField = apiKeyField(page);
  const keyInput = apiKeyInput(page);
  const peekButton = keyField.locator(".btn-peek-apikey");
  const copyButton = keyField.locator(".btn-copy-apikey");

  await expect(peekButton).toBeVisible();
  await expect(copyButton).toBeVisible();
  await expect(peekButton.locator("svg")).toBeVisible();
  await expect(copyButton.locator("svg")).toBeVisible();
  await expect(keyInput).toHaveAttribute("type", "text");
  await expect(keyInput).toHaveClass(/\bapi-key-masked\b/);

  await peekButton.dispatchEvent("mousedown");
  await expect(keyInput).toHaveAttribute("type", "text");
  await expect(keyInput).not.toHaveClass(/\bapi-key-masked\b/);
  await expect(keyInput).toHaveValue("sk_test_ab_cd");

  await peekButton.dispatchEvent("mouseup");
  await expect(keyInput).toHaveAttribute("type", "text");
  await expect(keyInput).toHaveClass(/\bapi-key-masked\b/);
});

test("copy API key button writes plaintext value and shows copied feedback", async ({
  page,
}) => {
  const profilesWithApiKey = {
    ...profilesFixture,
    profiles: {
      ...profilesFixture.profiles,
      default: {
        ...profilesFixture.profiles.default,
        ai_enabled: true,
        ai: {
          endpoint: "https://api.openai.com/v1",
          api_key: "sk_test_ab_cd",
          model: "gpt-4o-mini",
          language: "en",
        },
      },
    },
  };

  await page.addInitScript(() => {
    const globalWindow = window as unknown as {
      __GWT_E2E_COPIED_API_KEY__?: string;
    };
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          globalWindow.__GWT_E2E_COPIED_API_KEY__ = text;
        },
      },
    });
  });

  await page.goto("/");
  await openSettings(
    page,
    standardSettingsResponses({ get_profiles: profilesWithApiKey }),
  );

  await profilesTab(page).click();

  const copyButton = apiKeyField(page).locator(".btn-copy-apikey");

  await expect(copyButton).toBeVisible();
  await copyButton.click();

  await expect(copyButton).toHaveAttribute("title", "Copied!");
  await expect(copyButton).toHaveClass(/copied/);

  const copiedValue = await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_E2E_COPIED_API_KEY__?: string;
    };
    return globalWindow.__GWT_E2E_COPIED_API_KEY__ ?? null;
  });
  expect(copiedValue).toBe("sk_test_ab_cd");
});

test("Profiles API key value with underscores is persisted on save_profiles", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await profilesTab(page).click();

  const apiKeyValue = "sk_test_ab_cd";
  await apiKeyInput(page).fill(apiKeyValue);

  await page.getByRole("button", { name: "Save" }).click();
  await waitForInvokeCommand(page, "save_profiles");

  const args = await getInvokeArgs(page, "save_profiles");
  const config = (args as Record<string, unknown>)?.config as Record<
    string,
    unknown
  >;
  const profiles = config?.profiles as Record<string, unknown>;
  const defaultProfile = profiles?.default as Record<string, unknown>;
  const ai = defaultProfile?.ai as Record<string, unknown>;

  expect(ai?.api_key).toBe(apiKeyValue);
});

test("Profiles API key peek and copy buttons appear after typing", async ({
  page,
}) => {
  const profilesWithAi = {
    ...profilesFixture,
    profiles: {
      ...profilesFixture.profiles,
      default: {
        ...profilesFixture.profiles.default,
        ai_enabled: true,
        ai: {
          endpoint: "https://api.openai.com/v1",
          api_key: "",
          model: "",
          language: "en",
        },
      },
    },
  };

  await page.goto("/");
  await openSettings(
    page,
    standardSettingsResponses({ get_profiles: profilesWithAi }),
  );

  await profilesTab(page).click();

  const keyField = apiKeyField(page);
  await expect(keyField.locator(".btn-peek-apikey")).toBeHidden();
  await expect(keyField.locator(".btn-copy-apikey")).toBeHidden();
  await apiKeyInput(page).fill("sk-typed-key");

  await expect(keyField.locator(".btn-peek-apikey")).toBeVisible();
  await expect(keyField.locator(".btn-copy-apikey")).toBeVisible();
});

test("Profiles API key typed value is sent to list_ai_models on Refresh", async ({
  page,
}) => {
  const profilesWithAi = {
    ...profilesFixture,
    profiles: {
      ...profilesFixture.profiles,
      default: {
        ...profilesFixture.profiles.default,
        ai_enabled: true,
        ai: {
          endpoint: "https://api.openai.com/v1",
          api_key: "",
          model: "",
          language: "en",
        },
      },
    },
  };

  await page.goto("/");
  await openSettings(
    page,
    standardSettingsResponses({ get_profiles: profilesWithAi }),
  );

  await profilesTab(page).click();

  await apiKeyInput(page).fill("sk-refresh-check-123");

  await page.getByRole("button", { name: "Refresh" }).click();
  await waitForInvokeCommand(page, "list_ai_models");

  const args = await getInvokeArgs(page, "list_ai_models");
  expect(args).toMatchObject({
    endpoint: "https://api.openai.com/v1",
    apiKey: "sk-refresh-check-123",
  });
});

test("Profiles pasted API key shows actions and is sent to list_ai_models on Refresh", async ({
  page,
}) => {
  const profilesWithAi = {
    ...profilesFixture,
    profiles: {
      ...profilesFixture.profiles,
      default: {
        ...profilesFixture.profiles.default,
        ai_enabled: true,
        ai: {
          endpoint: "https://api.openai.com/v1",
          api_key: "",
          model: "",
          language: "en",
        },
      },
    },
  };

  await page.goto("/");
  await openSettings(
    page,
    standardSettingsResponses({ get_profiles: profilesWithAi }),
  );

  await profilesTab(page).click();

  await apiKeyInput(page).evaluate((input, pasted) => {
    const element = input as HTMLInputElement;
    element.focus();
    const cursor = element.value.length;
    element.setSelectionRange(cursor, cursor);
    const pasteEvent = new Event("paste", {
      bubbles: true,
      cancelable: true,
    }) as ClipboardEvent;
    Object.defineProperty(pasteEvent, "clipboardData", {
      value: {
        getData: (type: string) => (type === "text/plain" ? pasted : ""),
      },
      configurable: true,
    });
    element.dispatchEvent(pasteEvent);
  }, "sk-pasted-e2e-refresh");

  await expect(apiKeyField(page).locator(".btn-peek-apikey")).toBeVisible();
  await expect(apiKeyField(page).locator(".btn-copy-apikey")).toBeVisible();

  await page.getByRole("button", { name: "Refresh" }).click();
  await waitForInvokeCommand(page, "list_ai_models");

  const args = await getInvokeArgs(page, "list_ai_models");
  expect(args).toMatchObject({
    endpoint: "https://api.openai.com/v1",
    apiKey: "sk-pasted-e2e-refresh",
  });
});

test("Profiles pasted API key is sent to save_profiles on Save", async ({
  page,
}) => {
  const profilesWithAi = {
    ...profilesFixture,
    profiles: {
      ...profilesFixture.profiles,
      default: {
        ...profilesFixture.profiles.default,
        ai_enabled: true,
        ai: {
          endpoint: "https://api.openai.com/v1",
          api_key: "",
          model: "",
          language: "en",
        },
      },
    },
  };

  await page.goto("/");
  await openSettings(
    page,
    standardSettingsResponses({ get_profiles: profilesWithAi }),
  );

  await profilesTab(page).click();

  await apiKeyInput(page).evaluate((input, pasted) => {
    const element = input as HTMLInputElement;
    element.focus();
    const cursor = element.value.length;
    element.setSelectionRange(cursor, cursor);
    const pasteEvent = new Event("paste", {
      bubbles: true,
      cancelable: true,
    }) as ClipboardEvent;
    Object.defineProperty(pasteEvent, "clipboardData", {
      value: {
        getData: (type: string) => (type === "text/plain" ? pasted : ""),
      },
      configurable: true,
    });
    element.dispatchEvent(pasteEvent);
  }, "sk-pasted-e2e-save");

  await page.getByRole("button", { name: "Save" }).click();
  await waitForInvokeCommand(page, "save_profiles");

  const args = await getInvokeArgs(page, "save_profiles");
  const config = (args as Record<string, unknown>)?.config as Record<
    string,
    unknown
  >;
  const profiles = config?.profiles as Record<string, unknown>;
  const defaultProfile = profiles?.default as Record<string, unknown>;
  const ai = defaultProfile?.ai as Record<string, unknown>;
  expect(ai?.api_key).toBe("sk-pasted-e2e-save");
});

test("UI Font Family selector shows presets", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  const selector = page.getByLabel("UI font family");
  await expect(selector).toBeVisible();
  const options = selector.locator("option");
  await expect(options).not.toHaveCount(0);
});

test("Terminal Font Family selector shows presets", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await terminalTab(page).click();
  const selector = page.getByLabel("Terminal font family");
  await expect(selector).toBeVisible();
  const options = selector.locator("option");
  await expect(options).not.toHaveCount(0);
});

test("changing UI font family updates CSS variable preview", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  const interValue =
    '"Inter", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';
  await page.getByLabel("UI font family").selectOption(interValue);

  const uiFont = await page.evaluate(() =>
    getComputedStyle(document.documentElement)
      .getPropertyValue("--ui-font-family")
      .trim(),
  );
  expect(uiFont).toBe(interValue);
});

test("closing Settings without saving restores original font preview", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  const originalFont = settingsFixture.ui_font_family;
  const interValue =
    '"Inter", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';

  await page.getByLabel("UI font family").selectOption(interValue);

  await page
    .locator(".settings-footer .btn-cancel", { hasText: "Close" })
    .click();

  const restored = await page.evaluate(() =>
    getComputedStyle(document.documentElement)
      .getPropertyValue("--ui-font-family")
      .trim(),
  );
  expect(restored).toBe(originalFont);
});

test("save button is present in settings panel", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await expect(page.getByRole("button", { name: "Save" })).toBeVisible();
});

test("navigating between settings tabs preserves changed values", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  // Change UI font family in Appearance
  const interValue =
    '"Inter", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';
  await page.getByLabel("UI font family").selectOption(interValue);

  // Navigate to Voice Input and back
  await voiceInputTab(page).click();
  await generalTab(page).click();

  // Value should be preserved
  await expect(page.getByLabel("UI font family")).toHaveValue(interValue);
});

test("Voice Input unavailable reason banner is shown", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, {
    ...standardSettingsResponses(),
    get_settings: voiceEnabledSettings(),
    get_voice_capability: {
      available: false,
      reason: "GPU acceleration is not available",
    },
  });

  await voiceInputTab(page).click();

  await expect(
    page.getByText("GPU acceleration is not available"),
  ).toBeVisible();
});

test("Log Retention days field is visible", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());
  await expect(page.getByLabel("Log retention")).toBeVisible();
});

test("Protected Branches section is visible", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());
  await expect(page.getByText("Protected branches")).toBeVisible();
  await expect(
    page.locator(".branch-tags .branch-tag").filter({ hasText: "main" }),
  ).toBeVisible();
  await expect(
    page.locator(".branch-tags .branch-tag").filter({ hasText: "develop" }),
  ).toBeVisible();
});
