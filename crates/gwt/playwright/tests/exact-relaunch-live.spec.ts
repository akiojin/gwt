/* SPEC #1921 T447 / SC-069: exact relaunch continuity on an isolated checkout.
 * gwt runs from this checkout with a fresh HOME on a throwaway project
 * repository; only the Codex executable is an argv recorder, so no provider conversation or quota is involved. The
 * contract under test ends where gwt hands off to the provider:
 *   (a) the exact native resume identity reaches the provider argv, and
 *   (b) after a window restart and an app restart the relaunched window is
 *       rebound to that same exact session.
 */
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { startExactRelaunchFixture, type ExactRelaunchInvocation } from "./_helpers/exact-relaunch";
import { gotoLiveGwt, openLiveLaunchWizardForBranch, sendLiveGwtEvent } from "./_helpers/live-gwt";

type AgentWindow = { id: string; session_id: string; status: string; agent_id: string };

function capture(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
  return errors;
}

async function cursor(page: Page): Promise<number> {
  return page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
}

async function latestWizard(page: Page, after: number, requireOpen = false) {
  return (await page.waitForFunction(({ after, requireOpen }) => {
    const message = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
      entry.sequence > after && entry.payload.kind === "launch_wizard_state"
      && (!requireOpen || entry.payload.wizard)
      && (!entry.payload.wizard || (!entry.payload.wizard.is_hydrating
        && !entry.payload.wizard.runtime_resolution_pending
        && !entry.payload.wizard.launch_materialization_pending)));
    return message ? { wizard: message.payload.wizard } : null;
  }, { after, requireOpen }, { timeout: 30_000 })).jsonValue();
}

async function wizardAction(page: Page, action: unknown) {
  const after = await cursor(page);
  await sendLiveGwtEvent(page, { kind: "launch_wizard_action", action,
    bounds: { x: 80, y: 80, width: 700, height: 440 } });
  return latestWizard(page, after);
}

async function codexWindows(page: Page): Promise<AgentWindow[]> {
  return page.evaluate(() => {
    const state = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
      entry.payload.kind === "workspace_state");
    return (state?.payload.workspace.tabs ?? []).flatMap((tab: any) => tab.workspace.windows)
      .filter((entry: any) => entry.agent_id === "codex")
      .map(({ id, session_id, status, agent_id }: any) => ({ id, session_id, status, agent_id }));
  });
}

async function liveCodexWindow(page: Page, sessionId: string): Promise<AgentWindow> {
  await expect.poll(async () => (await codexWindows(page))
    .find((entry) => entry.session_id === sessionId && ["running", "idle"].includes(entry.status)),
  { timeout: 60_000, message: `window bound to Session ${sessionId}` }).toBeTruthy();
  return (await codexWindows(page)).find((entry) => entry.session_id === sessionId)!;
}

async function launchCodex(page: Page, branch: string) {
  const after = await cursor(page);
  await openLiveLaunchWizardForBranch(page, branch);
  await latestWizard(page, after, true);
  await wizardAction(page, { kind: "set_launch_path", path: "manual_setup" });
  // The installed (fixture) Codex is the only route: the wizard has no version choice.
  let state = await wizardAction(page, { kind: "set_agent", agent_id: "codex" });
  for (let step = 0; step < 12 && state.wizard; step += 1) {
    expect(state.wizard.error).toBeFalsy();
    if (state.wizard.selected_runtime_target !== "host"
      && state.wizard.runtime_target_options?.some((option: any) => option.value === "host")) {
      state = await wizardAction(page, { kind: "set_runtime_target", target: "Host" });
    } else {
      expect(state.wizard.primary_action_enabled, state.wizard.primary_action_disabled_reason).toBe(true);
      state = await wizardAction(page, { kind: "submit" });
    }
  }
  expect(state.wizard).toBeNull();
}

async function sessionRecord(home: string, sessionId: string): Promise<string> {
  const pending = [home];
  while (pending.length) {
    const directory = pending.pop()!;
    for (const entry of await readdir(directory, { withFileTypes: true }).catch(() => [])) {
      const path = join(directory, entry.name);
      if (entry.isDirectory() && entry.name !== "runtime" && !entry.isSymbolicLink()) pending.push(path);
      else if (entry.name === `${sessionId}.toml`) return readFile(path, "utf8");
    }
  }
  return "";
}

function tomlString(record: string, key: string): string | null {
  return record.match(new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, "m"))?.[1] ?? null;
}

