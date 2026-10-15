/** Issue #4249 AC-16: scoped CLI update preference changes reach live Settings. */
import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { readFile, realpath, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import {
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const REPO_ROOT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";
const GWTD = process.env.GWT_PLAYWRIGHT_GWTD ?? "";
const PROJECT_HASH = process.env.GWT_PLAYWRIGHT_PROJECT_HASH ?? "";

function fixtureApproval(enabled: boolean) {
  return {
    id: randomUUID(),
    scope: {
      operation: "update.settings.set",
      params_sha256: createHash("sha256")
        .update(JSON.stringify({ auto_apply_updates: enabled })).digest("hex"),
    },
    reason: "Isolated Issue #4249 headed E2E fixture",
    rollback: "Restore the isolated auto_apply_updates preference to false",
    standing: false,
    revoked: false,
    consumed: false,
    created_at: new Date().toISOString(),
  };
}

function command(operation: string, params: object = {}) {
  const env: NodeJS.ProcessEnv = { ...process.env, HOME: CHECK_HOME, USERPROFILE: CHECK_HOME };
  // Invoke this checkout directly, without the parent agent's hook authority.
  for (const key of Object.keys(env)) {
    if (key.startsWith("GWT_")) delete env[key];
  }
  delete env.GH_TOKEN;
  delete env.GITHUB_TOKEN;
  const reply = JSON.parse(execFileSync(GWTD, [], {
    cwd: REPO_ROOT, env, encoding: "utf8", windowsHide: true, timeout: 30_000,
    input: JSON.stringify({ schema_version: 1, operation, params }),
  }));
  expect(reply.ok, `${operation}: ${JSON.stringify(reply)}`).toBe(true);
  return JSON.parse(reply.output);
}

async function setPreference(enabled: boolean, approvalId: string): Promise<void> {
  const result = command("update.settings.set", {
    auto_apply_updates: enabled, approval_id: approvalId,
  });
  expect(result.after.auto_apply_updates).toBe(enabled);
  expect(command("update.settings.status")).toMatchObject({
    auto_apply_updates: enabled,
    auto_apply_updates_effective: enabled,
  });
  expect(await readFile(join(CHECK_HOME, ".gwt/config.toml"), "utf8"))
    .toMatch(new RegExp(`^auto_apply_updates\\s*=\\s*${enabled}\\s*$`, "m"));
}

test.use({ headless: false, viewport: { width: 1440, height: 900 } });

test.describe("Issue #4249 PM update settings", () => {
  test.skip(!BASE || !CHECK_HOME || !REPO_ROOT || !GWTD || !PROJECT_HASH,
    "requires the Issue #4249 browser-check checkout and fresh HOME fixture");
  test.setTimeout(90_000);

  test("scoped CLI changes persist and reload in System Settings", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const home = await realpath(CHECK_HOME);
      expect(home).not.toBe(await realpath(homedir()));
      expect(PROJECT_HASH).toMatch(/^[a-zA-Z0-9_-]+$/);
      const state = join(home, ".gwt/projects", PROJECT_HASH, "project-state");
      expect(await realpath(state)).toBe(state);
      expect(JSON.parse(await readFile(join(state, "pm.json"), "utf8"))
        .settings.auto_start).toBe(false);
      expect(JSON.parse(await readFile(join(state, "issue-monitor.json"), "utf8"))
        .enabled).toBe(false);

      const storePath = join(state, "pm-authorizations.json");
      const previous = await readFile(storePath).catch(error => {
        if (error.code === "ENOENT") return null;
        throw error;
      });
      const store = previous ? JSON.parse(previous.toString("utf8"))
        : { approvals: [], consumed_instructions: [] };
      // Test-only authority; these fixtures are not native human approvals.
      const enable = fixtureApproval(true);
      const disable = fixtureApproval(false);
      const cleanup = fixtureApproval(false);
      store.approvals.push(enable, disable, cleanup);
      await writeFile(storePath, JSON.stringify(store));

      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => {
        if (message.type() === "error") errors.push(message.text());
      });
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      try {
        await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
        await gotoLiveGwt(page, BASE, {
          enableTestBridge: true, suppressUpdateApplyStart: true,
        });
        await openLiveGwtProject(page, REPO_ROOT);
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await setPreference(true, enable.id);
        await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", {
          detail: { section: "system" },
        })));
        const settings = page.locator('.workspace-window[data-preset="settings"]');
        const system = settings.locator('[data-settings-panel="system"]');
        const toggle = system.getByLabel("Auto-apply updates", { exact: true });
        const status = system.locator("[data-update-settings-status]");
        await expect(system).toBeVisible();
        await expect(toggle).toBeEnabled();
        await expect(toggle).toBeChecked();
        await expect(status).toContainText("Saved: On · Effective: On");
        await info.attach(`pm-update-settings-${theme}`, {
          body: await settings.screenshot(), contentType: "image/png",
        });

        await setPreference(false, disable.id);
        await sendLiveGwtEvent(page, { kind: "get_update_settings" });
        await expect(toggle).not.toBeChecked();
        await expect(status).toContainText("Saved: Off · Effective: Off");
      } finally {
        try {
          await setPreference(false, cleanup.id);
        } finally {
          if (previous) await writeFile(storePath, previous);
          else await rm(storePath, { force: true });
        }
      }
      expect(errors, "console and page errors while reloading update settings").toEqual([]);
    });
  });
});
