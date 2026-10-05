/* Issue #4158 — saved allowed labels and next-scan admission through the real
 * checkout backend, in both Operator themes. Requires a browser-check fresh
 * HOME with PM/Monitor disabled and a PATH gh fixture serving matching,
 * nonmatching and unlabelled open Issues. GWT_PLAYWRIGHT_BASE_URL / CHECK_HOME /
 * PROJECT_ROOT identify the fresh launch. CHECK_HOME/url contains its URL;
 * CHECK_HOME/issue-4158-isolated.json records LabelsFixture below.
 */
import { spawnSync } from "node:child_process";
import { readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const PROJECT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";

type LabelsFixture = {
  project: string;
  repo_hash: string;
  allowed_label: string;
  expected_excluded_issues: number[];
  expected_admitted_issues: number[];
};

async function isolatedFixture() {
  expect(CHECK_HOME, "requires a browser-check fresh HOME").not.toBe("");
  expect(PROJECT, "requires the launch checkout").not.toBe("");
  const home = await realpath(CHECK_HOME);
  const project = await realpath(PROJECT);
  expect(home).not.toBe(await realpath(homedir()));
  const fixture: LabelsFixture = JSON.parse(await readFile(join(home, "issue-4158-isolated.json"), "utf8"));
  expect(await realpath(fixture.project)).toBe(project);
  expect(fixture.repo_hash).toMatch(/^[a-zA-Z0-9_-]+$/);
  expect(fixture.allowed_label.trim()).not.toBe("");
  expect(fixture.expected_excluded_issues.length, "seed nonmatching/unlabelled open Issues").toBeGreaterThan(0);
  expect(fixture.expected_admitted_issues.length, "seed an Issue matching the allowed label").toBeGreaterThan(0);
  const preferences = join(home, ".gwt/projects", fixture.repo_hash, "project-state");
  const pm = JSON.parse(await readFile(join(preferences, "pm.json"), "utf8"));
  expect(pm.settings.auto_start).toBe(false);
  const servedUrl = (await readFile(join(home, "url"), "utf8")).trim();
  expect(new URL(BASE).origin).toBe(new URL(servedUrl).origin);
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GWT_")));
  env.HOME = home;
  env.USERPROFILE = home;
  const cli = (operation: string, params: Record<string, unknown> = {}) => {
    const result = spawnSync(join(project, "target/debug/gwtd"), [], {
      cwd: project, env, encoding: "utf8", timeout: 30_000,
      input: JSON.stringify({ schema_version: 1, operation, params: { ...params, project_root: project } }),
    });
    expect(result.status, `${operation}: ${result.stderr}`).toBe(0);
    const envelope = JSON.parse(result.stdout);
    expect(envelope.ok, `${operation}: ${envelope.error ?? "refused"}`).toBe(true);
    return JSON.parse(envelope.output);
  };
  return { fixture, preferences, cli };
}

test.describe("Issue Monitor allowed labels (live backend)", () => {
  test.skip(!BASE, "requires an isolated checkout gwt instance");
  test.setTimeout(180_000);
  test.use({ viewport: { width: 1440, height: 1000 } });

  test("saves any-of labels, displays scan exclusions and restores all-label admission", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      const { fixture, preferences, cli } = await isolatedFixture();
      const before = cli("issue.monitor.status");
      expect(before.enabled).toBe(false);
      expect(before.autonomous_mode).toBe(false);
      expect(before.active_launches).toEqual([]);
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator("#close-project-button")).toBeVisible();
      const windows = page.locator('.workspace-window[data-preset="issue"]');
      const created = !(await windows.count());
      if (created) await sendLiveGwtEvent(page, {
        kind: "create_window", preset: "issue", bounds: { x: 60, y: 50, width: 1120, height: 820 },
      });
      await expect(windows.first()).toBeVisible();
      const windowId = await windows.first().getAttribute("data-id");
      const surface = page.locator(`.workspace-window[data-id="${windowId}"]`);
      const labels = surface.locator(".knowledge-monitor-labels");
      try {
        await sendLiveGwtEvent(page, { kind: "set_issue_monitor_allowed_labels", allowed_labels: [] });
        await expectSavedLabels(page, []);
        await expect(labels.locator("summary")).toContainText("All labels");
        await labels.locator("summary").click();
        await expect(labels).toContainText("Empty list allows all labels");
        await expect(labels).toContainText("any listed label on this terminal");
        // Both clicks occur before any WebSocket status can be processed.
        // Commas belong to the second label; neither addition may be lost.
        await labels.evaluate((section, allowed) => {
          const input = section.querySelector<HTMLInputElement>('[aria-label="Allowed label"]')!;
          const add = section.querySelector<HTMLButtonElement>('[data-action="monitor-label-add"]')!;
          input.value = `  ${allowed}  `;
          add.click();
          input.value = "agent, ready";
          add.click();
        }, fixture.allowed_label);
        await expectSavedLabels(page, [fixture.allowed_label, "agent, ready"]);
        const persisted = JSON.parse(await readFile(join(preferences, "issue-monitor.json"), "utf8"));
        expect(persisted.allowed_labels).toEqual([fixture.allowed_label, "agent, ready"]);

        // Save requests the next scan. Read its actual admission projection;
        // a changed input or an old status frame is insufficient evidence.
        await expect.poll(() => latestStatus(page).then(status => [...(status?.label_excluded_issues ?? [])].sort((a, b) => a - b)),
          { timeout: 75_000, intervals: [1_000, 2_000, 5_000] }).toEqual([...fixture.expected_excluded_issues].sort((a, b) => a - b));
        const scanned = await latestStatus(page);
        expect(scanned.label_excluded_count).toBe(fixture.expected_excluded_issues.length);
        await expect(labels.locator('[data-metric="label-excluded"]')).toContainText(`(${fixture.expected_excluded_issues.length})`);
        for (const number of fixture.expected_excluded_issues) {
          await expect(labels.locator('[data-metric="label-excluded"]')).toContainText(`#${number}`);
        }
        const admission = cli("issue.monitor.status");
        expect(admission.allowed_labels).toEqual([fixture.allowed_label, "agent, ready"]);
        expect(admission.label_excluded_issues.slice().sort((a: number, b: number) => a - b))
          .toEqual(fixture.expected_excluded_issues.slice().sort((a, b) => a - b));
        const admitted = admission.inbox.map((issue: any) => issue.issue_number);
        for (const number of fixture.expected_admitted_issues) expect(admitted).toContain(number);
        for (const number of fixture.expected_excluded_issues) expect(admitted).not.toContain(number);

        await gotoLiveGwt(page, BASE, { enableTestBridge: true });
        await expectSavedLabels(page, [fixture.allowed_label, "agent, ready"]);
        if (!(await labels.evaluate(node => (node as HTMLDetailsElement).open))) await labels.locator("summary").click();
        await expect(labels.locator("[data-allowed-label]")).toHaveCount(2);
        const remove = labels.getByRole("button", { name: "Remove allowed label agent, ready", exact: true });
        await remove.focus();
        const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
        await sendLiveGwtEvent(page, { kind: "list_issue_monitor" });
        await page.waitForFunction(before => ((window as any).__gwtPlaywrightMessages ?? [])
          .some((entry: any) => entry.sequence > before && entry.payload.kind === "issue_monitor_status"), cursor);
        await expect(remove).toBeFocused();
        expect(await remove.evaluate(node => document.activeElement === node)).toBe(true);
        await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
        await testInfo.attach(`allowed-labels-${testInfo.project.name}`, { body: await labels.screenshot(), contentType: "image/png" });
        await labels.evaluate((section, allowed) => {
          for (const label of ["agent, ready", allowed]) {
            const row = [...section.querySelectorAll<HTMLElement>('[data-allowed-label]')]
              .find(node => node.dataset.allowedLabel === label)!;
            row.querySelector<HTMLButtonElement>("button")!.click();
          }
        }, fixture.allowed_label);
        await expectSavedLabels(page, []);
        await expect(labels.locator("summary")).toContainText("All labels");
        await expect(labels.locator('[data-metric="label-excluded"]')).toHaveText("Excluded by labels: 0", { timeout: 75_000 });
        const after = cli("issue.monitor.status");
        expect(after.enabled).toBe(false);
        expect(after.autonomous_mode).toBe(false);
        expect(after.active_launches).toEqual([]);
        expect(after.label_excluded_issues).toEqual([]);
        expect(errors).toEqual([]);
      } finally {
        await sendLiveGwtEvent(page, { kind: "set_issue_monitor_allowed_labels", allowed_labels: before.allowed_labels ?? [] });
        await expectSavedLabels(page, before.allowed_labels ?? []);
        if (created && !page.isClosed()) await sendLiveGwtEvent(page, { kind: "close_window", id: windowId });
      }
    });
  });
});

async function latestStatus(page: Page): Promise<any> {
  return page.evaluate(() => {
    const messages = ((window as any).__gwtPlaywrightMessages ?? [])
      .filter(({ payload }: any) => payload.kind === "issue_monitor_status");
    return messages.at(-1)?.payload.status;
  });
}

async function expectSavedLabels(page: Page, expected: string[]) {
  await expect.poll(async () => {
    await sendLiveGwtEvent(page, { kind: "list_issue_monitor" });
    return (await latestStatus(page))?.allowed_labels;
  }, { timeout: 30_000, intervals: [1_000, 2_000, 5_000] }).toEqual(expected);
}