test.describe("Exact relaunch continuity (isolated checkout)", () => {
  // The fixture spawns this checkout's target/debug/gwt, which the embedded
  // CI Playwright job does not build.
  test.skip(process.env.GWT_PLAYWRIGHT_EXACT_RELAUNCH !== "1",
    "GWT_PLAYWRIGHT_EXACT_RELAUNCH=1 is not set; checkout relaunch E2E skipped");
  test.skip(process.platform === "win32", "the argv recorder fixture requires POSIX executables");
  test.setTimeout(420_000);

  test("restart evidence failure reaps the owned process", async ({}, testInfo) => {
    let starts = 0;
    const cleanups: { pid: number; ps_status: number }[] = [];
    const fixture = await startExactRelaunchFixture({
      async attach(name, attachment) {
        if (name === "exact-relaunch-fixture" && ++starts === 2) throw new Error("fixture evidence failure");
        if (name === "fixture-process-cleanup") cleanups.push(JSON.parse(attachment.body!.toString()));
        await testInfo.attach(name, attachment);
      },
    } as TestInfo);
    try {
      await expect(fixture.restart()).rejects.toThrow("fixture evidence failure");
      expect(cleanups).toHaveLength(2);
      expect(cleanups.every(cleanup => cleanup.pid > 0 && cleanup.ps_status === 1)).toBe(true);
    } finally {
      await fixture.stop();
    }
  });

  test("window restart and app restart rebind the exact provider session", async ({ page }, testInfo) => {
    const fixture = await startExactRelaunchFixture(testInfo);
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    let errors = capture(page);
    const launchAt = async (count: number): Promise<ExactRelaunchInvocation> => {
      await expect.poll(async () => (await fixture.launches()).length,
        { timeout: 90_000, message: `provider launch #${count}` }).toBeGreaterThanOrEqual(count);
      return (await fixture.launches())[count - 1];
    };
    const syncedResumeId = async (sessionId: string) => {
      await expect.poll(async () => tomlString(await sessionRecord(fixture.home, sessionId), "agent_session_id"),
        { timeout: 60_000, message: `Session ${sessionId} learns the native id` }).toBeTruthy();
      return sessionRecord(fixture.home, sessionId);
    };
    try {
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await launchCodex(page, fixture.branch);

      // Initial launch: a fresh conversation whose native id gwt learns from the provider hook.
      const first = await launchAt(1);
      expect(first.resume_session_id).toBeNull();
      expect(first.gwt_session_id).toBeTruthy();
      const native = first.provider_session_id;
      const firstWindow = await liveCodexWindow(page, first.gwt_session_id!);
      expect(tomlString(await syncedResumeId(first.gwt_session_id!), "agent_session_id")).toBe(native);
      await testInfo.attach(`launched-${theme}`, { body: await page.screenshot(), contentType: "image/png" });

      // Window restart: the provider exits, the user presses Restart agent.
      // Let the SessionStart runtime state land first so it cannot race the exit.
      await expect.poll(async () => (await codexWindows(page)).find((entry) => entry.id === firstWindow.id)?.status,
        { timeout: 60_000, message: "SessionStart runtime state reaches the window" }).toBe("idle");
      const wizardCursor = await cursor(page);
      await openLiveLaunchWizardForBranch(page, fixture.branch);
      const { wizard } = await latestWizard(page, wizardCursor, true);
      await testInfo.attach("quick-start-cache-view", {
        body: JSON.stringify(wizard.quick_start_entries), contentType: "application/json" });
      await expect(page.locator("#wizard-modal .start-method-list")).toBeVisible();
      await testInfo.attach(`quick-start-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      await wizardAction(page, { kind: "cancel" });
      // The provider dies (a clean exit closes the window instead); the window
      // runtime turns error and the Restart agent control appears.
      process.kill(first.pid, "SIGTERM");
      const restart = page.locator(`.workspace-window[data-id="${firstWindow.id}"] [data-action="restart"]`);
      await expect(restart).toBeVisible({ timeout: 60_000 });
      await restart.click();
      const second = await launchAt(2);
      expect(second.argv.slice(second.argv.indexOf("resume"))).toContain(native);
      expect(second.resume_session_id).toBe(native);
      expect(second.provider_session_id).toBe(native);
      await liveCodexWindow(page, second.gwt_session_id!);
      const secondRecord = await syncedResumeId(second.gwt_session_id!);
      expect(tomlString(secondRecord, "agent_session_id")).toBe(native);
      expect(tomlString(secondRecord, "restore_source_session_id")).toBe(first.gwt_session_id);
      expect(tomlString(secondRecord, "launch_origin")).toBe("user_restart");
      await testInfo.attach(`window-restarted-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      expect(errors, "console/page errors before app restart").toEqual([]);

      // App restart: the checkout process stops with the agent window open and
      // the fresh process restores it with the same exact resume identity.
      await fixture.restart();
      await page.close();
      const restored = await page.context().newPage();
      errors = capture(restored);
      await gotoLiveGwt(restored, fixture.url, { enableTestBridge: true });
      await expect(restored.locator("html")).toHaveAttribute("data-theme", theme);
      const third = await launchAt(3);
      expect(third.argv.slice(third.argv.indexOf("resume"))).toContain(native);
      expect(third.resume_session_id).toBe(native);
      expect(third.provider_session_id).toBe(native);
      await liveCodexWindow(restored, third.gwt_session_id!);
      const thirdRecord = await syncedResumeId(third.gwt_session_id!);
      expect(tomlString(thirdRecord, "agent_session_id")).toBe(native);
      expect(tomlString(thirdRecord, "restore_source_session_id")).toBe(second.gwt_session_id);
      expect(tomlString(thirdRecord, "launch_origin")).toBe("automatic_restore");
      expect((await fixture.launches()).map((launch) => launch.provider_session_id)).toEqual([native, native, native]);
      await testInfo.attach(`app-restarted-${theme}`, { body: await restored.screenshot(), contentType: "image/png" });
      expect(errors, "console/page errors after app restart").toEqual([]);
    } finally {
      try {
        await testInfo.attach("provider-launches", {
          body: JSON.stringify(await fixture.launches(), null, 2), contentType: "application/json",
        });
      } finally {
        await fixture.stop();
      }
    }
  });
});
