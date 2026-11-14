/** Issue #3813 — headed dark/light verification for Settings > System. */
import { test, expect } from "@playwright/test";
import {
  acquireLiveGwtBackendLock,
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const VISUAL_DIR = process.env.GWT_AGENT_RESOURCE_VISUAL_DIR ?? "";

async function waitForSettingsEvent(
  page: import("@playwright/test").Page,
  kind: "system_settings" | "system_settings_updated",
  cursor: number,
  expected: Record<string, unknown> = {},
): Promise<Record<string, any>> {
  await page.waitForFunction(
    ({ cursor, kind, expected }) => {
      const messages = (window as any).__gwtPlaywrightMessages;
      return Array.isArray(messages) && messages.some((entry: any) =>
        entry?.sequence > cursor
        && entry?.payload?.kind === kind
        && Object.entries(expected).every(([key, value]) =>
          entry.payload?.[key] === value
        )
      );
    },
    { cursor, kind, expected },
  );
  return page.evaluate(({ cursor, kind, expected }) => {
    const messages = (window as any).__gwtPlaywrightMessages as Array<any>;
    return [...messages].reverse().find((entry) =>
      entry?.sequence > cursor
      && entry?.payload?.kind === kind
      && Object.entries(expected).every(([key, value]) =>
        entry.payload?.[key] === value
      )
    )?.payload;
  }, { cursor, kind, expected });
}

async function currentSettings(
  page: import("@playwright/test").Page,
): Promise<Record<string, any>> {
  const cursor = await page.evaluate(() =>
    Number((window as any).__gwtPlaywrightMessageSequence) || 0
  );
  await sendLiveGwtEvent(page, { kind: "get_system_settings" });
  return waitForSettingsEvent(page, "system_settings", cursor);
}

async function changeAndWait(
  page: import("@playwright/test").Page,
  expected: Record<string, unknown>,
  change: () => Promise<void>,
): Promise<void> {
  const cursor = await page.evaluate(() =>
    Number((window as any).__gwtPlaywrightMessageSequence) || 0
  );
  await change();
  await waitForSettingsEvent(page, "system_settings_updated", cursor, expected);
}

async function openSystemSettings(
  page: import("@playwright/test").Page,
): Promise<import("@playwright/test").Locator> {
  await page.evaluate(() => {
    document.dispatchEvent(new CustomEvent("settings:open", {
      detail: { target: "system" },
      bubbles: true,
    }));
  });
  const settingsWindow = page.locator('.workspace-window[data-preset="settings"]');
  await expect(settingsWindow).toBeVisible();
  await expect(settingsWindow.getByRole("tab", { name: "System" }))
    .toHaveAttribute("aria-selected", "true");
  return settingsWindow;
}

test.describe.serial("Agent resource settings (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  let releaseBackendLock: (() => Promise<void>) | undefined;

  test.beforeEach(async ({}, testInfo) => {
    test.skip(
      !["chromium-dark", "chromium-light"].includes(testInfo.project.name),
      "resource Settings verification runs only in dark and light Chromium",
    );
    releaseBackendLock = await acquireLiveGwtBackendLock(BASE, testInfo);
  });

  test.afterEach(async () => {
    if (!releaseBackendLock) return;
    await releaseBackendLock();
    releaseBackendLock = undefined;
  });

  test("renders default resource controls without browser errors", async ({
    page,
  }, testInfo) => {
    const browserErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") browserErrors.push(message.text());
    });
    page.on("pageerror", (error) => browserErrors.push(error.message));

    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    if (await page.locator(".project-tab").count() === 0) {
      await openLiveGwtProject(page);
    }
    const original = await currentSettings(page);
    let settingsWindow = await openSystemSettings(page);

    const enabled = settingsWindow.locator("#settings-system-agent-resources");
    const priority = settingsWindow.locator("#settings-system-agent-priority");
    const cpu = settingsWindow.locator("#settings-system-agent-cpu-limit");
    const cargo = settingsWindow.locator("#settings-system-agent-cargo-jobs");
    await expect(enabled).toBeChecked();
    await expect(priority).toHaveValue("below_normal");
    await expect(priority.locator("option")).toHaveText([
      "Normal",
      "Below normal",
      "Idle",
    ]);
    await expect(cpu).toHaveValue("");
    await expect(cpu).toHaveAttribute("placeholder", "Auto");
    await expect(cargo).toHaveValue("");
    await expect(cargo).toHaveAttribute("placeholder", "Auto");

    const expectedTheme = testInfo.project.name.endsWith("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", expectedTheme);
    if (VISUAL_DIR) {
      await cargo.scrollIntoViewIfNeeded();
      await settingsWindow.screenshot({
        path: `${VISUAL_DIR}/agent-resources-${expectedTheme}.png`,
        animations: "disabled",
      });
    }

    try {
      await changeAndWait(
        page,
        { agent_resource_enabled: false },
        () => enabled.uncheck(),
      );
      await changeAndWait(
        page,
        { agent_resource_priority: "idle" },
        () => priority.selectOption("idle"),
      );
      await changeAndWait(
        page,
        { agent_cpu_limit_percent: 40 },
        async () => {
          await cpu.fill("40");
          await cpu.blur();
        },
      );
      await changeAndWait(
        page,
        { agent_cargo_build_jobs: 3 },
        async () => {
          await cargo.fill("3");
          await cargo.blur();
        },
      );

      const persisted = await currentSettings(page);
      expect(persisted).toMatchObject({
        agent_resource_enabled: false,
        agent_resource_priority: "idle",
        agent_cpu_limit_percent: 40,
        agent_cargo_build_jobs: 3,
      });
      settingsWindow = page.locator('.workspace-window[data-preset="settings"]');
      await expect(settingsWindow.locator("#settings-system-agent-resources"))
        .not.toBeChecked();
      await expect(settingsWindow.locator("#settings-system-agent-priority"))
        .toHaveValue("idle");
      await expect(settingsWindow.locator("#settings-system-agent-cpu-limit"))
        .toHaveValue("40");
      await expect(settingsWindow.locator("#settings-system-agent-cargo-jobs"))
        .toHaveValue("3");

      const persistedCpu = settingsWindow.locator("#settings-system-agent-cpu-limit");
      await persistedCpu.fill("1.5");
      await persistedCpu.blur();
      await expect(
        settingsWindow.locator("[data-role='system-settings-status']"),
      ).toContainText("Enter a value from 1 to 100");
      expect((await currentSettings(page)).agent_cpu_limit_percent).toBe(40);
    } finally {
      await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
      const cursor = await page.evaluate(() =>
        Number((window as any).__gwtPlaywrightMessageSequence) || 0
      );
      await sendLiveGwtEvent(page, {
        kind: "update_system_settings",
        language: original.language || "auto",
        agent_resource_enabled: original.agent_resource_enabled,
        agent_resource_priority: original.agent_resource_priority,
        agent_cpu_limit_percent: original.agent_cpu_limit_percent ?? null,
        agent_cpu_limit_automatic: original.agent_cpu_limit_percent == null,
        agent_cargo_build_jobs: original.agent_cargo_build_jobs ?? null,
        agent_cargo_build_jobs_automatic: original.agent_cargo_build_jobs == null,
      });
      await waitForSettingsEvent(page, "system_settings_updated", cursor, {
        agent_resource_enabled: original.agent_resource_enabled,
        agent_resource_priority: original.agent_resource_priority,
      });
    }
    expect(browserErrors).toEqual([]);
  });
});
