import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  openRecentProject,
  setMockCommandResponses,
  standardBranchResponses,
  detectedAgents,
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

test("StatusBar shows agent detection status", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    detect_agents: detectedAgents,
  });
  await openRecentProject(page);

  // Should show agent names in status bar
  await expect(page.locator(".statusbar .agents")).toBeVisible();
});

test("StatusBar shows Codex agent with version when detected", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    detect_agents: detectedAgents,
  });
  await openRecentProject(page);

  await expect(
    page.locator(".statusbar .agent", { hasText: "Codex" }),
  ).toBeVisible();
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

test("StatusBar shows not-installed agents with bad class", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    detect_agents: [],
  });
  await openRecentProject(page);

  // When no agents detected, all agent entries show "not installed"
  await expect(page.locator(".statusbar .agent.bad").first()).toBeVisible();
});

test("StatusBar branch indicator is visible", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  await expect(page.locator(".statusbar .branch-indicator")).toBeVisible();
});
