import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  openSettings,
  saveE2ECoverage,
  standardSettingsResponses,
} from "./support/helpers";

const syncResultDiff = {
  syncType: "diff",
  updatedCount: 5,
  deletedCount: 0,
  durationMs: 123,
  completedAt: 1710000000000,
  error: null,
};

const syncResultFull = {
  syncType: "full",
  updatedCount: 20,
  deletedCount: 3,
  durationMs: 456,
  completedAt: 1710000000000,
  error: null,
};

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

test("Issue cache section is visible in Settings General tab", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(
    page,
    standardSettingsResponses({
      sync_issue_cache: syncResultDiff,
    }),
  );

  await expect(page.getByText("Issue cache")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Diff Sync" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Full Sync" }),
  ).toBeVisible();
});

test("Diff Sync button click triggers sync and shows result", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(
    page,
    standardSettingsResponses({
      sync_issue_cache: syncResultDiff,
    }),
  );

  await page.getByRole("button", { name: "Diff Sync" }).click();

  await expect(page.getByText(/Updated: 5/)).toBeVisible();
  await expect(page.getByText(/Deleted: 0/)).toBeVisible();
  await expect(page.getByText(/Duration: 123ms/)).toBeVisible();
});

test("Full Sync button click triggers sync and shows result", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(
    page,
    standardSettingsResponses({
      sync_issue_cache: syncResultFull,
    }),
  );

  await page.getByRole("button", { name: "Full Sync" }).click();

  await expect(page.getByText(/Updated: 20/)).toBeVisible();
  await expect(page.getByText(/Deleted: 3/)).toBeVisible();
  await expect(page.getByText(/Duration: 456ms/)).toBeVisible();
});
