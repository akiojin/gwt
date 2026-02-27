import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  openRecentProject,
  setMockCommandResponses,
  standardBranchResponses,
  getInvokeLog,
} from "./support/helpers";

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("Project Mode panel shows prompt input", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();
});

test("Project Mode shows Send button", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
});

test("sending a message displays user message", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  const prompt = page.getByPlaceholder("Type a task and press Enter...");
  await prompt.fill("hello project mode");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(
    page.getByText("hello project mode", { exact: true }),
  ).toBeVisible();
});

test("sending a message displays echo response", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  const prompt = page.getByPlaceholder("Type a task and press Enter...");
  await prompt.fill("test echo");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText("Echo: test echo")).toBeVisible();
});

test("send invokes send_project_mode_message_cmd", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  const prompt = page.getByPlaceholder("Type a task and press Enter...");
  await prompt.fill("invoke check");
  await prompt.press("Enter");

  await expect(page.getByText("Echo: invoke check")).toBeVisible();
  const log = await getInvokeLog(page);
  expect(log).toContain("send_project_mode_message_cmd");
});

test("input clears after sending", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  const prompt = page.getByPlaceholder("Type a task and press Enter...");
  await prompt.fill("clear test");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText("Echo: clear test")).toBeVisible();
  await expect(prompt).toHaveValue("");
});

test("multiple messages accumulate in chat", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  const prompt = page.getByPlaceholder("Type a task and press Enter...");

  await prompt.fill("message one");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Echo: message one")).toBeVisible();

  await prompt.fill("message two");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Echo: message two")).toBeVisible();
  // Both messages should be visible
  await expect(
    page.getByText("message one", { exact: true }),
  ).toBeVisible();
});

test("Send button is disabled when input is empty", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  // Send button should be disabled when input is empty
  await expect(
    page.getByRole("button", { name: "Send" }),
  ).toBeDisabled();
});

test("pressing Enter sends message", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  const prompt = page.getByPlaceholder("Type a task and press Enter...");
  await prompt.fill("enter test");
  await prompt.press("Enter");

  await expect(page.getByText("Echo: enter test")).toBeVisible();
});

test("Project Mode state is retrieved on mount", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  await expect
    .poll(async () => {
      const log = await getInvokeLog(page);
      return log.includes("get_project_mode_state_cmd");
    })
    .toBe(true);
});

test("project mode tab is saved to localStorage", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  await expect
    .poll(async () => {
      return page.evaluate(() => {
        const raw = window.localStorage.getItem("gwt.projectTabs.v2");
        if (!raw) return false;
        try {
          const parsed = JSON.parse(raw) as {
            byProjectPath?: Record<
              string,
              { activeTabId?: string | null }
            >;
          };
          return (
            parsed.byProjectPath?.["/tmp/gwt-playwright"]?.activeTabId ===
            "projectMode"
          );
        } catch {
          return false;
        }
      });
    })
    .toBe(true);
});

test("Project Mode shows session name in header", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  // The session name "Project Mode" appears in the agent header
  await expect(page.locator(".agent-title")).toContainText("Project Mode");
});

test("chat area scrolls on new messages", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  const prompt = page.getByPlaceholder("Type a task and press Enter...");

  // Send several messages to fill chat
  for (let i = 0; i < 5; i++) {
    await prompt.fill(`scroll test ${i}`);
    await page.getByRole("button", { name: "Send" }).click();
    await expect(page.getByText(`Echo: scroll test ${i}`)).toBeVisible();
  }

  // Last message should be visible (auto-scrolled)
  await expect(page.getByText("Echo: scroll test 4")).toBeVisible();
});
