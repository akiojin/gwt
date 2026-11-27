/* Issue #7248 — update apply failures never drop out of the UI.
 *
 * AC-1: a failure that arrives while no update flow is on screen (the CTA is
 *       idle, so no modal opens) is still recorded in the notification center.
 * AC-2: Settings status shows the last settled apply (outcome, target,
 *       expected / observed version, time), the session's last apply error,
 *       and a route to the update log.
 *
 * Runs against the embedded frontend with a stub backend, in both themes,
 * with zero console / page errors.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const lastApplyResult = {
  outcome: "failure", from_version: "9.1.0", to_version: "9.2.0", observed_version: "9.1.0",
  attempt: 2, recorded_at: "2026-11-11T00:00:00Z", message: "version mismatch",
};

test.describe("Update failure visibility", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("an idle-time failure reaches the notification center and Settings status (AC-1, AC-2)", async ({ page }, info) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    await installEmbeddedRoutes(page);
    await page.addInitScript(({ appOrigin, theme, lastApplyResult }) => {
      if (location.origin === appOrigin) localStorage.setItem("gwt:ui:theme", theme);
      try { window.sessionStorage.setItem("gwt:ui:briefing", "1"); } catch { /* no-op */ }
      (window as any).__gwtPlaywrightTestBridge = true;
      const sent: any[] = [];
      (window as any).__updateSent = sent;
      const workspaceState = {
        kind: "workspace_state",
        workspace: {
          app_version: "playwright",
          tabs: [{
            id: "tab-1", title: "Update Fixture", project_root: "/fixture",
            project_key: location.pathname.split("/")[2], kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [{
                id: "settings-fixture", title: "Settings", preset: "settings", status: "running",
                persist: true, z_index: 1, geometry: { x: 40, y: 40, width: 900, height: 760 },
              }],
            },
          }],
          active_tab_id: "tab-1",
          recent_projects: [],
        },
      };
      class FixtureWebSocket extends EventTarget {
        static CONNECTING = 0;
        static OPEN = 1;
        static CLOSING = 2;
        static CLOSED = 3;
        constructor(url: string) {
          super();
          (this as any).url = url;
          (this as any).readyState = FixtureWebSocket.CONNECTING;
          (window as any).__updateSocket = this;
          setTimeout(() => {
            (this as any).readyState = FixtureWebSocket.OPEN;
            this.dispatchEvent(new Event("open"));
          }, 0);
        }
        send(raw: string) {
          const message = JSON.parse(raw);
          sent.push(message);
          if (message.kind === "frontend_ready") this.emit(workspaceState);
          if (message.kind === "get_update_settings") {
            this.emit({ kind: "update_settings", project_root: "/fixture", status: {
              auto_apply_updates: true, auto_apply_updates_effective: true, update_drain: null,
              last_apply_result: lastApplyResult,
            } });
          }
        }
        close() {
          (this as any).readyState = FixtureWebSocket.CLOSED;
          this.dispatchEvent(new CloseEvent("close"));
        }
        emit(payload: unknown) {
          setTimeout(() => this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) })), 0);
        }
      }
      Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
    }, { appOrigin: new URL(APP_URL).origin, theme, lastApplyResult });
    await page.goto(APP_URL);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const settings = page.locator('.workspace-window[data-preset="settings"]');
    await expect(settings).toBeVisible();
    await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { target: "system" } })));
    const status = settings.locator("[data-update-settings-status]");

    // AC-2: the settled failure names expected and observed versions.
    await expect(status.locator("[data-update-last-apply-result]")).toHaveText(
      "Last update: Failed · v9.1.0 → v9.2.0 · expected v9.2.0, observed v9.1.0 · 2026-11-11T00:00:00Z · version mismatch",
    );

    // AC-1: no update flow is on screen, so no modal opens for this failure.
    await page.evaluate(() => (window as any).__updateSocket.emit({
      kind: "update_apply_error", stage: "Replace staged update",
      reason: "Could not remove the staged payload", log_path: "/fixture/.gwt/logs/update.log",
    }));
    await expect(status.locator("[data-update-last-apply-error]")).toContainText(
      "Last apply error: Replace staged update — Could not remove the staged payload",
    );
    await expect(page.locator("#update-modal")).toHaveCount(0);
    await status.getByRole("button", { name: "Open update log" }).click();
    expect(await page.evaluate(() => (window as any).__updateSent.filter((message: any) => message.kind === "open_update_log")))
      .toEqual([{ kind: "open_update_log", log_path: "/fixture/.gwt/logs/update.log" }]);
    await page.screenshot({ path: info.outputPath(`update-failure-settings-${theme}.png`) });

    await page.locator("#op-notifications-button").click();
    const drawer = page.locator("#notification-center");
    await expect(drawer).toBeVisible();
    await expect(drawer).toContainText("Update failed: Replace staged update");
    await expect(drawer).toContainText("Could not remove the staged payload");
    await expect(drawer).toContainText("Update failed: v9.1.0 → v9.2.0");
    await page.screenshot({ path: info.outputPath(`update-failure-notifications-${theme}.png`) });
    expect(errors, "zero console/page errors").toEqual([]);
  });
});
