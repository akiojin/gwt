/* Issue #5242: real issue.close/reopen, daemon state and browser projections.
 * Run with crates/gwt/playwright/verify-issue-5242-closed.mjs. GitHub is a loopback recorder;
 * the checkout binaries use a disposable project/HOME with agents disabled.
 */
import { spawn } from "node:child_process";
import { readFile, realpath, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";

async function isolatedFixture() {
  const home = await realpath(CHECK_HOME);
  expect(home).not.toBe(await realpath(homedir()));
  const marker = JSON.parse(await readFile(join(home, "issue-5242-isolated.json"), "utf8"));
  expect(new URL(marker.endpoint).hostname).toBe("127.0.0.1");
  expect(new URL(BASE).origin).toBe(new URL((await readFile(join(home, "url"), "utf8")).trim()).origin);
  const prefsPath = join(home, ".gwt/projects", marker.repo_hash, "project-state/issue-monitor.json");
  expect(JSON.parse(await readFile(join(home, ".gwt/projects", marker.repo_hash, "project-state/pm.json"), "utf8")).settings.auto_start).toBe(false);
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GWT_") && !["GH_TOKEN", "GITHUB_TOKEN"].includes(key)));
  Object.assign(env, {
    HOME: home, USERPROFILE: home, GWT_TEST_GH: marker.fake_gh,
    GWT_OWNER_GITHUB_TEST_MODE: "loopback-v1", GWT_OWNER_GITHUB_REST_BASE: marker.endpoint,
    GWT_OWNER_GITHUB_GRAPHQL_URL: `${marker.endpoint}/graphql`, GWT_OWNER_GITHUB_TOKEN: "fixture-only",
  });
  const cli = async (operation: string, params: Record<string, unknown> = {}) => {
    const child = spawn(marker.gwtd, [], { cwd: marker.project, env, windowsHide: true });
    let stdout = "", stderr = "";
    child.stdout.on("data", data => { stdout += data; });
    child.stderr.on("data", data => { stderr += data; });
    const code = await new Promise<number | null>((resolve, reject) => {
      const timer = setTimeout(() => { child.kill(); reject(new Error(`${operation} timed out`)); }, 45_000);
      child.once("error", error => { clearTimeout(timer); reject(error); });
      child.once("close", code => { clearTimeout(timer); resolve(code); });
      child.stdin.end(JSON.stringify({ schema_version: 1, operation, params }));
    });
    expect(code, `${operation}: ${stdout}\n${stderr}`).toBe(0);
    const envelope = JSON.parse(stdout);
    expect(envelope.ok, `${operation}: ${stdout}`).toBe(true);
    return JSON.parse(envelope.output);
  };
  return { marker, cli, prefsPath };
}

async function status(page: Page): Promise<any> {
  const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
  await sendLiveGwtEvent(page, { kind: "list_issue_monitor" });
  await page.waitForFunction(before => ((window as any).__gwtPlaywrightMessages ?? []).some((entry: any) =>
    entry.sequence > before && entry.payload.kind === "issue_monitor_status"), cursor, { timeout: 75_000 });
  return page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? []).filter((entry: any) =>
    entry.payload.kind === "issue_monitor_status").at(-1).payload.status);
}

async function inboxNumbers(page: Page): Promise<number[]> {
  return page.evaluate(() => (((window as any).__gwtPlaywrightMessages ?? []).filter((entry: any) =>
    entry.payload.kind === "issue_monitor_inbox").at(-1)?.payload.items ?? []).map((item: any) => item.issue.number));
}

