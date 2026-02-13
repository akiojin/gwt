import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";

test.beforeEach(async ({ page }) => {
  await installTauriMock(page);
});

test("launches and completes open-project -> agent send smoke flow", async ({
  page,
}) => {
  await page.goto("/");

  await expect(page.getByRole("button", { name: "Open Project..." })).toBeVisible();
  const recentItem = page.locator("button.recent-item").first();
  await expect(recentItem).toBeVisible();

  await recentItem.click();

  const prompt = page.getByPlaceholder("Type a task and press Enter...");
  await expect(prompt).toBeVisible();

  const message = "smoke message";
  await prompt.fill(message);
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText(`Echo: ${message}`)).toBeVisible();

  const invokeCommands = await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
    };
    return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).map((entry) => entry.cmd);
  });

  expect(invokeCommands).toContain("open_project");
  expect(invokeCommands).toContain("send_agent_mode_message");
});
