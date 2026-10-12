import { readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const PROJECT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";

test.use({ headless: false, viewport: { width: 1440, height: 1000 } });

test.describe("Issue #5248 pane close snapshot", () => {
  test.skip(!BASE, "requires an isolated checkout gwt instance");
  test.setTimeout(120_000);

  test("closing a live pane removes it from the canvas and window list", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      expect(CHECK_HOME, "requires browser-check isolation").not.toBe("");
      expect(await realpath(CHECK_HOME)).not.toBe(await realpath(homedir()));
      const marker = JSON.parse(await readFile(join(CHECK_HOME, "issue-5248-isolated.json"), "utf8"));
      expect(await realpath(marker.project)).toBe(await realpath(PROJECT));
      const stateRoot = join(CHECK_HOME, ".gwt/projects", marker.repo_hash, "project-state");
      expect(JSON.parse(await readFile(join(stateRoot, "pm.json"), "utf8")).settings.auto_start).toBe(false);
      expect(JSON.parse(await readFile(join(stateRoot, "issue-monitor.json"), "utf8")).enabled).toBe(false);
      const servedUrl = (await readFile(join(CHECK_HOME, "url"), "utf8")).trim();
      expect(new URL(BASE).origin).toBe(new URL(servedUrl).origin);

      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => {
        if (message.type() === "error") errors.push(message.text());
      });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator("#close-project-button")).toBeVisible();
      await expect(page.locator("html")).toHaveAttribute(
        "data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark",
      );
      const existingIds = await page.locator(".workspace-window").evaluateAll(nodes =>
        nodes.map(node => (node as HTMLElement).dataset.id));
      await sendLiveGwtEvent(page, {
        kind: "create_window", preset: "shell",
        bounds: { x: 64, y: 64, width: 720, height: 420 },
      });
      const id = await page.waitForFunction(before => {
        const pane = Array.from(document.querySelectorAll('.workspace-window[data-preset="shell"]'))
          .find(node => !before.includes((node as HTMLElement).dataset.id));
        return (pane as HTMLElement | undefined)?.dataset.id;
      }, existingIds).then(handle => handle.jsonValue());
      expect(id).toBeTruthy();
      const pane = page.locator(`.workspace-window[data-id="${id}"]`);
      try {
        // Query the authoritative list so this close targets an actual running
        // PTY, rather than only a freshly inserted frontend node.
        await sendLiveGwtEvent(page, { kind: "list_windows" });
        await page.waitForFunction(windowId => ((window as any).__gwtPlaywrightMessages ?? [])
          .some(({ payload }: any) => (payload.kind === "window_list"
            && payload.windows.some((window: any) => window.id === windowId && window.status === "running"))
            || (payload.kind === "window_state" && payload.window_id === windowId && payload.state === "running")), id);
        const cursor = await page.evaluate(() => Number((window as any).__gwtPlaywrightMessageSequence));
        await sendLiveGwtEvent(page, { kind: "close_window", id });
        await expect(pane).toHaveCount(0);
        await sendLiveGwtEvent(page, { kind: "list_windows" });
        const snapshots = await page.waitForFunction(({ cursor, id }) => {
          const frames = ((window as any).__gwtPlaywrightMessages ?? [])
            .filter((entry: any) => entry.sequence > cursor);
          const canvas = frames.filter(({ payload }: any) => payload.kind === "workspace_state").at(-1)?.payload;
          const list = frames.filter(({ payload }: any) => payload.kind === "window_list").at(-1)?.payload;
          if (!canvas || !list) return null;
          const canvasIds = canvas.workspace.tabs.flatMap((tab: any) => tab.workspace.windows.map((window: any) => window.id));
          const listIds = list.windows.map((window: any) => window.id);
          return !canvasIds.includes(id) && !listIds.includes(id) ? { canvasIds, listIds } : null;
        }, { cursor, id }, { timeout: 10_000 }).then(handle => handle.jsonValue());
        expect(snapshots.canvasIds).not.toContain(id);
        expect(snapshots.listIds).not.toContain(id);
        await page.screenshot({ path: join(CHECK_HOME, `close-snapshot-${testInfo.project.name}.png`), fullPage: true });
        // Daemon snapshot identities are not a public status field. The Rust
        // transport regression checks that consumer; this run covers the real
        // browser close path and its frontend projections.
        expect(errors, "console and page errors during pane close").toEqual([]);
      } finally {
        if (!page.isClosed() && await pane.count()) {
          await sendLiveGwtEvent(page, { kind: "close_window", id });
        }
      }
    });
  });
});