test.describe("Closed Issue retirement (live backend)", () => {
  test.skip(!BASE || !CHECK_HOME, "requires the isolated Issue #5242 harness");
  test.setTimeout(240_000);
  test.use({ viewport: { width: 1440, height: 1000 } });

  test("close releases live membership and reopen permits explicit queue admission", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      const { marker, cli, prefsPath } = await isolatedFixture();
      const number = testInfo.project.name.endsWith("light") ? 524202 : 524201;
      const reason = testInfo.project.name.endsWith("light") ? "not_planned" : "completed";
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator("#close-project-button")).toBeVisible();
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? []).some((entry: any) =>
        entry.payload.kind === "workspace_state"));
      const windows = page.locator('.workspace-window[data-preset="issue"]');
      if (!await windows.count()) await sendLiveGwtEvent(page, { kind: "create_window", preset: "issue", bounds: { x: 50, y: 40, width: 1150, height: 850 } });
      await expect(windows.first()).toBeVisible();
      const windowId = await windows.first().getAttribute("data-id");
      await sendLiveGwtEvent(page, { kind: "update_window_geometry", id: windowId,
        geometry: { x: 50, y: 40, width: 1150, height: 850 }, cols: 100, rows: 28 });
      await expect.poll(() => windows.first().evaluate(element => element.getBoundingClientRect().height)).toBeGreaterThan(800);
      const before = await cli("issue.monitor.status");
      expect(before.enabled).toBe(false);
      expect(before.autonomous_mode).toBe(false);
      expect(before.active_launches).toContain(number);
      expect(before.gui_status.terminal_queue.map((entry: any) => entry.number)).toContain(number);
      await expect.poll(() => inboxNumbers(page), { timeout: 90_000 }).toContain(number);

      expect(await cli("issue.close", { number, reason, comment: "Issue #5242 isolated close fixture" })).toMatchObject({ status: "closed", changed: true });
      const closed = await cli("issue.monitor.status");
      expect(closed.active_launches).not.toContain(number);
      expect(closed.queue).not.toContain(number);
      expect(closed.inbox.map((entry: any) => entry.issue_number)).not.toContain(number);
      expect(closed.gui_status.terminal_queue.map((entry: any) => entry.number)).not.toContain(number);
      const persisted = JSON.parse(await readFile(prefsPath, "utf8"));
      expect((persisted.launched_issues ?? []).map((entry: any) => entry.issue_number)).not.toContain(number);
      expect(Object.values(persisted.terminal_queues ?? {}).flatMap((queue: any) => queue.entries.map((entry: any) => entry.number))).not.toContain(number);
      const remote = await fetch(`${marker.endpoint}/fixture/issues/${number}`).then(response => response.json()) as any;
      expect(remote.state).toBe("closed");
      expect(remote.state_reason).toBe(reason);
      expect(remote.labels.map((label: any) => label.name).sort()).toEqual(["auto-improve", "bug"]);
      await expect.poll(async () => (await status(page)).active_count, { timeout: 90_000 }).toBe(closed.active_launches.length);
      await expect.poll(async () => (await status(page)).terminal_queue.map((entry: any) => entry.number), { timeout: 90_000 }).not.toContain(number);
      await expect.poll(() => inboxNumbers(page), { timeout: 90_000 }).not.toContain(number);
      await expect(windows.first().locator('[data-metric="active"]')).toContainText(`Active ${closed.active_launches.length}/`);
      await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
      // Restored presentation presets can arrive after the initial canvas.
      // Keep the isolated fixture's capture unobscured by another Issue view.
      for (const id of await windows.evaluateAll(elements => elements.map(element => element.getAttribute("data-id")))) {
        if (id !== windowId) await sendLiveGwtEvent(page, { kind: "close_window", id });
      }
      await expect(windows).toHaveCount(1);
      const screenshot = await windows.first().screenshot();
      await writeFile(join(CHECK_HOME, `closed-retirement-${testInfo.project.name}.png`), screenshot);
      await testInfo.attach(`closed-retirement-${testInfo.project.name}`, { body: screenshot, contentType: "image/png" });

      expect(await cli("issue.reopen", { number })).toMatchObject({ status: "open", changed: true });
      expect((await cli("issue.monitor.status")).gui_status.terminal_queue.map((entry: any) => entry.number)).not.toContain(number);
      expect((await cli("issue.monitor.queue.push", { numbers: [number] })).accepted).toContain(number);
      // Monitor is disabled: queue.push writes durable admission without a scan
      // notification. Its source of truth is this entry and the GitHub label;
      // the GUI admission projection refreshes at the next scheduled scan.
      const pushed = JSON.parse(await readFile(prefsPath, "utf8"));
      expect(Object.values(pushed.terminal_queues).flatMap((queue: any) => queue.entries.map((entry: any) => entry.number))).toContain(number);
      const admitted = await cli("issue.monitor.status");
      expect(admitted.active_launches).not.toContain(number);
      const reopened = await fetch(`${marker.endpoint}/fixture/issues/${number}`).then(response => response.json()) as any;
      expect(reopened.state).toBe("open");
      expect(reopened.labels.map((label: any) => label.name)).toContain("gwt-queued");
      expect(errors).toEqual([]);
    });
  });
});
