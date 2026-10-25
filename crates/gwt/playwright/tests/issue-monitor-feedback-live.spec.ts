import { spawnSync } from "node:child_process";
import { readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";

test.describe("External feedback intake (isolated live backend)", () => {
  test.skip(process.env.GWT_PLAYWRIGHT_FEEDBACK_INTAKE !== "1" || !BASE,
    "requires verify-issue-4810-intake.mjs and its dedicated offline fixture");
  test.setTimeout(180_000);
  test.use({ viewport: { width: 1600, height: 1100 } });

  test("retains raw/external exclusions and the trusted repair queue", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const home = await realpath(CHECK_HOME);
      expect(home).not.toBe(await realpath(homedir()));
      const fixture = JSON.parse(await readFile(join(home, "issue-4810-isolated.json"), "utf8"));
      expect(new URL(BASE).origin).toBe(new URL((await readFile(join(home, "url"), "utf8")).trim()).origin);
      expect(fixture.gwtd).toBe(join(fixture.checkout, "target/debug/gwtd"));
      const preferences = join(home, ".gwt/projects", fixture.repo_hash, "project-state");
      expect(JSON.parse(await readFile(join(preferences, "pm.json"), "utf8")).settings.auto_start).toBe(false);
      const prefs = JSON.parse(await readFile(join(preferences, "issue-monitor.json"), "utf8"));
      expect(prefs).toMatchObject({ enabled: false, autonomous_mode: false, launch_auto: false, terminal_queue_auto_refill: false });
      const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GWT_")
        && !["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN", "GH_HOST"].includes(key)));
      Object.assign(env, { HOME: home, USERPROFILE: home, GWT_TEST_GH: fixture.fake_gh, GWT_TEST_GH_SANDBOX: "1" });
      const cli = () => {
        const result = spawnSync(fixture.gwtd, [], { cwd: fixture.project, env, encoding: "utf8", timeout: 30_000,
          input: JSON.stringify({ schema_version: 1, operation: "issue.monitor.status", params: {} }) });
        expect(result.status, result.stderr).toBe(0);
        const envelope = JSON.parse(result.stdout);
        expect(envelope.ok).toBe(true);
        return JSON.parse(envelope.output);
      };
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator("#close-project-button")).toBeVisible();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await page.locator('.op-rail [data-surface="issues"]').click();
      const surface = page.locator('.workspace-window[data-preset="settings"] [data-settings-panel="issues"]:visible');
      await expect(surface).toBeVisible();
      // This existing control requests a real scan even with launches disabled.
      await sendLiveGwtEvent(page, { kind: "set_issue_monitor_allowed_labels", allowed_labels: [] });
      const inbox = await page.waitForFunction(({ raw, external, trusted }) => {
        const rows = ((window as any).__gwtPlaywrightMessages ?? []).filter(({ payload }: any) => payload.kind === "issue_monitor_inbox").at(-1)?.payload.items ?? [];
        return [raw, external, trusted].every(number => rows.some((row: any) => row.issue.number === number))
          && rows.some((row: any) => row.issue.number === raw && /untrusted feedback/.test(row.exclusion_reason || ""))
          && rows.some((row: any) => row.issue.number === external && /author|trusted|triage/i.test(row.exclusion_reason || "")) ? rows : null;
      }, fixture, { timeout: 90_000 }).then(handle => handle.jsonValue());
      for (const number of [fixture.raw, fixture.external]) {
        const entry = inbox.find((row: any) => row.issue.number === number);
        expect(entry.issue.author_login).toBe(number === fixture.raw ? "akiojin" : "external-reporter");
        expect(entry.state).toMatch(/source_excluded|not_ready/);
        expect(entry.claim_id ?? null).toBeNull();
        expect(entry.delivery_id ?? null).toBeNull();
        expect(entry.launched_window_id ?? null).toBeNull();
        const card = surface.locator(`.knowledge-row[data-issue-number="${number}"]:visible, .kanban-card[data-issue-number="${number}"]:visible`).first();
        await expect(card).toBeVisible();
        await expect(card).toContainText(entry.exclusion_reason);
        if (entry.state === "source_excluded") {
          await expect(card.locator(".knowledge-row-badge")).toHaveText("Needs triage");
          await expect(card.locator(".knowledge-row-badge")).toHaveAttribute("data-tone", "needs-input");
        }
      }
      const status = cli();
      expect(status).toMatchObject({ enabled: false, autonomous_mode: false, active_launches: [] });
      expect(status.gui_status.terminal_queue.map((row: any) => row.number)).toContain(fixture.trusted);
      expect(inbox.find((row: any) => row.issue.number === fixture.trusted).exclusion_reason ?? null).toBeNull();
      expect(status.inbox.filter((row: any) => [fixture.raw, fixture.external].includes(row.issue_number))
        .every((row: any) => !row.claim_id && !row.delivery_id && !row.launched_window_id)).toBe(true);
      const launchedWindows = await page.evaluate(() => {
        const workspace = (window as any).__gwtPlaywrightMessages.findLast((entry: any) => entry.payload.kind === "workspace_state").payload.workspace;
        return workspace.tabs.flatMap((tab: any) => tab.workspace.windows)
          .filter((pane: any) => ["agent", "codex", "claude", "shell"].includes(pane.preset));
      });
      expect(launchedWindows, "disabled intake creates no agent or Shell window").toEqual([]);
      await expect(page.locator('.work-row, .workspace-window[data-preset="shell"]')).toHaveCount(0);
      const screenshot = info.outputPath(`feedback-intake-${theme}.png`);
      await page.screenshot({ path: screenshot, fullPage: true });
      await info.attach(`feedback-intake-${theme}`, { path: screenshot, contentType: "image/png" });
      await info.attach("feedback-intake-status", { body: JSON.stringify({ inbox, status }, null, 2), contentType: "application/json" });
      expect(errors, "console/page errors").toEqual([]);
    });
  });
});
