import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  branchMain,
  branchDevelop,
  branchFeature,
  captureUxSnapshot,
  openRecentProject,
  setMockCommandResponses,
  detectedAgents,
  standardBranchResponses,
  waitForInvokeCommand,
  waitForMenuActionListener,
  waitForEventListener,
  emitTauriEvent,
  expectAgentCanvasVisible,
  saveE2ECoverage,
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
  await page.evaluate((branch) => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: {
          activateBranch?: (branch: typeof branchFeature) => Promise<void> | void;
        };
      }
    ).__GWT_E2E_APP__?.activateBranch(branch);
  }, branchFeature);
  await expectAgentCanvasVisible(page);
  await expect(
    page.locator('[data-testid^="agent-canvas-worktree-card-"]', {
      hasText: branchFeature.name,
    }),
  ).toBeVisible();
  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "launch-agent" });
  await expect(
    page.getByRole("dialog", { name: "Launch Agent" }),
  ).toBeVisible();
}

async function clickLaunchButton(page: import("@playwright/test").Page) {
  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .evaluate((node) => (node as HTMLElement).click());
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
  await clickLaunchButton(page);

  await waitForInvokeCommand(page, "start_launch_job");
});

test("new terminal opens from menu action into Agent Canvas", async ({ page }, testInfo) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  await waitForInvokeCommand(page, "spawn_shell");
  await expect(
    page.locator('[data-testid^="agent-canvas-session-terminal-"]:visible'),
  ).toBeVisible();
  await expect(
    page.locator('[data-testid^="agent-canvas-session-surface-terminal-"]:visible'),
  ).toBeVisible();
  await captureUxSnapshot(page, testInfo, "menu-terminal-session-card");
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
      page.locator('[data-testid^="agent-canvas-session-terminal-"]:visible').count(),
    )
    .toBe(2);
});

test("terminal-output listener is registered after agent launch", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, launchResponses());
  await openRecentProject(page);

  await materializeWorktreeAndOpenLaunchDialog(page);
  await clickLaunchButton(page);

  await waitForEventListener(page, "terminal-output");
});

test("buffered launch-progress event is applied after delayed job start", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...launchResponses(),
    start_launch_job: {
      __delayMs: 250,
      value: "buffered-job-1",
    },
  });
  await openRecentProject(page);

  await materializeWorktreeAndOpenLaunchDialog(page);
  await clickLaunchButton(page);

  await emitTauriEvent(page, "launch-progress", {
    jobId: "buffered-job-1",
    step: "skills",
    detail: "Registering skills from buffered event",
  });

  await expect(page.getByRole("dialog", { name: "Preparing Launch" })).toBeVisible();
  await expect(page.getByText("Registering skills from buffered event")).toBeVisible();
});

test("buffered branch-exists launch-finished error keeps the conflicting branch visible", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...launchResponses(),
    start_launch_job: {
      __delayMs: 250,
      value: "buffered-job-branch-exists",
    },
  });
  await openRecentProject(page);

  await materializeWorktreeAndOpenLaunchDialog(page);
  await clickLaunchButton(page);

  await emitTauriEvent(page, "launch-finished", {
    jobId: "buffered-job-branch-exists",
    status: "error",
    paneId: null,
    error: "[E1004] Branch already exists: feature/workflow-demo",
  });

  await expect(page.getByRole("dialog", { name: "Preparing Launch" })).toContainText(
    "feature/workflow-demo",
  );
});

test("launch poll fallback surfaces an unexpected completion error", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...launchResponses(),
    start_launch_job: "poll-job-id",
    poll_launch_job: {
      running: false,
      finished: null,
    },
  });
  await openRecentProject(page);

  await materializeWorktreeAndOpenLaunchDialog(page);
  await clickLaunchButton(page);

  await expect(page.getByRole("dialog", { name: "Preparing Launch" })).toBeVisible();
  await expect
    .poll(async () =>
      page
        .getByRole("dialog", { name: "Preparing Launch" })
        .textContent()
        .catch(() => ""),
      { timeout: 10000 },
    )
    .toContain("Launch job ended unexpectedly");
});

test("launch modal shows cancel action while launch is pending", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...launchResponses(),
    start_launch_job: "cancel-job-id",
    poll_launch_job: {
      running: true,
      finished: null,
    },
  });
  await openRecentProject(page);

  await materializeWorktreeAndOpenLaunchDialog(page);
  await clickLaunchButton(page);

  await expect(page.getByRole("dialog", { name: "Preparing Launch" })).toBeVisible();
  await expect(page.getByRole("button", { name: /Cancel \(Esc\)/ })).toBeVisible();
});

test("launch-progress without an active job is ignored", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, launchResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await emitTauriEvent(page, "launch-progress", {
    jobId: "orphan-job",
    step: "skills",
    detail: "Ignored progress event",
  });

  await page.waitForTimeout(250);
  await expect(page.getByRole("dialog", { name: "Preparing Launch" })).toHaveCount(0);
  await expect(page.getByText("Ignored progress event")).toHaveCount(0);
});

test("launch-finished without an active job is ignored", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, launchResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await emitTauriEvent(page, "launch-finished", {
    jobId: "orphan-job",
    status: "error",
    paneId: null,
    error: "[E1004] Branch already exists: ignored-branch",
  });

  await page.waitForTimeout(250);
  await expect(page.getByRole("dialog", { name: "Preparing Launch" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Use Existing Branch" })).toHaveCount(0);
});

test("launch-progress for a different job is ignored while a launch is active", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...launchResponses(),
    start_launch_job: "active-job-id",
    poll_launch_job: {
      running: true,
      finished: null,
    },
  });
  await openRecentProject(page);

  await materializeWorktreeAndOpenLaunchDialog(page);
  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .click();

  await expect(page.getByRole("dialog", { name: "Preparing Launch" })).toBeVisible();
  await emitTauriEvent(page, "launch-progress", {
    jobId: "different-job-id",
    step: "skills",
    detail: "Wrong launch detail",
  });

  await page.waitForTimeout(250);
  await expect(page.getByRole("dialog", { name: "Preparing Launch" })).toBeVisible();
  await expect(page.getByText("Wrong launch detail")).toHaveCount(0);
});

test("launch-finished cancelled closes the launch modal", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...launchResponses(),
    start_launch_job: "cancelled-finish-job",
    poll_launch_job: {
      running: true,
      finished: null,
    },
  });
  await openRecentProject(page);

  await materializeWorktreeAndOpenLaunchDialog(page);
  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .click();

  await expect(page.getByRole("dialog", { name: "Preparing Launch" })).toBeVisible();
  await emitTauriEvent(page, "launch-finished", {
    jobId: "cancelled-finish-job",
    status: "cancelled",
    paneId: null,
    error: null,
  });

  await expect(page.getByRole("dialog", { name: "Preparing Launch" })).toBeHidden();
});
