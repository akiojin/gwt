import { expect, test } from "@playwright/test";
import {
  gotoLiveGwt,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

// Issue #4153: receive-contract coverage for the configuration diagnostic.
// Rust process fixtures prove detection, fallback, and attempt preservation.
test("Codex config failures retain cause, file, and recovery in launch error surfaces", async ({ page }, testInfo) => {
  test.skip(!BASE || !process.env.GWT_PLAYWRIGHT_CHECK_HOME,
    "requires browser-check checkout binaries and a fresh HOME");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  const diagnostic = "codex_config_preflight: Codex CLI 0.148.0 could not load configuration (~/.codex/config.toml). invalid type: map, expected a boolean in features\nNext: run `codex doctor` with the same CODEX_HOME and fix the reported config error, or update Codex if the setting requires a newer CLI; relaunch after repair.";
  await withLiveGwtBackendLock(BASE, testInfo, async () => {
    await gotoLiveGwt(page, BASE);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const drawer = page.locator("#notification-center");
    await page.evaluate(message => window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
      detail: { kind: "issue_monitor_toast", level: "error", message,
        issue_number: 4153, notification_transition: null },
    })), diagnostic);
    await page.locator("#op-notifications-button").click();
    const row = drawer.locator(".notification-center__item").filter({ hasText: "codex_config_preflight:" });
    await expect(row).toHaveCount(1);
    await expect(row).toContainText("~/.codex/config.toml");
    await expect(row).toContainText("invalid type: map, expected a boolean in features");
    await expect(row).toContainText("codex doctor");
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();
    await page.evaluate(message => window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
      detail: { kind: "launch_wizard_open_error", title: "Codex configuration", message },
    })), diagnostic);
    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toBeVisible();
    await expect(wizard.locator("#wizard-error")).toHaveText(diagnostic);
    const screenshot = testInfo.outputPath(`codex-config-error-${theme}.png`);
    await page.screenshot({ path: screenshot, fullPage: true });
    await testInfo.attach(`codex-config-error-${theme}`, {
      path: screenshot, contentType: "image/png",
    });
    await wizard.getByRole("button", { name: "Close", exact: true }).click();
    expect(errors, "console and page errors").toEqual([]);
  });
});

// Issue #5866: verify how the checkout-served UI presents the real event
// contracts. Runtime restore selection, initial reassignment, and continuing
// the remaining launches are covered in restore_launch_tests.rs; injecting
// receive events here does not prove those runtime behaviors.
test("automatic restore errors stay in Notifications while manual errors open a dialog", async ({ page }, testInfo) => {
  test.skip(!BASE || !process.env.GWT_PLAYWRIGHT_CHECK_HOME,
    "requires browser-check checkout binaries and a fresh HOME");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);

  await withLiveGwtBackendLock(BASE, testInfo, async () => {
    // The permanent receive-injection hook is sufficient here. Enabling the
    // optional bridge installs a second listener for the same injected event.
    await gotoLiveGwt(page, BASE);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

    const bell = page.locator("#op-notifications-button");
    const drawer = page.locator("#notification-center");
    const rows = drawer.locator(".notification-center__item");
    const wizard = page.locator("#wizard-modal");
    await bell.click();
    await drawer.getByRole("button", { name: "Clear all", exact: true }).click();
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();

    const restoreError = "Unable to restore saved Agent: configured executable is unavailable.";
    await page.evaluate(message => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
        detail: {
          kind: "issue_monitor_toast", level: "error", message,
          issue_number: null, notification_transition: null,
        },
      }));
    }, restoreError);
    await expect(bell.locator(".op-rail__badge")).toHaveText("1");
    await expect(bell.locator(".op-rail__badge")).toHaveAttribute("data-has-error", "true");
    await expect(wizard).toBeHidden();
    await expect(page.locator(".modal-backdrop:visible")).toHaveCount(0);

    // Clicking the ordinary rail control also verifies the failure did not
    // install a blocking overlay over the workspace.
    await bell.click();
    await expect(drawer).toBeVisible();
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toHaveAttribute("data-level", "error");
    await expect(rows.first()).toContainText(restoreError);
    const notificationShot = testInfo.outputPath(`automatic-restore-notification-${theme}.png`);
    await page.screenshot({ path: notificationShot, fullPage: true });
    await testInfo.attach(`automatic-restore-notification-${theme}`, {
      path: notificationShot, contentType: "image/png",
    });
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();

    const manualError = "Agent restore failed: configured executable is unavailable.";
    await page.evaluate(message => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
        detail: { kind: "launch_wizard_open_error", title: "Agent restore", message },
      }));
    }, manualError);
    await expect(wizard).toBeVisible();
    await expect(wizard.getByRole("dialog", { name: /agent restore/i }))
      .toHaveAttribute("aria-modal", "true");
    await expect(wizard.locator("#wizard-error")).toHaveText(manualError);
    const manualShot = testInfo.outputPath(`manual-launch-error-${theme}.png`);
    await page.screenshot({ path: manualShot, fullPage: true });
    await testInfo.attach(`manual-launch-error-${theme}`, {
      path: manualShot, contentType: "image/png",
    });
    await wizard.getByRole("button", { name: "Close", exact: true }).click();
    await expect(wizard).toBeHidden();
    expect(errors, "console and page errors").toEqual([]);
  });
});

