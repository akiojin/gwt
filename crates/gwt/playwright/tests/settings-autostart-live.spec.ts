/** Issue #5532 / SPEC #5390: delayed login-launch replies stay local to Settings. */
import { expect, test, type WebSocketRoute } from "@playwright/test";
import { gotoLiveGwt, openLiveGwtProject, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const PROJECT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? process.cwd();

test.describe("Autostart Settings (isolated live backend)", () => {
  test.skip(!BASE, "requires browser-check checkout and fresh HOME");
  test.use({ viewport: { width: 1440, height: 1000 } });
  test.setTimeout(120_000);

  test.afterEach(async ({ page }, info) => {
    const received = await page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .map(({ sequence, payload }: any) => ({ sequence, kind: payload.kind })));
    await info.attach("settings-backend-replies", {
      body: JSON.stringify(received), contentType: "application/json",
    });
  });

  test("delayed status, query failure, update failure and retry keep other settings responsive", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);

      // Forward the real backend protocol except login-launch operations. Holding
      // their replies makes slowness deterministic and prevents OS login changes.
      const requests: Array<{ kind: string; enabled?: boolean }> = [];
      let autostartSocket: WebSocketRoute | undefined;
      await page.routeWebSocket(/\/ws(?:\?|$)/, socket => {
        const server = socket.connectToServer();
        socket.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (payload?.kind === "get_autostart_status" || payload?.kind === "update_autostart") {
            requests.push(payload);
            autostartSocket = socket;
          } else server.send(message);
        });
        server.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (payload?.kind !== "autostart_status" && payload?.kind !== "autostart_error") socket.send(message);
        });
      });
      const reply = (payload: object) => {
        expect(autostartSocket, "an autostart request reached the controlled transport").toBeDefined();
        autostartSocket!.send(JSON.stringify(payload));
      };
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await openLiveGwtProject(page, PROJECT);
      await page.bringToFront();
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      const open = async () => {
        // A new page can restore the previous theme's Settings window. Focusing
        // that window sends no read; its initial snapshot is already in flight.
        const before = await settings.count() === 0
          ? await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0)
          : 0;
        await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "system" } })));
        await expect(settings).toBeVisible();
        await page.waitForFunction(before => ((window as any).__gwtPlaywrightMessages ?? [])
          .some(({ sequence, payload }: any) => sequence > before && payload.kind === "system_settings"),
        before, { timeout: 30_000 });
      };
      const close = async () => {
        await settings.getByRole("button", { name: "Close window", exact: true }).click();
        const confirmation = page.locator("#window-close-confirm-modal");
        await expect(confirmation).toBeVisible();
        await confirmation.getByRole("button", { name: "Close window", exact: true }).click();
        // Closing is a backend round trip, not a latency assertion for this Issue.
        await expect(settings).toHaveCount(0, { timeout: 30_000 });
      };
      const panel = settings.locator('[data-system-settings-content]');
      const toggle = panel.getByLabel("Launch GWT at login", { exact: true });
      const status = panel.locator('[data-role="autostart-settings-status"]');
      const systemStatus = panel.locator('[data-role="system-settings-status"]');
      const language = panel.getByLabel("Output Language", { exact: true });
      const saveLanguage = async (value: string) => {
        const before = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
        await language.selectOption(value);
        await page.waitForFunction(({ before, value }) => ((window as any).__gwtPlaywrightMessages ?? [])
          .some(({ sequence, payload }: any) => sequence > before
            && payload.kind === "system_settings_updated" && payload.language === value),
        { before, value }, { timeout: 30_000 });
        await expect(systemStatus).toHaveText("Saved system settings.", { timeout: 30_000 });
        await expect(language).toHaveValue(value);
      };

      await open();
      await expect.poll(() => requests.filter(request => request.kind === "get_autostart_status").length).toBeGreaterThan(0);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(toggle).toBeDisabled();
      await expect(status).toHaveText("Checking login launch status…");
      const queriesBeforeCheckingReopen = requests.filter(request => request.kind === "get_autostart_status").length;
      await close();
      await open();
      await expect(toggle).toBeDisabled();
      await expect(status).toHaveText("Checking login launch status…");
      await settings.getByRole("tab", { name: "Supported Agents", exact: true }).click();
      await expect(settings.locator('[data-settings-panel="supported-agents"] tbody [data-agent-id]').first())
        .toBeVisible({ timeout: 60_000 });
      await expect(settings.locator('[data-settings-panel="supported-agents"]')).not.toContainText("Loading agent detection…");
      await settings.getByRole("tab", { name: "System", exact: true }).click();
      await expect(status).toHaveText("Checking login launch status…");
      await expect(language).toBeEnabled();
      const originalLanguage = await language.inputValue();
      const nextLanguage = originalLanguage === "ja" ? "en" : "ja";
      await saveLanguage(nextLanguage);
      await expect(status).toHaveText("Checking login launch status…");
      expect(requests.filter(request => request.kind === "get_autostart_status").length,
        "reopening while checking reuses the in-flight query").toBe(queriesBeforeCheckingReopen);

      reply({ kind: "autostart_error", message: "Fixture status query unavailable." });
      await expect(status).toHaveText("Login launch status unavailable. Fixture status query unavailable.");
      await expect(toggle).toBeDisabled();
      await expect(systemStatus).toHaveText("Saved system settings.");
      await settings.evaluate(element => element.querySelector('[data-role="autostart-settings-status"]')
        ?.scrollIntoView({ block: "center" }));
      await info.attach(`autostart-unavailable-${theme}`, { body: await settings.screenshot(), contentType: "image/png" });

      const queriesBeforeReopen = requests.filter(request => request.kind === "get_autostart_status").length;
      await close();
      await open();
      await expect.poll(() => requests.filter(request => request.kind === "get_autostart_status").length).toBeGreaterThan(queriesBeforeReopen);
      await expect(status).toHaveText("Checking login launch status…");
      await expect(toggle).toBeDisabled();
      reply({ kind: "autostart_status", enabled: true, mechanism: "fixture", install_path: "/fixture/gwt" });
      await expect(toggle).toBeEnabled();
      await expect(toggle).toBeChecked();

      await toggle.uncheck();
      await expect(toggle).toBeDisabled();
      await expect(status).toHaveText("Saving…");
      await expect.poll(() => requests.filter(request => request.kind === "update_autostart")).toEqual([
        { kind: "update_autostart", enabled: false },
      ]);
      await saveLanguage(originalLanguage);
      await expect(status).toHaveText("Saving…");
      reply({ kind: "autostart_error", message: "Fixture login setting denied." });
      await expect(status).toHaveText("Fixture login setting denied.");
      await expect(toggle).toBeEnabled();
      await expect(toggle).toBeChecked();
      await expect(systemStatus).toHaveText("Saved system settings.");

      await toggle.uncheck();
      const queriesBeforePendingReopen = requests.filter(request => request.kind === "get_autostart_status").length;
      await close();
      await open();
      await expect(status).toHaveText("Saving…");
      await expect(toggle).toBeDisabled();
      expect(requests.filter(request => request.kind === "get_autostart_status").length,
        "reopening while saving cannot race the update with a new query").toBe(queriesBeforePendingReopen);
      reply({ kind: "autostart_status", enabled: false, mechanism: "fixture", install_path: "/fixture/gwt" });
      await expect(toggle).toBeEnabled();
      await expect(toggle).not.toBeChecked();
      await expect(status).toHaveText("Saved login launch setting.");
      await expect(systemStatus).toHaveText("Saved system settings.");
      await settings.evaluate(element => element.querySelector('[data-role="autostart-settings-status"]')
        ?.scrollIntoView({ block: "center" }));
      await info.attach(`autostart-saved-${theme}`, { body: await settings.screenshot(), contentType: "image/png" });
      expect(errors, "zero console/page errors through query failure and update recovery").toEqual([]);
      // The page fixture and wrapper's owned-process cleanup close the final window.
    });
  });
});
