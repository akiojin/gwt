import { readFile, realpath, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
test.use({ headless: false, viewport: { width: 1440, height: 1000 } });

test.describe("Issue #5351 cross-project update drain", () => {
  test.skip(!BASE || !process.env.GWT_PLAYWRIGHT_UPDATE_DRAIN, "requires verify-issue-5351.ps1 isolation");
  test.setTimeout(120_000);

  test("an applied drain in A is persisted away and admits a real agent pane in B", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      expect(await realpath(CHECK_HOME)).not.toBe(await realpath(homedir()));
      const fixture = JSON.parse(await readFile(join(CHECK_HOME, "fixture.json"), "utf8"));
      expect(await realpath(fixture.checkout)).toBe(await realpath(process.env.GWT_PLAYWRIGHT_PROJECT_ROOT!));
      for (const prefs of fixture.preferences) {
        const state = JSON.parse(await readFile(prefs, "utf8"));
        expect(state.enabled).toBe(false);
        expect(JSON.parse(await readFile(join(prefs, "../pm.json"), "utf8")).settings.auto_start).toBe(false);
      }
      expect(new URL(BASE).origin).toBe(new URL((await readFile(join(CHECK_HOME, "url.txt"), "utf8")).trim()).origin);
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");

      // Seed after startup: exercise launch admission or the next live scan,
      // rather than relying on startup marker migration.
      const prefs = JSON.parse(await readFile(fixture.preferences[1], "utf8"));
      prefs.update_drain = { version: fixture.version, since: new Date().toISOString(), reason: "auto" };
      await writeFile(fixture.preferences[1], JSON.stringify(prefs));
      const receiptsBefore = (await readFile(fixture.provider_log, "utf8")).split("\n").filter(Boolean).length;
      const existingIds = await page.locator(".workspace-window").evaluateAll(nodes => nodes.map(node => (node as HTMLElement).dataset.id));
      await sendLiveGwtEvent(page, { kind: "create_window", preset: "codex", bounds: { x: 64, y: 64, width: 900, height: 520 } });
      const id = await page.waitForFunction(before => {
        const pane = Array.from(document.querySelectorAll('.workspace-window[data-preset="codex"]'))
          .find(node => !before.includes((node as HTMLElement).dataset.id));
        return (pane as HTMLElement | undefined)?.dataset.id;
      }, existingIds).then(handle => handle.jsonValue());
      const pane = page.locator(`.workspace-window[data-id="${id}"]`);
      try {
        // This fixture provider runs in the actual PTY and does not call a model.
        await expect.poll(async () => (await readFile(fixture.provider_log, "utf8")).split("\n").filter(Boolean).length, { timeout: 30_000 }).toBeGreaterThan(receiptsBefore);
        const receipts = (await readFile(fixture.provider_log, "utf8")).trim().split("\n").map(line => JSON.parse(line));
        expect(receipts.at(-1).cwd.replace(/\\/g, "/").toLowerCase()).toBe(fixture.checkout.replace(/\\/g, "/").toLowerCase());
        await sendLiveGwtEvent(page, { kind: "list_windows" });
        await page.waitForFunction(windowId => ((window as any).__gwtPlaywrightMessages ?? []).some(({ payload }: any) =>
          (payload.kind === "window_list" && payload.windows.some((window: any) => window.id === windowId && ["running", "idle"].includes(window.status)))
          || (payload.kind === "window_state" && payload.window_id === windowId && ["running", "idle"].includes(payload.state))), id, { timeout: 30_000 });
        await expect.poll(async () => JSON.parse(await readFile(fixture.preferences[1], "utf8")).update_drain ?? null).toBeNull();
        await page.screenshot({ path: join(CHECK_HOME, `update-drain-${testInfo.project.name}.png`), fullPage: true });
        expect(errors, "console and page errors during cross-project admission").toEqual([]);
      } finally {
        if (!page.isClosed() && await pane.count()) await sendLiveGwtEvent(page, { kind: "close_window", id });
      }
    });
  });
});