// Issue #4222: this checks presentation of backend fallback reasons; native
// conversation ownership and resume selection are exercised by Rust tests.
test("Issue Monitor fresh-session notifications retain each native conversation fallback reason", async ({ page }, testInfo) => {
  test.skip(!BASE || !process.env.GWT_PLAYWRIGHT_CHECK_HOME,
    "requires browser-check checkout binaries and a fresh HOME");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);

  await withLiveGwtBackendLock(BASE, testInfo, async () => {
    await gotoLiveGwt(page, BASE);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

    const bell = page.locator("#op-notifications-button");
    const drawer = page.locator("#notification-center");
    const rows = drawer.locator(".notification-center__item");
    await bell.click();
    await drawer.getByRole("button", { name: "Clear all", exact: true }).click();
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();

    const messages = [
      "Issue Monitor started a fresh session because native conversation is missing",
      "Issue Monitor started a fresh session because native conversation ownership does not match this worktree",
      "Issue Monitor started a fresh session because native conversation could not be verified",
      "Issue Monitor started a fresh session because native conversation is already held by window holder-window",
    ];
    for (const [index, message] of messages.entries()) {
      await page.evaluate(message => {
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "issue_monitor_toast", level: "warn", message,
            issue_number: 4222, notification_transition: null,
          },
        }));
      }, message);
      await expect(bell.locator(".op-rail__badge")).toHaveText(String(index + 1));
    }

    await bell.click();
    await expect(drawer).toBeVisible();
    await expect(rows).toHaveCount(messages.length);
    for (const message of messages) {
      const row = rows.filter({ hasText: message });
      await expect(row).toHaveCount(1);
      await expect(row).toHaveAttribute("data-level", "warn");
      await expect(row).toBeVisible();
    }
    await expect(page.locator(".modal-backdrop:visible")).toHaveCount(0);
    const screenshot = testInfo.outputPath(`native-conversation-fallback-reasons-${theme}.png`);
    await page.screenshot({ path: screenshot, fullPage: true });
    await testInfo.attach(`native-conversation-fallback-reasons-${theme}`, {
      path: screenshot, contentType: "image/png",
    });
    expect(errors, "console and page errors").toEqual([]);
  });
});

// Issue #3626: this receive contract covers warning presentation only. The
// Rust launch regression injects installer failure and proves PTY/input reachability;
// injecting a toast here does not prove a full degraded launch.
test("degraded resume warnings retain the reason and Start Work recovery without blocking workspace controls", async ({ page }, testInfo) => {
  test.skip(!BASE || !process.env.GWT_PLAYWRIGHT_CHECK_HOME,
    "requires browser-check checkout binaries and a fresh HOME");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);

  await withLiveGwtBackendLock(BASE, testInfo, async () => {
    await gotoLiveGwt(page, BASE);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const bell = page.locator("#op-notifications-button");
    const drawer = page.locator("#notification-center");
    const rows = drawer.locator('.notification-center__item[data-kind="issue-monitor"]');
    await bell.click();
    await drawer.getByRole("button", { name: "Clear all", exact: true }).click();
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();

    const reason = "injected resume capability installation failure";
    const warning = `Resume degraded to unbound (no execution authority). Reason: ${reason}. Input remains available. Use Start Work / Launch to obtain execution authority before delivering changes.`;
    await page.evaluate(message => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
        detail: {
          kind: "issue_monitor_toast", level: "warn", message,
          issue_number: 3626, notification_transition: null,
        },
      }));
    }, warning);
    await expect(bell.locator(".op-rail__badge")).toHaveText("1");
    await expect(page.locator(".modal-backdrop:visible")).toHaveCount(0);
    await expect(page.locator("#wizard-modal")).toBeHidden();

    // Ordinary notification controls remain usable after the warning. Closing
    // and reopening the drawer must retain its Issue, reason, and recovery path.
    await bell.click();
    await expect(drawer).toBeVisible();
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toHaveAttribute("data-level", "warn");
    await expect(rows.first().locator(".notification-center__title")).toHaveText("Issue Monitor #3626");
    await expect(rows.first().locator(".notification-center__message")).toHaveText(warning);
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();
    await expect(drawer).toBeHidden();
    await bell.click();
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText(reason);
    await expect(rows.first()).toContainText("Start Work / Launch");
    const screenshot = testInfo.outputPath(`degraded-resume-notification-${theme}.png`);
    await page.screenshot({ path: screenshot, fullPage: true });
    await testInfo.attach(`degraded-resume-notification-${theme}`, {
      path: screenshot, contentType: "image/png",
    });
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();
    await expect(page.locator(".modal-backdrop:visible")).toHaveCount(0);
    expect(errors, "console and page errors").toEqual([]);
  });
});
