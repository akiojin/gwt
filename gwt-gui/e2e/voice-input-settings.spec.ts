import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  getInvokeArgs,
  openSettings,
  profilesFixture,
  settingsFixture,
  standardBranchResponses,
  waitForInvokeCommand,
} from "./support/helpers";

function voiceInputTab(page: Parameters<typeof test>[0]["page"]) {
  return page.getByRole("button", { name: "Voice Input", exact: true });
}

async function enableVoiceInput(page: Parameters<typeof test>[0]["page"]) {
  const enabledCheckbox = page.locator("#voice-input-enabled");
  await expect(enabledCheckbox).toBeVisible();
  if (!(await enabledCheckbox.isChecked())) {
    await enabledCheckbox.check();
  }
}

const sharedCommandResponses = {
  ...standardBranchResponses(),
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
  await page.goto("/");
  await openSettings(page, sharedCommandResponses);

  await voiceInputTab(page).click();
  await enableVoiceInput(page);

  const enabledCheckbox = page.locator("#voice-input-enabled");
  const languageSelect = page.locator("#voice-language");
  const qualitySelect = page.locator("#voice-quality");

  await expect(enabledCheckbox).toBeEnabled();
  await expect(languageSelect).toBeEnabled();
  await expect(qualitySelect).toBeEnabled();
  await expect(page.getByText(/Cmd\+Shift\+Space/)).toBeVisible();
});

test("voice input settings can be changed and saved when capability is unavailable", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(page, sharedCommandResponses);

  await voiceInputTab(page).click();
  await enableVoiceInput(page);

  await page.locator("#voice-language").selectOption("ja");

  await page.getByRole("button", { name: "Save" }).click();
  await waitForInvokeCommand(page, "save_settings");

  const args = await getInvokeArgs(page, "save_settings");
  const settings = (args as Record<string, unknown>)?.settings as Record<
    string,
    unknown
  >;

  expect(settings).not.toBeNull();
  const voiceInput = settings?.voice_input as Record<string, unknown>;
  expect(voiceInput?.language).toBe("ja");
});

test("shows unavailable reason banner with settings-still-configurable note", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(page, sharedCommandResponses);

  await voiceInputTab(page).click();
  await enableVoiceInput(page);

  await expect(
    page.getByText("GPU acceleration is not available"),
  ).toBeVisible();
  await expect(
    page.getByText(/Settings can still be configured/),
  ).toBeVisible();
});
