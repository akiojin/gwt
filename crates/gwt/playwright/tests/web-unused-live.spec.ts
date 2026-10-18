/** Issue #5601 AC-3: real checkout startup and loaded bytes before/after web cleanup. */
import { execFileSync } from "node:child_process";
import { readFile, readdir, realpath, writeFile } from "node:fs/promises";
import { basename, isAbsolute, join, relative } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const ROOT = process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? "";
const PROJECT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";

async function startupSamples() {
  const directory = join(HOME, ".gwt/logs/perf");
  const files = (await readdir(directory).catch(() => []))
    .filter(file => /^perf-\d{4}-\d{2}-\d{2}\.jsonl$/.test(file)).sort();
  const samples: Array<Record<string, unknown>> = [];
  for (const file of files) {
    const text = await readFile(join(directory, file), "utf8");
    // Ignore a concurrent writer's unfinished last line; the next poll reads it.
    for (const line of text.slice(0, text.lastIndexOf("\n") + 1).split("\n").filter(Boolean)) {
      const row = JSON.parse(line);
      if (row.type === "sample" && row.stream === "ui" && row.target === "route:startup") samples.push(row);
    }
  }
  return samples;
}

test.describe("Issue #5601 web cleanup (isolated live checkout)", () => {
  test.skip(!BASE || !HOME || !ROOT || !PROJECT, "requires browser-check checkout and fresh HOME");
  test.use({ viewport: { width: 1440, height: 1000 } });
  test.setTimeout(120_000);

  test("Issue #5601: startup and loaded bytes preserve rail and Settings behavior", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const home = await realpath(HOME);
      const project = relative(home, await realpath(PROJECT));
      expect(basename(home)).toMatch(/^gwt-issue-5595-home-/);
      expect(isAbsolute(project) || project.startsWith("..")).toBe(false);
      const phase = process.env.GWT_5601_PHASE ?? "verification";
      expect(phase).toMatch(/^[a-zA-Z0-9_-]+$/);
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(page.locator("#op-rail")).toBeVisible({ timeout: 30_000 });
      await expect.poll(async () => (await startupSamples()).length, { timeout: 30_000 }).toBe(1);

      // Keep the disposable Normal Git fixture intact while releasing its
      // unrelated migration focus trap, using the same renderer as the app.
      await page.evaluate(async () => {
        const modalEl = document.getElementById("migration-modal");
        if (!modalEl?.classList.contains("open")) return;
        const modulePath = "/migration-modal.js";
        const { renderMigrationModal } = await import(modulePath);
        renderMigrationModal({ modalEl, dialogEl: modalEl.querySelector(".modal-shell"),
          state: { migrationModal: { open: false } } });
      });

      const rail = page.locator("#op-rail");
      const entry = (surface: string) => rail.locator(`[data-surface="${surface}"]`);
      await expect(rail.locator(".op-rail__surface .op-rail__surface-label")).toHaveText(["Issues", "Agents", "Board", "Settings"]);
      await entry("settings").click();
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      await expect(settings).toBeVisible({ timeout: 30_000 });
      await settings.getByRole("tab", { name: "System", exact: true }).click();
      await expect(settings.getByRole("tab", { name: "System", exact: true })).toHaveAttribute("aria-selected", "true");
      await expect(settings.locator('[data-settings-panel="system"]')).toBeVisible();
      await expect(settings.getByLabel("Output Language", { exact: true })).toBeEnabled({ timeout: 30_000 });
      await info.attach(`${phase}-settings-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      await settings.getByRole("tab", { name: "Issues", exact: true }).click();
      await expect(settings.getByRole("tab", { name: "Issues", exact: true })).toHaveAttribute("aria-selected", "true");
      await expect(settings.locator('[data-settings-panel="issues"]')).toBeVisible();
      await entry("agents").click();
      await expect(entry("agents")).toHaveAttribute("aria-pressed", "true");
      await entry("issues").click();
      await expect(entry("issues")).toHaveAttribute("aria-pressed", "true");
      await entry("settings").click();
      await expect(settings).toBeVisible();
      await expect(settings).toHaveCount(1);
      await expect(page.locator("#split-surfaces")).toBeVisible();

      const loaded = await page.evaluate(() => {
        const entries = [...performance.getEntriesByType("navigation"), ...performance.getEntriesByType("resource")] as
          Array<PerformanceNavigationTiming | PerformanceResourceTiming>;
        const resources = entries.filter(entry => new URL(entry.name).origin === location.origin)
          .map(entry => ({ path: new URL(entry.name).pathname, initiator_type: entry.initiatorType,
            transfer_bytes: entry.transferSize, encoded_body_bytes: entry.encodedBodySize,
            decoded_body_bytes: entry.decodedBodySize }));
        return { resources, total_transfer_bytes: resources.reduce((sum, entry) => sum + entry.transfer_bytes, 0),
          total_encoded_body_bytes: resources.reduce((sum, entry) => sum + entry.encoded_body_bytes, 0),
          total_decoded_body_bytes: resources.reduce((sum, entry) => sum + entry.decoded_body_bytes, 0) };
      });
      expect(loaded.total_decoded_body_bytes).toBeGreaterThan(0);
      expect(loaded.resources.some(resource => resource.path === "/app.js")).toBe(true);
      expect(errors, "zero console and page errors across real rail/Settings navigation").toEqual([]);
      const evidence = {
        phase, theme, sha: execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim(),
        isolated_home: home, startup_samples: await startupSamples(),
        startup_scope: "one process-wide sample per fresh runner, shared by both theme projects",
        loaded, errors,
      };
      const evidencePath = info.outputPath(`${phase}-web-unused-${theme}.json`);
      await writeFile(evidencePath, JSON.stringify(evidence, null, 2));
      await info.attach(`${phase}-web-unused-${theme}`, { path: evidencePath, contentType: "application/json" });
      await info.attach(`${phase}-rail-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      console.log(`Issue #5601 ${phase} ${theme}: route:startup=${evidence.startup_samples[0].value}ms; loaded=${loaded.total_decoded_body_bytes} decoded bytes`);
    });
  });
});
