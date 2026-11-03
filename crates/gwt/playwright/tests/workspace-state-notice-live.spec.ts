import { access, mkdir, readFile, realpath, rename, rmdir } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, relative, sep } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const PROJECT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";

async function isolatedStatePath() {
  expect(CHECK_HOME, "requires a browser-check isolated HOME").not.toBe("");
  expect(PROJECT, "requires the checkout project root").not.toBe("");
  const checkHome = await realpath(CHECK_HOME);
  const project = await realpath(PROJECT);
  expect(checkHome).not.toBe(await realpath(homedir()));
  const marker = JSON.parse(await readFile(join(checkHome, "issue-4925-isolated.json"), "utf8"));
  expect(await realpath(marker.project)).toBe(project);
  expect(marker.repo_hash).toMatch(/^[a-zA-Z0-9_-]+$/);
  const stateDir = join(checkHome, ".gwt/projects", marker.repo_hash, "project-state");
  const statePath = join(stateDir, "current.json");
  // Resolve both the parent and file before mutating anything: an isolated
  // HOME must not lead through a junction/symlink to a real state directory.
  for (const candidate of [stateDir, statePath]) {
    const resolved = await realpath(candidate);
    const insideHome = relative(checkHome, resolved);
    expect(
      insideHome !== "" && insideHome !== ".." && !insideHome.startsWith(`..${sep}`) && !isAbsolute(insideHome),
      `state fixture must remain inside isolated HOME: ${resolved}`,
    ).toBe(true);
  }
  const pm = JSON.parse(await readFile(join(stateDir, "pm.json"), "utf8"));
  expect(pm.settings.auto_start).toBe(false);
  const servedUrl = (await readFile(join(checkHome, "url"), "utf8")).trim();
  expect(new URL(BASE).origin).toBe(new URL(servedUrl).origin);
  return statePath;
}

