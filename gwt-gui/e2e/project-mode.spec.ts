import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  openRecentProject,
  getInvokeLog,
} from "./support/helpers";

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("Project Mode panel shows decree input", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Decree something..."),
  ).toBeVisible();
});

test("Project Mode shows Send button", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
});

test("sending a message invokes command", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  const prompt = page.getByPlaceholder("Decree something...");
  await prompt.fill("hello project mode");
  await page.getByRole("button", { name: "Send" }).click();

  await expect
    .poll(async () => {
      const log = await getInvokeLog(page);
      return log.includes("send_project_mode_message_cmd");
    })
    .toBe(true);
});

test("sending a message displays echo response in timeline", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  const prompt = page.getByPlaceholder("Decree something...");
  await prompt.fill("test echo");
  await page.getByRole("button", { name: "Send" }).click();

  // Click Lead Orb to open timeline
  await page.getByRole("button", { name: /Lead/ }).click();
  await expect(page.getByText("Echo: test echo")).toBeVisible();
});

test("send invokes send_project_mode_message_cmd via Enter", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  const prompt = page.getByPlaceholder("Decree something...");
  await prompt.fill("invoke check");
  await prompt.press("Enter");

  await expect
    .poll(async () => {
      const log = await getInvokeLog(page);
      return log.includes("send_project_mode_message_cmd");
    })
    .toBe(true);
});

test("input clears after sending", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  const prompt = page.getByPlaceholder("Decree something...");
  await prompt.fill("clear test");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(prompt).toHaveValue("");
});

test("Send button is disabled when input is empty", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  await expect(
    page.getByRole("button", { name: "Send" }),
  ).toBeDisabled();
});

test("pressing Enter sends message", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  const prompt = page.getByPlaceholder("Decree something...");
  await prompt.fill("enter test");
  await prompt.press("Enter");

  await expect
    .poll(async () => {
      const log = await getInvokeLog(page);
      return log.includes("send_project_mode_message_cmd");
    })
    .toBe(true);
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
    page.getByPlaceholder("Decree something..."),
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

test("Project Mode shows session name in GodBar", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(page.locator(".god-bar-session")).toContainText("Project Mode");
});

test("God Game world shows Lead Orb", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(page.getByRole("button", { name: /Lead/ })).toBeVisible();
});

test("empty world shows guide message", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByText("The world is quiet. Issue a decree to begin."),
  ).toBeVisible();
});
