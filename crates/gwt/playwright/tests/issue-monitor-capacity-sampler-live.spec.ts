/* Issue #5500: observe the real checkout sampler without changing Monitor
 * controls. Rust tests cover exact fanout suppression and saved authority. */
import { readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const PROJECT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";

test("Issue #5500: live sampler renews the machine snapshot and renders capacity", async ({ page }, testInfo) => {
  test.skip(!BASE, "requires the isolated checkout harness");
  test.setTimeout(900_000);
  await withLiveGwtBackendLock(BASE, testInfo, async () => {
    expect(CHECK_HOME).not.toBe("");
    expect(PROJECT).not.toBe("");
    const home = await realpath(CHECK_HOME);
    expect(home).not.toBe(await realpath(homedir()));
    const marker = JSON.parse(await readFile(join(home, "issue-5500-isolated.json"), "utf8"));
    expect(await realpath(marker.project)).toBe(await realpath(PROJECT));
    expect(new URL(marker.url).origin).toBe(new URL(BASE).origin);
    const state = join(home, ".gwt/projects", marker.repo_hash, "project-state");
    expect(JSON.parse(await readFile(join(state, "pm.json"), "utf8")).settings.auto_start).toBe(false);
    expect(JSON.parse(await readFile(join(state, "issue-monitor.json"), "utf8")).enabled).toBe(false);
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await page.bringToFront();
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .some(({ payload }: any) => payload.kind === "workspace_state"));
    await page.locator(".op-rail").getByRole("button", { name: "Issues", exact: true }).click();
    const surface = page.locator('.split-pane[data-active="true"][data-surface="issues"] .workspace-window[data-preset="issue"]');
    await expect(surface).toHaveCount(1);
    await expect(surface).toBeVisible();
    const latest = () => page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .filter(({ payload }: any) => payload.kind === "issue_monitor_status").at(-1)?.payload.status);
    // Large target trees advance over bounded sampler ticks, independently of
    // the stopped Monitor. Do not wait for unchanged WebSocket retransmission.
    await expect.poll(latest, { timeout: 600_000 }).toMatchObject({
      enabled: false, max_active_agents_override: null,
      agent_capacity: { measurement_complete: true },
    });
    await expect(surface.locator(".knowledge-monitor-capacity-badge")).toBeVisible();
    await surface.locator(".knowledge-monitor-capacity-badge").click();
    const notice = page.locator('.notification-center__item[data-error-key="issue-monitor:capacity"]');
    await expect(notice).toBeVisible();
    const details = notice.locator(".notification-center__details");
    await details.locator("summary").click();
    await expect(details).toContainText("Machine live:");
    await expect(details).toContainText("GUI CPU reserved:");
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
    await testInfo.attach(`capacity-sampler-${testInfo.project.name}`, {
      body: await page.screenshot({ animations: "disabled" }), contentType: "image/png",
    });
    const snapshotPath = join(home, ".gwt/machine-state/agent-capacity.json");
    const snapshot = () => readFile(snapshotPath, "utf8").then(JSON.parse);
    const first = await snapshot();
    await expect.poll(async () => (await snapshot()).observed_at, { timeout: 90_000 })
      .toBeGreaterThan(first.observed_at);
    const renewed = await snapshot();
    expect(renewed.expires_at).toBeGreaterThan(renewed.observed_at);
    expect(renewed.projects).toHaveProperty(PROJECT);
    expect((await latest()).max_active_agents_override).toBeNull();
    expect(errors, "zero console/page errors").toEqual([]);
  });
});