test.describe("Workspace state diagnostics", () => {
  test.skip(!BASE, "requires an isolated checkout gwt instance");
  test.setTimeout(90_000);
  test.use({ viewport: { width: 1440, height: 1000 } });

  test("notice bursts render the latest error once and recover with suspended rAF", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const theme = testInfo.project.name.endsWith("light") ? "light" : "dark";
      await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
      const retries: unknown[] = [];
      await page.routeWebSocket(/\/ws(?:\?|$)/, socket => {
        const server = socket.connectToServer();
        socket.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (payload?.kind === "retry_workspace_state_load") retries.push(payload);
          else server.send(message);
        });
        server.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (payload?.kind !== "workspace_state_notice") socket.send(message);
        });
      });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.bringToFront();
      const control = page.locator("#close-project-button");
      await expect(control).toBeVisible();
      await control.focus();
      await control.hover();
      const burst = await page.evaluate(async () => {
        const app = document.getElementById("app")!;
        const focused = document.activeElement;
        let renders = 0;
        const observer = new MutationObserver(records => {
          renders += records.flatMap(record => [...record.addedNodes])
            .filter(node => node instanceof Element && node.matches(".workspace-state-notice")).length;
        });
        observer.observe(app, { childList: true });
        try {
          for (let index = 1; index <= 50; index++) {
            window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
              kind: "workspace_state_notice", notice: { kind: "load_error",
                path: "/fixture/current.json", message: `Error <${index}>` },
            } }));
          }
          const deferred = !document.querySelector(".workspace-state-notice");
          await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
          const banner = document.querySelector(".workspace-state-notice")!;
          window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: { kind: "runtime_health", snapshot: {} } }));
          await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
          return { deferred, renders, text: banner.textContent,
            safeText: banner.querySelector("img, script") === null,
            unrelatedPreserved: document.querySelector(".workspace-state-notice") === banner,
            focusPreserved: document.activeElement === focused, hoverPreserved: focused?.matches(":hover") };
        } finally { observer.disconnect(); }
      });
      expect(burst).toMatchObject({ deferred: true, renders: 1, safeText: true,
        unrelatedPreserved: true, focusPreserved: true, hoverPreserved: true });
      expect(burst.text).toBe("/fixture/current.json: Error <50>Retry");
      await testInfo.attach("notice-frame-burst", { body: JSON.stringify(burst), contentType: "application/json" });
      const banner = page.locator('.workspace-state-notice[role="alert"]');
      await banner.getByRole("button", { name: "Retry", exact: true }).click();
      await expect.poll(() => retries.length).toBe(1);
      await expect(banner).toContainText("Error <50>");
      await page.evaluate(() => {
        (window as any).__noticeRaf = window.requestAnimationFrame;
        window.requestAnimationFrame = () => 0;
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: { kind: "workspace_state_notice",
          notice: { kind: "load_error", path: "/fixture/current.json", message: "Fallback error" } } }));
      });
      try {
        await expect(banner).toContainText("Fallback error", { timeout: 1000 });
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await testInfo.attach(`notice-frame-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
        await page.evaluate(() => window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
          detail: { kind: "workspace_state_notice", notice: null },
        })));
        await expect(banner).toHaveCount(0, { timeout: 1000 });
      } finally {
        await page.evaluate(() => { window.requestAnimationFrame = (window as any).__noticeRaf; });
      }
      expect(errors).toEqual([]);
    });
  });

  test("retired HOME layout is preserved and shows the upgrade path after reconnect", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      const statePath = await isolatedStatePath();
      const legacyDir = join(dirname(dirname(statePath)), "workspace");
      const legacyPath = join(legacyDir, "current.json");
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => {
        if (message.type() === "error") errors.push(message.text());
      });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator("#close-project-button")).toBeVisible();
      const original = await readFile(statePath);
      // Exclusive mkdir prevents overwriting any pre-existing legacy fixture.
      await mkdir(legacyDir);
      let moved = false;
      try {
        await rename(statePath, legacyPath);
        moved = true;
        await sendLiveGwtEvent(page, { kind: "retry_workspace_state_load" });
        const banner = page.locator('.workspace-state-notice[role="alert"]');
        await expect(banner).toContainText(join("workspace", "current.json"));
        const displayedPath = (await banner.locator("span").textContent())!.split(": ")[0];
        expect(await realpath(displayedPath)).toBe(legacyPath);
        await expect(banner).toContainText("v9.106.0");
        await page.reload();
        await expect(banner).toContainText("v9.72.1");
        await banner.getByRole("button", { name: "Retry", exact: true }).click();
        await expect(banner).toContainText("no longer imported");
        await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
        await page.screenshot({ path: join(CHECK_HOME, `retired-home-${testInfo.project.name}.png`), fullPage: true });
        expect(await readFile(legacyPath)).toEqual(original);
        await expect(access(statePath)).rejects.toThrow();
      } finally {
        if (moved) await rename(legacyPath, statePath);
        await rmdir(legacyDir);
        if (!page.isClosed()) await sendLiveGwtEvent(page, { kind: "retry_workspace_state_load" });
      }
      await expect(page.locator(".workspace-state-notice")).toHaveCount(0);
      expect(errors).toEqual([]);
    });
  });

  test("real read failure names the file, survives reconnect, and Retry reloads repaired state", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      const statePath = await isolatedStatePath();
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => {
        if (message.type() === "error") errors.push(message.text());
      });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator("#close-project-button")).toBeVisible();
      const original = await readFile(statePath);
      const backup = `${statePath}.e2e-${testInfo.workerIndex}-${Date.now()}`;
      await rename(statePath, backup);
      let blockedPath = false;
      let restored = false;
      try {
        // A directory produces an I/O failure on both Windows and Unix and
        // cannot be silently replaced by background projection rebuilds.
        await mkdir(statePath);
        blockedPath = true;
        await sendLiveGwtEvent(page, { kind: "retry_workspace_state_load" });
        const banner = page.locator('.workspace-state-notice[role="alert"]');
        await expect(banner).toBeVisible();
        await expect(banner).toContainText("current.json");
        await expect(banner).toContainText(/directory|denied|ディレクトリ|拒否|os error/i);
        await expect(banner.getByRole("button", { name: "Retry", exact: true })).toBeVisible();
        await page.reload();
        await expect(banner).toBeVisible();
        await expect(banner).toContainText("current.json");
        await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
        await page.screenshot({ path: join(CHECK_HOME, `workspace-state-notice-${testInfo.project.name}.png`), fullPage: true });
        expect(await readFile(backup)).toEqual(original);
        await rmdir(statePath);
        blockedPath = false;
        await rename(backup, statePath);
        restored = true;
        await banner.getByRole("button", { name: "Retry", exact: true }).click();
        await expect(page.locator(".workspace-state-notice")).toHaveCount(0);
        await expect(page.locator("#close-project-button")).toBeVisible();
      } finally {
        if (blockedPath) await rmdir(statePath);
        if (!restored) await rename(backup, statePath);
        if (!page.isClosed()) await sendLiveGwtEvent(page, { kind: "retry_workspace_state_load" });
      }
      expect(errors).toEqual([]);
    });
  });
});
