import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  branchMain,
  branchDevelop,
  branchFeature,
  openRecentProject,
  setMockCommandResponses,
  detectedAgents,
  standardBranchResponses,
  waitForInvokeCommand,
  waitForMenuActionListener,
  waitForEventListener,
  emitTauriEvent,
  expectAgentCanvasVisible,
  selectBranchInBrowser,
} from "./support/helpers";

function launchResponses() {
  return {
    ...standardBranchResponses(),
    detect_agents: detectedAgents,
    list_agent_versions: {
      agentId: "codex",
      package: "codex",
      tags: ["latest"],
      versions: ["0.99.0"],
      source: "cache",
    },
  };
}

async function materializeWorktreeAndOpenLaunchDialog(page: import("@playwright/test").Page) {
  await selectBranchInBrowser(page, branchFeature.name);
  await page.getByRole("button", { name: "Create Worktree" }).click();
  await expectAgentCanvasVisible(page);
  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "launch-agent" });
  await expect(
    page.getByRole("dialog", { name: "Launch Agent" }),
  ).toBeVisible();
}

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("Launch Agent dialog opens from current shell flow", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, launchResponses());
  await openRecentProject(page);

  await materializeWorktreeAndOpenLaunchDialog(page);
});

test("Launch Agent dialog shows detected agent in selector", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, launchResponses());
  await openRecentProject(page);

  await materializeWorktreeAndOpenLaunchDialog(page);
  await expect(page.locator("select#agent-select")).toHaveValue("codex");
});

test("Launch Agent invokes start_launch_job", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, launchResponses());
  await openRecentProject(page);

  await materializeWorktreeAndOpenLaunchDialog(page);
  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .click();

  await waitForInvokeCommand(page, "start_launch_job");
});

test("agent session card appears inside Agent Canvas after launch", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, launchResponses());
  await openRecentProject(page);

  await materializeWorktreeAndOpenLaunchDialog(page);
  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .click();

  await expect(
    page.locator('[data-testid^="agent-canvas-session-agent-"]', {
      hasText: branchFeature.name,
    }),
  ).toBeVisible();

  const sessionCard = page.locator('[data-testid^="agent-canvas-session-agent-"]').first();
  const cardBox = await sessionCard.boundingBox();
  if (!cardBox) throw new Error("agent session card bounding box missing");
  expect(cardBox.width).toBeGreaterThan(500);
  expect(cardBox.height).toBeGreaterThan(360);
});

test("agent session card shows live terminal surface after launch", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, launchResponses());
  await openRecentProject(page);

  await materializeWorktreeAndOpenLaunchDialog(page);
  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .click();

  await expect(
    page.locator('[data-testid^="agent-canvas-session-surface-agent-"] .xterm'),
  ).toBeVisible();

  const sessionSurface = page.locator('[data-testid^="agent-canvas-session-surface-agent-"]').first();
  const surfaceBox = await sessionSurface.boundingBox();
  if (!surfaceBox) throw new Error("agent session surface bounding box missing");
  expect(surfaceBox.width).toBeGreaterThan(500);
  expect(surfaceBox.height).toBeGreaterThan(260);
});

test("new terminal opens from menu action into Agent Canvas", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  await waitForInvokeCommand(page, "spawn_shell");
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]'),
  ).toBeVisible();

  const sessionSurface = page.locator('[data-testid^="agent-canvas-session-surface-terminal-"]').first();
  const surfaceBox = await sessionSurface.boundingBox();
  if (!surfaceBox) throw new Error("terminal session surface bounding box missing");
  expect(surfaceBox.width).toBeGreaterThan(500);
  expect(surfaceBox.height).toBeGreaterThan(260);
});

test("multiple terminal session cards can be opened", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  await expect
    .poll(async () =>
      page.locator('[data-testid^="agent-canvas-session-terminal-"]').count(),
    )
    .toBe(2);

  const cardBoxes = await page
    .locator('[data-testid^="agent-canvas-session-terminal-"]')
    .evaluateAll((els) =>
      els.map((el) => {
        const rect = el.getBoundingClientRect();
        return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
      }),
    );
  expect(cardBoxes.length).toBe(2);
  expect(cardBoxes[0]?.width ?? 0).toBeGreaterThan(500);
  expect(cardBoxes[1]?.width ?? 0).toBeGreaterThan(500);
});

test("terminal-output listener is registered after agent launch", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, launchResponses());
  await openRecentProject(page);

  await materializeWorktreeAndOpenLaunchDialog(page);
  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .click();

  await waitForEventListener(page, "terminal-output");
});
