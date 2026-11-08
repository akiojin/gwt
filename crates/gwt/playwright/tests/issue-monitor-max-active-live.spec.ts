import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const base = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const project = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";
const checkout = process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? "";

test.afterEach(async ({ page }, info) => {
  if (info.status === info.expectedStatus || page.isClosed()) return;
  const messages = await page.evaluate(() => (window as any).__gwtPlaywrightMessages ?? []);
  await info.attach("max-active-backend-messages", {
    body: JSON.stringify(messages), contentType: "application/json",
  });
});

test("Project settings and normal project saves preserve machine capacity across reload", async ({ page }, info) => {
  test.skip(!base || !home, "requires browser-check's isolated checkout backend");
  test.setTimeout(180_000);
  expect(base).not.toBe("");
  expect(home).not.toBe(process.env.HOME);
  expect(home).not.toBe("");
  await withLiveGwtBackendLock(base, info, async () => {
    const errors: string[] = [], saves: unknown[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("websocket", socket => socket.on("framesent", ({ payload }) => {
      const message = JSON.parse(String(payload));
      if (message.kind === "set_issue_monitor_max_active_agents") saves.push(message.max_active_agents);
    }));
    const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(GWT_|CODEX_|CLAUDE_)/.test(key)));
    Object.assign(env, { HOME: home, USERPROFILE: home });
    const operation = (name: string, params: object = {}) => {
      const result = spawnSync(join(checkout, "target/debug/gwtd"), [], { cwd: project, env, encoding: "utf8",
        input: JSON.stringify({ schema_version: 1, operation: name, params }), timeout: 30_000 });
      expect(result.status, result.stderr).toBe(0);
      const envelope = JSON.parse(result.stdout);
      expect(envelope.ok).toBe(true);
      return envelope;
    };
    await gotoLiveGwt(page, base, { enableTestBridge: true });
    // Opening Issues requests the Monitor snapshot in a fresh workspace.
    await page.locator('button[data-surface="issues"]').click();
    const issueHost = page.locator('.split-pane[data-active="true"][data-surface="issues"] .workspace-window[data-preset="issue"]');
    await issueHost.getByRole("button", { name: "Issues settings", exact: true }).click();
    const issues = page.locator('.workspace-window[data-preset="settings"] [data-settings-panel="issues"]');
    await expect(issues).toBeVisible();
    await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .some(({ payload }: any) => payload.kind === "issue_monitor_status"));
    await sendLiveGwtEvent(page, { kind: "agent_roster", action: { kind: "read" }, expected_revision: null });
    await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .some(({ payload }: any) => payload.kind === "agent_roster" && payload.roster.members.length > 0));
    const initial = operation("issue.monitor.status");
    const preferences = join(home, ".gwt/projects", initial.project_store.hash, "project-state/issue-monitor.json");
    const machinePreferences = join(home, ".gwt/machine-state/agent-settings.json");
    const machineBefore = await readFile(machinePreferences, "utf8");
    const prefs = async () => JSON.parse(await readFile(preferences, "utf8"));
    const before = await prefs();
    expect(before.agent_roster).toBeTruthy();
    const capacity = (value: any) => ({
      agent_roster: value.agent_roster,
      max_active_agents: value.max_active_agents,
      max_active_last_change: value.max_active_last_change,
    });

    await expect(issues.locator(".issue-bridge-root")).toHaveCount(0);
    await expect(issues.getByLabel("Max active", { exact: true })).toHaveCount(0);
    await expect(issues.getByLabel("Max active mode", { exact: true })).toHaveCount(0);
    await expect(issues.getByRole("button", { name: "Apply Max active", exact: true })).toHaveCount(0);
    await expect(issues.locator(".settings-issues-capacity")).toHaveCount(0);
    // A normal project save causes a real status redraw without a capacity write.
    // Explicit Manual/Auto setter semantics belong to the machine-policy owner.
    const saveCursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
    operation("issue.monitor.config.set", { allowed_labels: ["t26-settings"] });
    const waitForLabels = (cursor = 0) => page.waitForFunction(cursor =>
      ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ sequence, payload }: any) => sequence > cursor
          && payload.kind === "issue_monitor_status"
          && payload.status.allowed_labels?.join(",") === "t26-settings"), cursor);
    await waitForLabels(saveCursor);
    await expect.poll(prefs).toMatchObject({ allowed_labels: ["t26-settings"] });
    await page.locator('button[data-surface="issues"]').click();
    await issueHost.getByRole("button", { name: "Issues settings", exact: true }).click();
    await expect(issues).toBeVisible();
    expect(capacity(await prefs())).toEqual(capacity(before));
    expect(await readFile(machinePreferences, "utf8")).toBe(machineBefore);
    expect(saves).toEqual([]);

    await gotoLiveGwt(page, base, { enableTestBridge: true });
    await page.locator('button[data-surface="issues"]').click();
    await issueHost.getByRole("button", { name: "Issues settings", exact: true }).click();
    await expect(issues).toBeVisible();
    await waitForLabels();
    await expect(issues.getByLabel("Max active", { exact: true })).toHaveCount(0);
    expect(capacity(await prefs())).toEqual(capacity(before));
    expect(await readFile(machinePreferences, "utf8")).toBe(machineBefore);
    expect(saves).toEqual([]);
    await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.endsWith("light") ? "light" : "dark");
    const screenshot = info.outputPath(`max-active-${info.project.name}.png`);
    await issues.screenshot({ path: screenshot });
    await info.attach(`max-active-${info.project.name}`, { path: screenshot, contentType: "image/png" });
    expect(errors).toEqual([]);
  });
});
