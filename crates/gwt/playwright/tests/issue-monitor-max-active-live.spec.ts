import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { gotoLiveGwt, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const base = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const project = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";
const checkout = process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? "";

test("Max active commits the final stepper value once and Auto never saves numbers", async ({ page }, info) => {
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
    const issues = page.locator('.workspace-window[data-preset="settings"] [data-settings-panel="issues"]');
    await expect(issues).toBeVisible();
    await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .some(({ payload }: any) => payload.kind === "issue_monitor_status"));
    const waitForLimit = (override: number | null) => page.waitForFunction(expected =>
      ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "issue_monitor_status")
        .at(-1)?.payload.status.max_active_agents_override === expected, override);
    operation("issue.monitor.config.set", { max_active: 6 });
    await waitForLimit(6);
    const initial = operation("issue.monitor.status");
    expect(JSON.parse(initial.output).max_active_last_change).toMatchObject({ source: "json_operation" });
    const preferences = join(home, ".gwt/projects", initial.project_store.hash, "project-state/issue-monitor.json");
    const prefs = async () => JSON.parse(await readFile(preferences, "utf8"));

    const disclosure = issues.locator(".settings-issues-options");
    if (!await disclosure.evaluate(element => (element as HTMLDetailsElement).open)) {
      await disclosure.locator(":scope > summary").click();
    }
    const input = issues.getByLabel("Max active", { exact: true });
    const mode = issues.getByLabel("Max active mode", { exact: true });
    const apply = issues.getByRole("button", { name: "Apply Max active", exact: true });
    await expect(mode).toHaveValue("manual");
    await expect(input).toHaveValue("6");
    await input.fill("2");
    operation("issue.monitor.config.set", { max_active_mode: "auto" });
    await waitForLimit(null);
    await expect(mode).toHaveValue("auto");
    await expect(input).toBeDisabled();
    await expect(apply).toBeDisabled();
    await apply.evaluate(button => (button as HTMLButtonElement).click());
    expect(saves).toEqual([]);
    expect((await prefs()).max_active_last_change.source).toBe("json_operation");
    operation("issue.monitor.config.set", { max_active: 6 });
    await waitForLimit(6);
    await expect(mode).toHaveValue("manual");
    await expect(input).toHaveValue("6");
    await expect(apply).toBeDisabled();
    await input.focus();
    await input.press("ArrowDown");
    await input.press("ArrowDown");
    await input.press("ArrowDown");
    await expect(input).toHaveValue("3");
    expect(saves).toEqual([]);
    expect((await prefs()).max_active_agents).toBe(6);
    await apply.click();
    expect(saves).toEqual([3]);
    await expect.poll(prefs, { timeout: 30_000 }).toMatchObject({ max_active_agents: 3,
      max_active_last_change: { source: "gui", max_active_agents_override: 3 } });
    await expect(apply).toBeDisabled();
    expect(saves).toEqual([3]);
    const after = JSON.parse(operation("issue.monitor.status").output);
    expect(after.max_active_last_change.source).toBe("gui");
    expect(Number.isFinite(Date.parse(after.max_active_last_change.changed_at))).toBe(true);

    await mode.selectOption("auto");
    await expect.poll(prefs, { timeout: 30_000 }).toMatchObject({ max_active_agents_mode: "auto" });
    await expect(input).toBeDisabled();
    await expect(apply).toBeDisabled();
    await expect.poll(() => page.evaluate(() => {
      const status = ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "issue_monitor_status").at(-1)?.payload.status;
      const capacity = document.querySelector('[data-settings-issues-controls] .settings-issues-capacity');
      const input = capacity?.querySelector('input[aria-label="Max active"]') as HTMLInputElement | null;
      return status?.max_active_agents_override == null && input?.disabled
        && input.value === String(status.max_active_agents)
        && capacity!.textContent!.includes("Measuring capacity") === !status.agent_capacity?.measurement_complete;
    })).toBe(true);
    await input.dispatchEvent("change");
    await input.dispatchEvent("input");
    expect(saves).toEqual([3, null]);
    const status = JSON.parse(operation("issue.monitor.status").output);
    expect(status.max_active_last_change).toMatchObject({ source: "gui", max_active_agents_override: null });
    await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.endsWith("light") ? "light" : "dark");
    const screenshot = info.outputPath(`max-active-${info.project.name}.png`);
    await issues.screenshot({ path: screenshot });
    await info.attach(`max-active-${info.project.name}`, { path: screenshot, contentType: "image/png" });
    expect(errors).toEqual([]);
  });
});
