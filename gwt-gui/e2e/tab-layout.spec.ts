import { expect, test, type Page } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  emitTauriEvent,
  openRecentProject,
  waitForMenuActionListener,
} from "./support/helpers";

async function openSettingsTab(page: Page) {
  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "open-settings" });
  await expect(page.locator('[data-tab-id="settings"]')).toBeVisible();
}

async function createDataTransfer(page: Page) {
  return page.evaluateHandle(() => new DataTransfer());
}

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
  await page.goto("/");
  await openRecentProject(page);
  await openSettingsTab(page);
});

test("explicit split action creates a second group and drag merge collapses back", async ({
  page,
}) => {
  const settingsTab = page.locator('[data-tab-id="settings"]').first();
  await settingsTab.locator(".tab-actions-toggle").click();
  await settingsTab
    .locator(".tab-actions-menu")
    .getByRole("button", { name: "Split Right" })
    .click();

  await expect(page.locator(".group-pane")).toHaveCount(2);

  const dataTransfer = await createDataTransfer(page);
  const sourceTab = page.locator(".group-pane").nth(1).locator('[data-tab-id="settings"]');
  const targetTabBar = page.locator(".group-pane").nth(0).locator(".tab-bar");

  await sourceTab.dispatchEvent("dragstart", { dataTransfer });
  await targetTabBar.dispatchEvent("dragover", { dataTransfer });
  await targetTabBar.dispatchEvent("drop", { dataTransfer });

  await expect(page.locator(".group-pane")).toHaveCount(1);
  await expect(page.locator(".group-pane").first().locator('[data-tab-id="settings"]')).toBeVisible();
});

test("dragging a tab onto a split target creates a new group", async ({ page }) => {
  const settingsTab = page.locator('[data-tab-id="settings"]').first();
  const splitTarget = page.locator(".group-pane").first().locator(".split-target-right");
  const dataTransfer = await createDataTransfer(page);

  await settingsTab.dispatchEvent("dragstart", { dataTransfer });
  await splitTarget.dispatchEvent("dragover", { dataTransfer });
  await splitTarget.dispatchEvent("drop", { dataTransfer });
  await settingsTab.dispatchEvent("dragend", { dataTransfer });

  await expect(page.locator(".group-pane")).toHaveCount(2);
});

test("group-local tabs use a compact fixed width", async ({ page }) => {
  const assistantTab = page.locator('[data-tab-id="assistant"]').first();
  const settingsTab = page.locator('[data-tab-id="settings"]').first();

  const assistantWidth = await assistantTab.evaluate((element) =>
    element.getBoundingClientRect().width,
  );
  const settingsWidth = await settingsTab.evaluate((element) =>
    element.getBoundingClientRect().width,
  );

  expect(assistantWidth).toBeGreaterThanOrEqual(175);
  expect(assistantWidth).toBeLessThanOrEqual(185);
  expect(settingsWidth).toBeGreaterThanOrEqual(175);
  expect(settingsWidth).toBeLessThanOrEqual(185);
});
