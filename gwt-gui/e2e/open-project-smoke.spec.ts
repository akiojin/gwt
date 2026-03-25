import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  branchFeature,
  captureUxSnapshot,
  defaultRecentProject,
  detectedAgents,
  emitTauriEvent,
  expectAgentCanvasVisible,
  openBranchBrowser,
  openRecentProject,
  saveE2ECoverage,
  selectBranchInBrowser,
  setMockCommandResponses,
  standardBranchResponses,
  waitForMenuActionListener,
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

async function materializeWorktree(page: import("@playwright/test").Page) {
  await selectBranchInBrowser(page, branchFeature.name);
  await page.getByRole("button", { name: "Create Worktree" }).click();
  await expectAgentCanvasVisible(page);
  await expect(
    page.locator('[data-testid="agent-canvas-worktree-tile-feature-workflow-demo"]'),
  ).toBeVisible();
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

test("opens a recent project into the current shell smoke flow", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);
  await expect(page.locator(".statusbar .path")).toContainText(
    defaultRecentProject.path,
  );
  await captureUxSnapshot(page, testInfo, "open-project-smoke-agent-canvas");
});

test("Branch Browser shows branch detail in a full-window single surface", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await openBranchBrowser(page);
  await expect(
    page.locator('[data-testid="branch-browser-surface"]'),
  ).toBeVisible();
  await expect(
    page.locator(".branch-name", { hasText: branchFeature.name }),
  ).toBeVisible();

  await selectBranchInBrowser(page, branchFeature.name);
  await expect(page.getByTestId("branch-browser-detail")).toContainText(
    branchFeature.name,
  );
  await captureUxSnapshot(page, testInfo, "open-project-smoke-branch-browser");
});

test("materializes a worktree from Branch Browser into Agent Canvas", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await materializeWorktree(page);
  await captureUxSnapshot(page, testInfo, "open-project-smoke-worktree-tile");
});

test("launches an agent into a live session tile after worktree creation", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await setMockCommandResponses(page, launchResponses());
  await openRecentProject(page);

  await materializeWorktree(page);
  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "launch-agent" });
  await expect(
    page.getByRole("dialog", { name: "Launch Agent" }),
  ).toBeVisible();

  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .click();

  await expect(
    page.locator('[data-testid^="agent-canvas-session-agent-"]'),
  ).toBeVisible();
  await expect(
    page.locator('[data-testid^="agent-canvas-session-surface-agent-"] .xterm'),
  ).toBeVisible();
  await captureUxSnapshot(page, testInfo, "open-project-smoke-agent-session");
});

test("opens a terminal session tile and keeps the surface readable", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  const terminalTile = page.locator('[data-testid^="agent-canvas-session-terminal-"]').first();
  const terminalSurface = page.locator(
    '[data-testid^="agent-canvas-session-surface-terminal-"]',
  ).first();
  await expect(terminalTile).toBeVisible();
  await expect(terminalSurface).toBeVisible();

  const box = await terminalSurface.boundingBox();
  if (!box) throw new Error("terminal surface bounding box missing");
  expect(box.width).toBeGreaterThan(500);
  expect(box.height).toBeGreaterThan(260);
  await captureUxSnapshot(page, testInfo, "open-project-smoke-terminal-tile");
});

test("opens report dialog from the current shell and keeps it readable", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  const reportDialog = page.locator(".report-dialog");
  await expect(reportDialog).toBeVisible();
  await expect(reportDialog.locator("#bug-title")).toBeVisible();
  await expect(reportDialog.locator("#steps")).toBeVisible();

  const viewport = await reportDialog.evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    const titleInput = dialog.querySelector<HTMLInputElement>("#bug-title");
    return {
      heightPx: rect.height,
      viewportHeightPx: window.innerHeight,
      inputFontSizePx: titleInput
        ? parseFloat(getComputedStyle(titleInput).fontSize)
        : 0,
    };
  });

  expect(viewport.heightPx).toBeGreaterThanOrEqual(
    viewport.viewportHeightPx * 0.88,
  );
  expect(viewport.inputFontSizePx).toBeGreaterThanOrEqual(14);
  await captureUxSnapshot(page, testInfo, "open-project-smoke-report-dialog");
});

test("preserves saved window sessions on startup without crashing", async ({
  page,
}) => {
  const savedSessions = [
    {
      projectPath: "/tmp/project-main",
      tabs: [{ id: "agentCanvas", type: "agentCanvas", label: "Agent Canvas" }],
      activeTabId: "agentCanvas",
      branchBrowser: null,
      agentCanvas: null,
    },
    {
      projectPath: "/tmp/project-second",
      tabs: [{ id: "branchBrowser", type: "branchBrowser", label: "Branch Browser" }],
      activeTabId: "branchBrowser",
      branchBrowser: {
        filter: "Local",
        query: "",
        selectedBranchName: "main",
      },
      agentCanvas: null,
    },
  ];

  await page.addInitScript((sessionsJson) => {
    window.localStorage.setItem(
      "gwt.windowSessions.v1",
      JSON.stringify(sessionsJson),
    );
  }, savedSessions);

  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();

  const stored = await page.evaluate(() =>
    window.localStorage.getItem("gwt.windowSessions.v1"),
  );
  expect(stored).not.toBeNull();
  expect(stored).toContain("/tmp/project-main");
  expect(stored).toContain("/tmp/project-second");
});
