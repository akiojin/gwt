import { expect, test } from "@playwright/test";
import { gotoLiveGwt, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test("Issue #6992: Wedged status exposes a cancelable confirmation and sends one explicit restart", async ({ page }, info) => {
  test.skip(!BASE || !process.env.GWT_PLAYWRIGHT_CHECK_HOME, "requires browser-check checkout binaries and fresh HOME");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => {
    localStorage.setItem("gwt:ui:theme", theme);
    const state = window as any;
    state.__ownerRestartSent = [];
    const original = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data: string | ArrayBufferLike | Blob | ArrayBufferView) {
      if (typeof data === "string") {
        const event = JSON.parse(data);
        if (event.kind === "restart_wedged_owner") {
          // Exercise the real frontend send path without terminating this
          // fixture's healthy daemon. Rust tests own signal/escalation proof.
          state.__ownerRestartSent.push(event);
          return;
        }
      }
      return original.call(this, data);
    };
  }, theme);
  await withLiveGwtBackendLock(BASE, info, async () => {
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const injectHealth = async (state: string) => page.evaluate(state => {
      const projectKey = location.pathname.split("/")[2];
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
        kind: "project_owner_health", health: { project_key: projectKey, state, recovery_id: state === "wedged" ? "wedge-test-1" : null,
          detail: state === "wedged" ? "Repeated owner health timeouts; diagnostics saved" : null },
      } }));
    }, state);
    const health = page.locator("[data-project-owner-health]");
    const restart = health.locator("[data-owner-restart]");
    const modal = page.locator("#owner-restart-modal");
    await injectHealth("degraded");
    await expect(health).toContainText("Unavailable");
    await expect(restart).toBeHidden();
    await injectHealth("wedged");
    await expect(health).toContainText("Wedged");
    await restart.click();
    const dialog = modal.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveAttribute("aria-modal", "true");
    await expect(dialog).toContainText("all agents");
    await expect(dialog).toContainText("SIGTERM");
    await expect(dialog).toContainText("SIGKILL");
    await expect(dialog.getByRole("button", { name: "Cancel", exact: true })).toBeFocused();
    await page.screenshot({ path: info.outputPath(`owner-wedged-${theme}.png`), fullPage: true });
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(modal).not.toBeVisible();
    expect(await page.evaluate(() => (window as any).__ownerRestartSent)).toEqual([]);
    await injectHealth("wedged");
    await restart.click();
    await dialog.getByRole("button", { name: "Save diagnostics and restart", exact: true }).click();
    await expect(modal).not.toBeVisible();
    expect(await page.evaluate(() => (window as any).__ownerRestartSent)).toEqual([{ kind: "restart_wedged_owner", confirmed: true, recovery_id: "wedge-test-1" }]);
    await expect(restart).toBeDisabled();
    await injectHealth("healthy");
    await expect(health).toContainText("Ready");
    await expect(restart).toBeHidden();
    expect(errors, "console and page errors").toEqual([]);
  });
});
