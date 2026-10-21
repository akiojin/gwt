import { expect, test } from "@playwright/test";
import {
  gotoLiveGwt,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

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
