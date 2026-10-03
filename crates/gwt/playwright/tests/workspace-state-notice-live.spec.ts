import { mkdir, readFile, realpath, rename, rmdir } from "node:fs/promises";
import { homedir } from "node:os";
import { isAbsolute, join, relative, sep } from "node:path";
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
