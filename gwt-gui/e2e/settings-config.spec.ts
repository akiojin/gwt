import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  settingsFixture,
  profilesFixture,
  openRecentProject,
  setMockCommandResponses,
  waitForInvokeCommand,
  openSettings,
  standardSettingsResponses,
  getInvokeArgs,
} from "./support/helpers";

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("opens Settings panel from menu action", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());
  await expect(
    page.getByRole("heading", { name: "Settings" }),
  ).toBeVisible();
});

test("Settings Appearance tab is active by default", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());
  await expect(
    page.getByRole("button", { name: "Appearance", exact: true }),
  ).toHaveClass(/active/);
});

test("Settings shows UI Font Size field", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());
  await expect(page.getByText("UI Font Size")).toBeVisible();
});

test("Settings shows Terminal Font Size field", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());
  await expect(page.getByText("Terminal Font Size")).toBeVisible();
});

test("changes UI font size via spinbutton and saves", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  // Find the UI Font Size spinbutton and clear + fill
  const uiFontSpinbutton = page
    .locator(".settings-tab-content")
    .getByRole("spinbutton")
    .nth(1);
  await expect(uiFontSpinbutton).toBeVisible();
  await uiFontSpinbutton.fill("16");

  await page.getByRole("button", { name: "Save" }).click();
  await waitForInvokeCommand(page, "save_settings");

  const args = await getInvokeArgs(page, "save_settings");
  const settings = (args as Record<string, unknown>)
    ?.settings as Record<string, unknown>;
  expect(settings?.ui_font_size).toBe(16);
});

test("changes terminal font size via spinbutton and saves", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  // Terminal Font Size is the first spinbutton
  const termFontSpinbutton = page
    .locator(".settings-tab-content")
    .getByRole("spinbutton")
    .nth(0);
  await expect(termFontSpinbutton).toBeVisible();
  await termFontSpinbutton.fill("18");

  await page.getByRole("button", { name: "Save" }).click();
  await waitForInvokeCommand(page, "save_settings");

  const args = await getInvokeArgs(page, "save_settings");
  const settings = (args as Record<string, unknown>)
    ?.settings as Record<string, unknown>;
  expect(settings?.terminal_font_size).toBe(18);
});

test("Voice Input tab shows fields", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await page
    .getByRole("button", { name: "Voice Input", exact: true })
    .click();

  await expect(page.locator("#voice-input-enabled")).toBeVisible();
  await expect(page.locator("#voice-hotkey")).toBeVisible();
  await expect(page.locator("#voice-language")).toBeVisible();
});

test("Voice Input hotkey can be changed", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await page
    .getByRole("button", { name: "Voice Input", exact: true })
    .click();

  await page.locator("#voice-hotkey").fill("Ctrl+Shift+V");
  await page.getByRole("button", { name: "Save" }).click();

  await waitForInvokeCommand(page, "save_settings");

  const args = await getInvokeArgs(page, "save_settings");
  const settings = (args as Record<string, unknown>)
    ?.settings as Record<string, unknown>;
  const voiceInput = settings?.voice_input as Record<string, unknown>;
  expect(voiceInput?.hotkey).toBe("Ctrl+Shift+V");
});

test("Voice Input language can be changed", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await page
    .getByRole("button", { name: "Voice Input", exact: true })
    .click();

  await page.locator("#voice-language").selectOption("ja");
  await page.getByRole("button", { name: "Save" }).click();

  await waitForInvokeCommand(page, "save_settings");

  const args = await getInvokeArgs(page, "save_settings");
  const settings = (args as Record<string, unknown>)
    ?.settings as Record<string, unknown>;
  const voiceInput = settings?.voice_input as Record<string, unknown>;
  expect(voiceInput?.language).toBe("ja");
});

test("Settings close button returns to branch view", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await page
    .locator(".settings-footer .btn-cancel", { hasText: "Close" })
    .click();

  await expect(
    page.getByRole("heading", { name: "Settings" }),
  ).toBeHidden();
});

test("Profiles tab shows default profile", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  await page
    .getByRole("button", { name: "Profiles", exact: true })
    .click();

  await expect(page.locator("#active-profile")).toHaveValue("default");
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
          summary_enabled: true,
        },
      },
    },
  };

  await page.goto("/");
  await openSettings(page, standardSettingsResponses({ get_profiles: profilesWithApiKey }));

  await page
    .getByRole("button", { name: "Profiles", exact: true })
    .click();

  const apiKeyField = page.locator(".ai-field").filter({ hasText: "API Key" });
  const apiKeyInput = apiKeyField.locator("input").first();
  const peekButton = apiKeyField.locator(".btn-peek-apikey");
  const copyButton = apiKeyField.locator(".btn-copy-apikey");

  await expect(peekButton).toBeVisible();
  await expect(copyButton).toBeVisible();
  await expect(peekButton.locator("svg")).toBeVisible();
  await expect(copyButton.locator("svg")).toBeVisible();
  await expect(apiKeyInput).toHaveAttribute("type", "password");

  await peekButton.dispatchEvent("mousedown");
  await expect(apiKeyInput).toHaveAttribute("type", "text");
  await expect(apiKeyInput).toHaveValue("sk_test_ab_cd");

  await peekButton.dispatchEvent("mouseup");
  await expect(apiKeyInput).toHaveAttribute("type", "password");
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
          summary_enabled: true,
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
  await openSettings(page, standardSettingsResponses({ get_profiles: profilesWithApiKey }));

  await page
    .getByRole("button", { name: "Profiles", exact: true })
    .click();

  const apiKeyField = page.locator(".ai-field").filter({ hasText: "API Key" });
  const copyButton = apiKeyField.locator(".btn-copy-apikey");

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

test("UI Font Family selector shows presets", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  const selector = page.getByLabel("UI Font Family");
  await expect(selector).toBeVisible();
  const options = selector.locator("option");
  await expect(options).not.toHaveCount(0);
});

test("Terminal Font Family selector shows presets", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());

  const selector = page.getByLabel("Terminal Font Family");
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
  await page.getByLabel("UI Font Family").selectOption(interValue);

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

  await page.getByLabel("UI Font Family").selectOption(interValue);

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
  await page.getByLabel("UI Font Family").selectOption(interValue);

  // Navigate to Voice Input and back
  await page
    .getByRole("button", { name: "Voice Input", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Appearance", exact: true })
    .click();

  // Value should be preserved
  await expect(page.getByLabel("UI Font Family")).toHaveValue(interValue);
});

test("Voice Input unavailable reason banner is shown", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, {
    ...standardSettingsResponses(),
    get_voice_capability: {
      available: false,
      reason: "GPU acceleration is not available",
    },
  });

  await page
    .getByRole("button", { name: "Voice Input", exact: true })
    .click();

  await expect(
    page.getByText("GPU acceleration is not available"),
  ).toBeVisible();
});

test("Log Retention days field is visible", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());
  await expect(page.getByText("Log Retention (days)")).toBeVisible();
});

test("Protected Branches section is visible", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, standardSettingsResponses());
  await expect(page.getByText("Protected Branches")).toBeVisible();
  await expect(
    page.locator(".branch-tags .branch-tag").filter({ hasText: "main" }),
  ).toBeVisible();
  await expect(
    page.locator(".branch-tags .branch-tag").filter({ hasText: "develop" }),
  ).toBeVisible();
});
