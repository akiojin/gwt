/** Issue #5694: PM ConfigSet persists through the live monitor control lane. */
import { spawnSync } from "node:child_process";
import { readFile, readdir, realpath, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const CHECKOUT = process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? "";
const PROJECT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";
const DAEMON_PID = Number(process.env.GWT_PLAYWRIGHT_DAEMON_PID);
function assertOwnedDaemon(cli: (operation: string) => any): void {
  expect(DAEMON_PID).toBeGreaterThan(0);
  expect(cli("daemon.status")).toMatch(new RegExp(`^running pid=${DAEMON_PID}\\b.*probe=ok`));
}
function recoveryElapsed(fixture: any): number {
  expect(fixture.recovery.route).toBe("forced-owned-pid");
  expect(fixture.recovery.pid).toBe(DAEMON_PID);
  expect(fixture.recovery.previous_pid).not.toBe(DAEMON_PID);
  expect(fixture.recovery.epoch).toBeGreaterThan(fixture.recovery.previous_epoch);
  const elapsed = Date.now() - Date.parse(fixture.recovery.started_at);
  expect(elapsed).toBeGreaterThanOrEqual(0);
  expect(elapsed, "replacement daemon recovers control and launches within five minutes").toBeLessThan(300_000);
  return elapsed;
}
test.use({ headless: false, viewport: { width: 1440, height: 1000 } });

test.describe("Issue #5694 monitor authority (isolated live backend)", () => {
  test.skip(!BASE || process.env.GWT_PLAYWRIGHT_ISSUE_5694 !== "1", "requires issue-5694-browser-check.ps1 isolation");
  test.setTimeout(150_000);

  test("a registered PM enables monitoring and Autonomous, then disables both on the same lane", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      expect(await realpath(CHECK_HOME)).not.toBe(await realpath(homedir()));
      const fixture = JSON.parse(await readFile(join(CHECK_HOME, "issue-5694-isolated.json"), "utf8"));
      expect(await realpath(fixture.project_root)).toBe(await realpath(PROJECT));
      expect(fixture.repo_hash).toMatch(/^[a-zA-Z0-9_-]+$/);
      expect(fixture.allowed_label).toMatch(/^browser-check-5694-/);
      expect(new URL(BASE).origin).toBe(new URL((await readFile(join(CHECK_HOME, "url.txt"), "utf8")).trim()).origin);
      const preferences = join(CHECK_HOME, ".gwt/projects", fixture.repo_hash, "project-state");
      const pmPath = join(preferences, "pm.json");
      const monitorPath = join(preferences, "issue-monitor.json");
      const readMonitor = async () => JSON.parse(await readFile(monitorPath, "utf8"));
      expect(JSON.parse(await readFile(pmPath, "utf8")).settings.auto_start).toBe(false);
      expect(await readMonitor()).toMatchObject({ enabled: false, autonomous_mode: false,
        allowed_labels: [fixture.allowed_label], update_drain: { version: "999.0.0", automatic: false } });

      const session = `issue-5694-pm-${info.project.name}`;
      // The CLI receives only this disposable PM's authority, never the owner
      // session, verification lease, or daemon context of the invoking agent.
      const cliEnvironment = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^GWT_/i.test(key)));
      Object.assign(cliEnvironment, { HOME: CHECK_HOME, USERPROFILE: CHECK_HOME, GWT_SESSION_ID: session });
      const cli = (operation: string, params: Record<string, unknown> = {}) => {
        const result = spawnSync(join(CHECKOUT, "target/debug", process.platform === "win32" ? "gwtd.exe" : "gwtd"), [], {
          cwd: PROJECT, env: cliEnvironment, encoding: "utf8", timeout: 35_000,
          input: JSON.stringify({ schema_version: 1, operation, params: operation === "daemon.status" ? {} : { ...params, project_root: PROJECT } }),
        });
        expect(result.status, `${operation}: ${result.error ?? result.stderr}\n${result.stdout}`).toBe(0);
        const envelope = JSON.parse(result.stdout);
        expect(envelope.ok, `${operation}: ${envelope.output}`).toBe(true);
        return operation === "daemon.status" ? envelope.output : JSON.parse(envelope.output);
      };
      assertOwnedDaemon(cli);
      const registerPm = async () => {
        // Register after frontend startup, and immediately before each PM
        // command: startup may clean up a registration without a live pane.
        const pm = JSON.parse(await readFile(pmPath, "utf8"));
        pm.registration = { session_id: session, agent_id: "codex", worktree_path: PROJECT,
          created_at: new Date().toISOString(), consecutive_crashes: 0, next_not_before: null };
        await writeFile(pmPath, JSON.stringify(pm));
      };
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
      const migration = page.locator("#migration-modal.open");
      if (await migration.isVisible()) {
        await migration.getByRole("button", { name: "Migrate", exact: true }).click();
        await expect(migration).toHaveCount(0, { timeout: 60_000 });
      }
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "issues" } })));
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      const panel = settings.locator('[data-settings-panel="issues"]');
      await expect(panel).toBeVisible();
      // Independent Settings controls project the same confirmed Monitor state.
      const monitoring = panel.getByRole("checkbox", { name: "Monitoring", exact: true });
      const autonomous = panel.getByRole("checkbox", { name: "Autonomous", exact: true });
      await expect(monitoring).not.toBeChecked();
      await expect(autonomous).not.toBeChecked();

      try {
        await registerPm();
        expect(cli("issue.monitor.config.set", { update_drain: true }).update_drain.version).toBe("999.0.0");
        await registerPm();
        expect(cli("issue.monitor.config.set", { enabled: true, autonomous_mode: true, update_drain: true }))
          .toMatchObject({ enabled: true, autonomous_mode: true });
        const controlRecoveryMs = recoveryElapsed(fixture);
        await writeFile(join(CHECK_HOME, `control-recovery-${theme}.json`), JSON.stringify({ ...fixture.recovery, elapsed_ms: controlRecoveryMs }));
        await expect.poll(readMonitor).toMatchObject({ enabled: true, autonomous_mode: true,
          allowed_labels: [fixture.allowed_label], update_drain: { version: "999.0.0" } });
        expect(cli("issue.monitor.status")).toMatchObject({ enabled: true, autonomous_mode: true, active_launches: [] });
        await sendLiveGwtEvent(page, { kind: "list_issue_monitor" });
        await expect(monitoring).toBeChecked();
        await expect(autonomous).toBeChecked();
        await info.attach(`monitor-enabled-${theme}`, { body: await settings.screenshot({ path: join(CHECK_HOME, `monitor-enabled-${theme}.png`) }), contentType: "image/png" });

        await registerPm();
        expect(cli("issue.monitor.config.set", { enabled: false, autonomous_mode: false }))
          .toMatchObject({ enabled: false, autonomous_mode: false });
        await expect.poll(readMonitor).toMatchObject({ enabled: false, autonomous_mode: false,
          allowed_labels: [fixture.allowed_label], update_drain: { version: "999.0.0" } });
        expect(cli("issue.monitor.status")).toMatchObject({ enabled: false, autonomous_mode: false, active_launches: [] });
        await sendLiveGwtEvent(page, { kind: "list_issue_monitor" });
        await expect(monitoring).not.toBeChecked();
        await expect(autonomous).not.toBeChecked();
        await expect(page.locator('.workspace-window[data-preset="codex"], .workspace-window[data-preset="claude"]')).toHaveCount(0);
        expect(JSON.parse(await readFile(pmPath, "utf8")).settings.auto_start).toBe(false);
        await info.attach(`monitor-disabled-${theme}`, { body: await settings.screenshot({ path: join(CHECK_HOME, `monitor-disabled-${theme}.png`) }), contentType: "image/png" });
        expect(errors, "console and page errors during live monitor control").toEqual([]);
        assertOwnedDaemon(cli);
      } finally {
        // The parent script owns process cleanup. Keep this HOME disabled even
        // after a failed assertion; retain the admission drain until shutdown.
        cli("issue.monitor.config.set", { enabled: false, autonomous_mode: false });
        const pm = JSON.parse(await readFile(pmPath, "utf8"));
        if (pm.registration?.session_id === session) {
          pm.registration = null;
          await writeFile(pmPath, JSON.stringify(pm));
        }
      }
    });
  });

  test("twelve confirmed deliveries cross daemon IPC, create live PTYs and receive durable ACKs", async ({ page }, info) => {
    test.setTimeout(240_000);
    await withLiveGwtBackendLock(BASE, info, async () => {
      const fixture = JSON.parse(await readFile(join(CHECK_HOME, "issue-5694-isolated.json"), "utf8"));
      expect(await realpath(fixture.project_root)).toBe(await realpath(PROJECT));
      expect(await realpath(CHECK_HOME)).not.toBe(await realpath(homedir()));
      const state = join(CHECK_HOME, ".gwt/projects", fixture.repo_hash, "project-state");
      const prefsPath = join(state, "issue-monitor.json");
      const readPrefs = async () => JSON.parse(await readFile(prefsPath, "utf8"));
      const env = Object.fromEntries(Object.entries(process.env).filter(([key]) =>
        !/^GWT_/i.test(key) && !["GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"].includes(key)));
      Object.assign(env, { HOME: CHECK_HOME, USERPROFILE: CHECK_HOME,
        PATH: `${join(CHECK_HOME, "bin")};${process.env.PATH ?? ""}`, GWT_TEST_GH: join(CHECK_HOME, "bin/gh.cmd") });
      const cli = (operation: string, params: Record<string, unknown> = {}) => {
        const result = spawnSync(join(CHECKOUT, "target/debug/gwtd.exe"), [], {
          cwd: PROJECT, env, encoding: "utf8", timeout: 35_000,
          input: JSON.stringify({ schema_version: 1, operation, params: operation === "daemon.status" ? {} : { ...params, project_root: PROJECT } }),
        });
        expect(result.status, `${operation}: ${result.stderr}\n${result.stdout}`).toBe(0);
        const envelope = JSON.parse(result.stdout);
        expect(envelope.ok, `${operation}: ${envelope.output}`).toBe(true);
        return operation === "daemon.status" ? envelope.output : JSON.parse(envelope.output);
      };
      assertOwnedDaemon(cli);
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      const numbers = Array.from({ length: 12 }, (_, index) => (theme === "dark" ? 569400 : 569500) + index);
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
      const migration = page.locator("#migration-modal.open");
      if (await migration.isVisible()) {
        await migration.getByRole("button", { name: "Migrate", exact: true }).click();
        await expect(migration).toHaveCount(0, { timeout: 60_000 });
      }
      const beforeReceipts = new Set(await readdir(join(CHECK_HOME, "workload")));
      const now = new Date().toISOString();
      // Resize the canonical roster before seeding the durable delivery fixture.
      // Raw numerical writes would leave an already-materialized roster inconsistent.
      cli("issue.monitor.config.set", { max_active: 12 });
      // Project roster size no longer changes the independent machine safety
      // policy. Seed this disposable experiment's explicit Manual 12 limit;
      // the real physical-spawn gate remains active for all twelve children.
      const machinePath = join(CHECK_HOME, ".gwt/machine-state/agent-settings.json");
      const machinePolicy = { ...JSON.parse(await readFile(machinePath, "utf8")),
        mode: "manual", manual_limit: 12, last_change: null };
      await writeFile(machinePath + ".twelve-pty.tmp", JSON.stringify(machinePolicy));
      await rename(machinePath + ".twelve-pty.tmp", machinePath);
      const prefs = await readPrefs();
      expect(prefs.max_active_agents).toBe(12);
      if (prefs.agent_roster) expect(prefs.agent_roster.members.filter((member: any) => member.enabled)).toHaveLength(12);
      expect(prefs.enabled).toBe(false);
      expect(prefs.autonomous_mode).toBe(false);
      // This fixture starts after remote claim acquisition. It exercises the
      // real durable outbox, IPC, preparation, process spawn and ACK transport;
      // remote claim/requeue policy stays covered by the existing Rust tests.
      Object.assign(prefs, {
        launch_auto: false, terminal_queue_auto_refill: false,
        launch_profile: { agent_id: "monitor-delivery-fixture", runtime_target: "Host", skip_permissions: true },
        launch_profiles: [{ agent_id: "monitor-delivery-fixture", runtime_target: "Host", skip_permissions: true }],
        launching_issues: numbers.map(issue_number => ({ issue_number, claimed_at: now })),
        pending_launch_deliveries: numbers.map(issue_number => ({ issue_number,
          delivery_id: `launch:fixture-${theme}-${issue_number}`, branch_name: `work/issue-${issue_number}`,
          linked_issue_kind: "issue", claim_id: `fixture-claim-${issue_number}`, claim_owner: "fixture/confirmed",
          launch_session_strategy: "fresh_required", created_at: now })),
      });
      const seedPath = join(state, "issue-5694-seed.tmp");
      await writeFile(seedPath, JSON.stringify(prefs));
      await rename(seedPath, prefsPath);
      const windows: Array<{ id: string; session_id: string }> = [];
      let primaryFailed = false;
      try {
        // A non-enabling control rebases the seeded outbox into the resident
        // daemon without starting scans against GitHub. Delivery replay and
        // materialization do not require enabled=true.
        cli("issue.monitor.config.set", { max_active: 12, update_drain: false });
        await expect.poll(async () => (await readPrefs()).pending_launch_deliveries?.length ?? 0,
          { timeout: 120_000, message: "twelve actual launches receive durable ACKs" }).toBe(0);
        const acknowledgedAt = Date.now();
        const launchRecoveryMs = recoveryElapsed(fixture);
        const launched = (await readPrefs()).launched_issues ?? [];
        expect(launched.filter((entry: any) => numbers.includes(entry.issue_number))).toHaveLength(12);
        const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
        await sendLiveGwtEvent(page, { kind: "list_windows" });
        await page.waitForFunction(before => ((window as any).__gwtPlaywrightMessages ?? [])
          .some(({ sequence, payload }: any) => sequence > before && payload.kind === "window_list"), cursor);
        const observed = await page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
          .filter(({ payload }: any) => payload.kind === "window_list").at(-1)?.payload.windows);
        windows.push(...observed
          .filter((pane: any) => pane.agent_id === "monitor-delivery-fixture" && ["running", "idle"].includes(pane.status))
          .map((pane: any) => ({ id: pane.id, session_id: pane.session_id })));
        expect(windows).toHaveLength(12);
        expect(new Set(windows.map(window => window.session_id)).size).toBe(12);
        const confirmations = (await readPrefs()).launch_confirmations ?? {};
        const exactAcks = numbers.map(number => {
          const launch = launched.find((entry: any) => entry.issue_number === number);
          const confirmation = confirmations[String(number)];
          expect(confirmation).toMatchObject({ window_id: launch.window_id,
            claim_id: `fixture-claim-${number}`, delivery_id: `launch:fixture-${theme}-${number}` });
          expect(windows.some(window => launch.window_id === window.id || launch.window_id.endsWith(`::${window.id}`))).toBe(true);
          return { number, ...confirmation };
        });
        expect(new Set(exactAcks.map(ack => ack.window_id)).size).toBe(12);
        const durable = JSON.parse(await readFile(join(state, "../workspace.json"), "utf8"));
        expect(durable.windows.filter((pane: any) => pane.agent_id === "monitor-delivery-fixture")
          .map((pane: any) => `${pane.id}/${pane.session_id}`).sort())
          .toEqual(windows.map(window => `${window.id.split("::").at(-1)}/${window.session_id}`).sort());
        await expect(page.locator('.op-rail__surface[data-surface="works"]')).toBeVisible({ timeout: 10_000 });
        await page.locator('.op-rail__surface[data-surface="works"]').click();
        if (await page.locator(".works-surface:not(.works-surface--preview) .works-full:not([hidden])").isVisible())
          await page.locator('.works-surface:not(.works-surface--preview) [data-action="back"]').click();
        for (const window of windows) {
          const tile = page.locator('.work-row[data-window-id="' + window.id + '"]');
          await tile.scrollIntoViewIfNeeded({ timeout: 10_000 });
          const preview = tile.locator(".terminal-text-preview");
          await expect(preview).toBeVisible();
          expect((await preview.innerText()).trim().split("\n").length).toBeLessThanOrEqual(3);
          const receiptPath = join(CHECK_HOME, "workload", `${window.session_id}.json`);
          expect(beforeReceipts.has(`${window.session_id}.json`)).toBe(false);
          await expect(async () => {
            expect(JSON.parse(await readFile(receiptPath, "utf8")).session_id).toBe(window.session_id);
          }).toPass({ timeout: 40_000 });
          const receipt = JSON.parse(await readFile(receiptPath, "utf8"));
          expect(receipt.cwd.toLowerCase()).toContain(CHECK_HOME.toLowerCase());
          expect(receipt.cli_binary.toLowerCase()).toBe(join(CHECKOUT, "target/debug/gwtd.exe").toLowerCase());
          expect(() => process.kill(receipt.pid, 0)).not.toThrow();
          await sendLiveGwtEvent(page, { kind: "terminal_input", id: window.id,
            data: JSON.stringify({ kind: "probe", id: window.id }) + "\r" });
          await expect(async () => {
            expect(JSON.parse(await readFile(join(CHECK_HOME, "workload", `${window.session_id}.probe.json`), "utf8")).ok).toBe(true);
          }).toPass({ timeout: 40_000 });
          await expect.poll(() => page.evaluate(id => (window as any).__gwtTerminalTestApi.bufferText(id), window.id))
            .toContain(`GWT_5694_PTY_${window.session_id}`);
          await expect(preview).toContainText(`GWT_5694_PTY_${window.session_id}`);
        }
        expect((await readPrefs()).failed_issues ?? []).toEqual([]);
        assertOwnedDaemon(cli);
        await page.screenshot({ path: join(CHECK_HOME, `twelve-pty-${theme}.png`), fullPage: true });
        await info.attach(`twelve-pty-${theme}`, { path: join(CHECK_HOME, `twelve-pty-${theme}.png`), contentType: "image/png" });
        await info.attach(`twelve-pty-ack-${theme}`, { body: JSON.stringify({ numbers, windows, exactAcks, machine_manual_limit: machinePolicy.manual_limit,
          daemon_pid: DAEMON_PID, recovery_elapsed_ms: launchRecoveryMs,
          ack_elapsed_ms: acknowledgedAt - Date.parse(now), pending: (await readPrefs()).pending_launch_deliveries ?? [],
          scope: "confirmed-delivery through actual PTY and durable ACK; remote claims covered separately" }), contentType: "application/json" });
        expect(errors).toEqual([]);
      } catch (error) {
        primaryFailed = true;
        const diagnosticPath = join(CHECK_HOME, `before-cleanup-${theme}.json`);
        await writeFile(diagnosticPath, JSON.stringify({ original_error: String(error), prefs: await readPrefs(),
          workspace: JSON.parse(await readFile(join(state, "../workspace.json"), "utf8")),
          received: await page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
            .filter(({ payload }: any) => /^(issue_monitor_|window_state)/.test(payload.kind))).catch(() => []) }));
        await info.attach(`before-cleanup-${theme}`, { path: diagnosticPath, contentType: "application/json" });
        throw error;
      } finally {
        try {
          // Each theme owns a fresh process tree. The parent script stops and
          // verifies those exact PIDs, preserving the complete durable ACK set.
          cli("issue.monitor.config.set", { enabled: false, autonomous_mode: false, update_drain: { reason: "manual", version: "999.0.0", automatic: false } });
        } catch (cleanupError) {
          const path = join(CHECK_HOME, `cleanup-error-${theme}.txt`);
          await writeFile(path, String(cleanupError));
          await info.attach(`cleanup-error-${theme}`, { path, contentType: "text/plain" });
          if (!primaryFailed) throw cleanupError;
        }
      }
    });
  });
});
