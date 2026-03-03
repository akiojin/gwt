import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  openRecentProject,
  waitForMenuActionListener,
  emitTauriEvent,
  waitForInvokeCommand,
} from "./support/helpers";

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("StatusBar shows project path", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  await expect(page.locator(".statusbar .path")).toContainText(
    "/tmp/gwt-playwright",
  );
});

test("StatusBar shows current branch name", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  await expect(page.locator(".statusbar")).toContainText("main");
});

test("StatusBar shows terminal count when terminals open", async ({
  page,
}) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  await waitForInvokeCommand(page, "spawn_shell");

  await expect(page.locator(".statusbar .terminal-count")).toContainText(
    "1 terminal",
  );
});

test("StatusBar shows voice status", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  await expect(page.locator(".statusbar .voice")).toBeVisible();
});

test("StatusBar branch indicator is visible", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  await expect(page.locator(".statusbar .branch-indicator")).toBeVisible();
});
