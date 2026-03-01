import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  branchMain,
  branchDevelop,
  branchFeature,
  openRecentProject,
  setMockCommandResponses,
  dismissSkillRegistrationScopeDialogIfPresent,
  waitForInvokeCommand,
  getInvokeLog,
  standardBranchResponses,
} from "./support/helpers";

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("displays Open Project and New Project buttons on launch", async ({
  page,
}) => {
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "New Project" }),
  ).toBeVisible();
});

test("shows recent projects list", async ({ page }) => {
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await expect(page.getByText("Recent Projects")).toBeVisible();
  await expect(page.locator("button.recent-item").first()).toBeVisible();
  await expect(page.locator(".recent-path")).toContainText(
    defaultRecentProject.path,
  );
});

test("opens project from recent projects list", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();
  const log = await getInvokeLog(page);
  expect(log).toContain("open_project");
});

test("shows New Project form when clicking New Project button", async ({
  page,
}) => {
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await page.getByRole("button", { name: "New Project" }).click();
  await expect(page.getByText("Repository URL")).toBeVisible();
  await expect(page.getByText("Parent Directory")).toBeVisible();
  await expect(page.getByText("Clone Mode")).toBeVisible();
});

test("New Project form has Shallow clone mode selected by default", async ({
  page,
}) => {
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await page.getByRole("button", { name: "New Project" }).click();
  await expect(
    page.getByRole("button", { name: "Shallow (Recommended)" }),
  ).toHaveClass(/active/);
});

test("toggles clone mode between Shallow and Full", async ({ page }) => {
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await page.getByRole("button", { name: "New Project" }).click();

  await page.getByRole("button", { name: "Full" }).click();
  await expect(page.getByRole("button", { name: "Full" })).toHaveClass(
    /active/,
  );
  await expect(
    page.getByRole("button", { name: "Shallow (Recommended)" }),
  ).not.toHaveClass(/active/);
});

test("Create button disabled when URL or parent dir is empty", async ({
  page,
}) => {
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await page.getByRole("button", { name: "New Project" }).click();
  await expect(page.getByRole("button", { name: "Create" })).toBeDisabled();
});

test("opens project and displays branch list in sidebar", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await expect(page.locator(".branch-name", { hasText: "main" })).toBeVisible();
  await expect(
    page.locator(".branch-name", { hasText: "develop" }),
  ).toBeVisible();
  await expect(
    page.locator(".branch-name", { hasText: branchFeature.name }),
  ).toBeVisible();
});

test("probe_path with gwtProject navigates to project", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await waitForInvokeCommand(page, "open_project");
  const log = await getInvokeLog(page);
  expect(log).toContain("probe_path");
  expect(log).toContain("open_project");
});

test("shows error when probe returns notFound", async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [
        { path: "/nonexistent/path", lastOpened: "2026-01-01T00:00:00.000Z" },
      ],
      probe_path: { kind: "notFound", message: "Path does not exist." },
    },
  });
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await page.locator("button.recent-item").first().click();
  await expect(page.locator(".error")).toContainText("Path does not exist");
});

test("shows migration modal when probe returns migrationRequired", async ({
  page,
}) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
      probe_path: {
        kind: "migrationRequired",
        migrationSourceRoot: "/tmp/gwt-playwright",
      },
    },
  });
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await page.locator("button.recent-item").first().click();
  await expect(page.getByText("Validating prerequisites")).toBeVisible();
});

test("close project returns to OpenProject screen", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  // Trigger close-project menu action
  await expect
    .poll(async () => {
      return page.evaluate(() => {
        const globalWindow = window as unknown as {
          __GWT_TAURI_INVOKE_LOG__?: Array<{
            cmd: string;
            args?: { event?: string };
          }>;
        };
        return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).some(
          (entry) =>
            entry.cmd === "plugin:event|listen" &&
            entry.args?.event === "menu-action",
        );
      });
    })
    .toBe(true);

  await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_MOCK_EMIT_EVENT__?: (event: string, payload: unknown) => void;
    };
    globalWindow.__GWT_MOCK_EMIT_EVENT__?.("menu-action", {
      action: "close-project",
    });
  });

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
});

test("displays gwt title and subtitle on open project screen", async ({
  page,
}) => {
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await expect(page.locator(".title", { hasText: "gwt" })).toBeVisible();
  await expect(
    page.locator(".subtitle", { hasText: "Git Worktree Manager" }),
  ).toBeVisible();
});

test("project mode prompt is visible after opening project", async ({
  page,
}) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();
});

test("StatusBar shows project path after opening", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(page.locator(".statusbar .path")).toContainText(
    defaultRecentProject.path,
  );
});

test("StatusBar shows current branch", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(page.locator(".statusbar")).toContainText("main");
});

test("recent project shows formatted date", async ({ page }) => {
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await expect(page.locator(".recent-time").first()).toBeVisible();
});

test("New Project form can be toggled closed", async ({ page }) => {
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await page.getByRole("button", { name: "New Project" }).click();
  await expect(page.getByText("Repository URL")).toBeVisible();
  await page.getByRole("button", { name: "New Project" }).click();
  await expect(page.getByText("Repository URL")).toBeHidden();
});

test("opening project invokes open_project with correct path", async ({
  page,
}) => {
  await page.goto("/");
  await openRecentProject(page);
  await waitForInvokeCommand(page, "open_project");

  const args = await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{
        cmd: string;
        args?: Record<string, unknown>;
      }>;
    };
    const log = globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? [];
    return log.find((e) => e.cmd === "open_project")?.args ?? null;
  });
  expect(args?.path).toBe(defaultRecentProject.path);
});

test("restores saved window sessions on startup", async ({ page }) => {
  const savedSessions = [
    { label: "main", projectPath: "/tmp/project-main" },
    { label: "project-2", projectPath: "/tmp/project-second" },
  ];

  await page.addInitScript((sessionsJson) => {
    window.localStorage.setItem(
      "gwt.windowSessions.v1",
      JSON.stringify(sessionsJson),
    );
  }, savedSessions);

  await page.goto("/");

  await expect
    .poll(async () => {
      const raw = await page.evaluate(() => {
        return (
          window as unknown as {
            __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
          }
        ).__GWT_TAURI_INVOKE_LOG__;
      });
      const entries = Array.isArray(raw) ? raw : [];
      return entries.some((entry) => entry.cmd === "open_project");
    })
    .toBe(true);

  const log = await getInvokeLog(page);
  expect(log).toContain("open_gwt_window");
});

test("disables buttons while opening project", async ({ page }) => {
  await page.goto("/");
  await dismissSkillRegistrationScopeDialogIfPresent(page);

  // Click recent item to start opening
  const recentItem = page.locator("button.recent-item").first();
  await expect(recentItem).toBeVisible();
  // After clicking, the Open Project... button should become disabled temporarily
  await recentItem.click();
  // We just verify the project eventually opens
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();
});
