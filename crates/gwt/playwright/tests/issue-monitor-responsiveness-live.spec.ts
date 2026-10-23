/**
 * Issue #4963 AC-1: a disabled Monitor's ListIssueMonitor projection must
 * remain responsive with a cold/stale cache, including Launch Wizard cancel.
 * The isolated server's fake gh gates only state=all&page=1. Generation files
 * in gate_root rendezvous through armed -> started -> released -> completed;
 * CacheOnly projections correctly never enter that remote gate.
 */
import { randomUUID } from "node:crypto";
import { mkdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import {
  clearLiveLaunchWizard,
  gotoLiveGwt,
  openLiveGwtProject,
  openLiveLaunchWizardForBranch,
  sendLiveGwtEvent,
  withLiveGwtBackendLock,
  type LiveLaunchWizardFixture,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const RESPONSE_BUDGET_MS = 5_000;

type MonitorFixture = {
  project: string;
  repo_hash: string;
  cache_root: string;
  gate_root: string;
};

test.describe("Issue Monitor cache responsiveness (live backend)", () => {
  test.skip(!BASE, "requires an isolated checkout gwt instance");
  test.setTimeout(120_000);

  for (const cacheState of ["cold", "stale"] as const) {
    test(`${cacheState} cache keeps the list and wizard open/cancel responsive`, async ({ page }, testInfo) => {
      await withLiveGwtBackendLock(BASE, testInfo, async () => {
        const fixture = await requireFixture();
        const branch = process.env.GWT_PLAYWRIGHT_BRANCH_NAME ?? "";
        expect(branch, "requires the current existing work branch").not.toBe("");
        const errors: string[] = [];
        page.on("pageerror", error => errors.push(error.message));
        page.on("console", message => {
          if (message.type() === "error") errors.push(message.text());
        });
        let wizardFixture: LiveLaunchWizardFixture | undefined;
        const generation = randomUUID();
        try {
          await gotoLiveGwt(page, BASE, { enableTestBridge: true });
          await openLiveGwtProject(page, fixture.project);
          await clearLiveLaunchWizard(page);
          // Materialize/reuse the Work surface before arming the remote gate.
          // This opens only the wizard; it never submits an agent launch.
          wizardFixture = await openLiveLaunchWizardForBranch(page, branch);
          const wizard = page.locator("#wizard-modal");
          await expect(wizard).toBeVisible({ timeout: 30_000 });
          await clearLiveLaunchWizard(page);

          await resetCache(fixture.cache_root, cacheState);
          await mkdir(fixture.gate_root, { recursive: true });
          await writeFile(join(fixture.gate_root, "armed"), generation);

          const listCursor = await messageCursor(page);
          await sendLiveGwtEvent(page, { kind: "list_issue_monitor" });
          const status = await page.waitForFunction(cursor => {
            const frame = ((window as any).__gwtPlaywrightMessages ?? []).find((entry: any) =>
              entry.sequence > cursor && entry.payload?.kind === "issue_monitor_status");
            return frame?.payload.status;
          }, listCursor, { timeout: RESPONSE_BUDGET_MS }).then(handle => handle.jsonValue());
          expect(status.enabled).toBe(false);
          expect(status.active_count).toBe(0);
          await assertRemoteStillGated(fixture.gate_root, generation);

          const openCursor = await messageCursor(page);
          await sendLiveGwtEvent(page, {
            kind: "open_launch_wizard",
            id: wizardFixture.windowId,
            branch_name: branch,
          });
          await waitForWizardReply(page, openCursor, true);
          await expect(wizard).toBeVisible({ timeout: RESPONSE_BUDGET_MS });
          await expect(page.locator("html")).toHaveAttribute(
            "data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark",
          );
          await assertRemoteStillGated(fixture.gate_root, generation);
          const screenshotPath = testInfo.outputPath("wizard.png");
          await wizard.screenshot({ path: screenshotPath });
          await testInfo.attach(`wizard-${cacheState}-${testInfo.project.name}`, {
            path: screenshotPath, contentType: "image/png",
          });

          const cancelCursor = await messageCursor(page);
          await page.locator("#wizard-cancel-button").click({ timeout: RESPONSE_BUDGET_MS });
          await waitForWizardReply(page, cancelCursor, false);
          await expect(wizard).toBeHidden({ timeout: RESPONSE_BUDGET_MS });
          await assertRemoteStillGated(fixture.gate_root, generation);
        } finally {
          // Unblock any old-implementation child before real backend cleanup.
          await mkdir(fixture.gate_root, { recursive: true });
          await writeFile(join(fixture.gate_root, "released"), generation);
          try {
            await clearLiveLaunchWizard(page);
          } finally {
            await wizardFixture?.cleanup();
          }
        }
        expect(errors).toEqual([]);
      });
    });
  }
});

async function requireFixture(): Promise<MonitorFixture> {
  const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
  const project = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";
  expect(home, "requires the browser-check isolated HOME").not.toBe("");
  expect(project, "requires the checkout project root").not.toBe("");
  const checkHome = await realpath(home);
  expect(checkHome).not.toBe(await realpath(homedir()));
  const fixture: MonitorFixture = JSON.parse(
    await readFile(join(checkHome, "issue-4963-isolated.json"), "utf8"),
  );
  expect(await realpath(fixture.project)).toBe(await realpath(project));
  expect(resolve(fixture.cache_root)).toBe(join(checkHome, ".gwt/cache/issues", fixture.repo_hash));
  expect(resolve(fixture.gate_root)).toBe(join(checkHome, "issue-4963-gh-gate"));
  const servedUrl = (await readFile(join(checkHome, "url"), "utf8")).trim();
  expect(new URL(BASE).origin).toBe(new URL(servedUrl).origin);
  const stateRoot = join(checkHome, ".gwt/projects", fixture.repo_hash, "project-state");
  const monitor = JSON.parse(await readFile(join(stateRoot, "issue-monitor.json"), "utf8"));
  const pm = JSON.parse(await readFile(join(stateRoot, "pm.json"), "utf8"));
  expect(monitor.enabled).toBe(false);
  expect(pm.settings.auto_start).toBe(false);
  return fixture;
}

async function resetCache(cacheRoot: string, state: "cold" | "stale"): Promise<void> {
  await rm(cacheRoot, { recursive: true, force: true });
  if (state === "cold") return;
  const issueRoot = join(cacheRoot, "4963001");
  await mkdir(issueRoot, { recursive: true });
  await writeFile(join(issueRoot, "meta.json"), JSON.stringify({
    number: 4963001, title: "Stale cache fixture", labels: [], state: "closed",
    updated_at: "2000-01-01T00:00:00Z", comment_ids: [],
  }));
  await writeFile(join(issueRoot, "body.md"), "Stale cache fixture");
  await writeFile(join(cacheRoot, "refresh-meta.json"), JSON.stringify({
    last_full_refresh: "2000-01-01T00:00:00Z", ttl_minutes: 15,
  }));
}

async function messageCursor(page: Page): Promise<number> {
  return page.evaluate(() => Number((window as any).__gwtPlaywrightMessageSequence) || 0);
}

async function waitForWizardReply(page: Page, cursor: number, hasWizard: boolean): Promise<void> {
  await page.waitForFunction(({ cursor, hasWizard }) =>
    ((window as any).__gwtPlaywrightMessages ?? []).some((entry: any) =>
      entry.sequence > cursor && entry.payload?.kind === "launch_wizard_state"
      && (entry.payload.wizard !== null) === hasWizard),
  { cursor, hasWizard }, { timeout: RESPONSE_BUDGET_MS });
}

async function assertRemoteStillGated(gateRoot: string, generation: string): Promise<void> {
  const started = await readFile(join(gateRoot, "started"), "utf8").catch(() => "");
  if (started.trim() !== generation) return;
  const completed = await readFile(join(gateRoot, "completed"), "utf8").catch(() => "");
  expect(completed.trim(), "remote cache enumeration must stay gated during interactions")
    .not.toBe(generation);
}
