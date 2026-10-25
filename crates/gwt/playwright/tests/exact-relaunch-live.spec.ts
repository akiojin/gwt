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
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

type AgentWindow = { id: string; session_id: string; status: string; agent_id: string };
type RosterMember = { agent_member_id: string; name: string; enabled: boolean;
  assignment: { session_id: string | null; issue_number: number | null } | null };
type AgentRoster = { project_key: string; revision: number; capacity: number; members: RosterMember[]; error: string | null };

async function rosterAfter(page: Page, after: number, previousRevision = -1): Promise<AgentRoster> {
  return page.waitForFunction(({ after, previousRevision }) => {
    const message = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
      entry.sequence > after && entry.payload.kind === "agent_roster"
      && entry.payload.roster.revision > previousRevision);
    return message?.payload.roster || null;
  }, { after, previousRevision }, { timeout: 30_000 }).then(handle => handle.jsonValue());
}

async function readRoster(page: Page): Promise<AgentRoster> {
  const after = await cursor(page);
  await sendLiveGwtEvent(page, { kind: "agent_roster", action: { kind: "read" } });
  return rosterAfter(page, after);
}


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
    const messages = (window as any).__gwtPlaywrightMessages ?? [];
    const state = messages.findLast((entry: any) =>
      entry.payload.kind === "workspace_state");
    return (state?.payload.workspace.tabs ?? []).flatMap((tab: any) => tab.workspace.windows)
      .filter((entry: any) => entry.agent_id === "codex")
      .map(({ id, session_id, status, agent_id }: any) => ({ id, session_id, agent_id,
        status: messages.findLast((entry: any) => entry.sequence > state.sequence
          && entry.payload.kind === "terminal_status" && entry.payload.id === id)?.payload.status ?? status }));
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
  await sendLiveGwtEvent(page, { kind: "open_active_work_launch_wizard", branch_name: branch });
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

async function attachScreenshot(page: Page, testInfo: TestInfo, name: string) {
  const path = testInfo.outputPath(`${name}.png`);
  await page.screenshot({ path });
  await testInfo.attach(name, { path, contentType: "image/png" });
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
    const browserContext = page.context();
    let restored: Page | undefined;
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
      await page.getByRole("button", { name: "Works", exact: true }).click();
      if (await page.locator(".works-surface:not(.works-surface--preview) .works-full:not([hidden])").isVisible())
        await page.locator('.works-surface:not(.works-surface--preview) [data-action="back"]').click();
      await launchCodex(page, fixture.branch);

      // Initial launch: a fresh conversation whose native id gwt learns from the provider hook.
      const first = await launchAt(1);
      expect(first.resume_session_id).toBeNull();
      expect(first.gwt_session_id).toBeTruthy();
      const native = first.provider_session_id;
      const firstWindow = await liveCodexWindow(page, first.gwt_session_id!);
      const firstRecord = await syncedResumeId(first.gwt_session_id!);
      expect(tomlString(firstRecord, "agent_session_id")).toBe(native);
      const memberId = tomlString(firstRecord, "agent_member_id")!;
      const memberName = tomlString(firstRecord, "agent_name")!;
      expect(memberId).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i);
      expect(memberName?.trim()).toBeTruthy();
      const identities = (roster: AgentRoster) => roster.members.map(member => ({
        agent_member_id: member.agent_member_id, name: member.name, enabled: member.enabled,
      })).sort((left, right) => left.agent_member_id.localeCompare(right.agent_member_id));
      const assertCapacity = async (roster: AgentRoster) => {
        expect(roster.error).toBeFalsy();
        expect(roster.capacity).toBe(roster.members.filter(member => member.enabled).length);
        const preferences = join(fixture.home, ".gwt", "projects", roster.project_key, "project-state", "issue-monitor.json");
        await expect.poll(async () => JSON.parse(await readFile(preferences, "utf8")).max_active_agents,
          { timeout: 30_000, message: "persisted max_active equals the latest roster capacity" }).toBe(roster.capacity);
      };
      const checkMemberLinks = async (target: Page, currentWindow: AgentWindow, sessionId: string, stage: string) => {
        const roster = await readRoster(target);
        await assertCapacity(roster);
        const member = roster.members.find(member => member.agent_member_id === memberId)!;
        expect(member).toMatchObject({ name: memberName, enabled: true });
        const nativeHost = target.locator(`.works-full .works-terminal[data-window-id="${currentWindow.id}"]`);
        await expect(nativeHost).toBeVisible();
        await expect(target.locator(".works-full-name .work-member")).toHaveText(memberName);
        await target.locator(".works-full-name .work-member").press("Enter");
        await expect(target.getByRole("region", { name: "Main pane", exact: true })).toHaveAttribute("data-surface", "agents");
        await expect(target.locator(`.roster-row[data-member-id="${memberId}"]`)).toHaveAttribute("aria-selected", "true");
        await expect(target.locator(".roster-detail h3")).toHaveText(memberName);
        await attachScreenshot(target, testInfo, `agents-real-${stage}-${theme}`);
        if (member.assignment?.session_id) {
          expect(member.assignment.session_id).toBe(sessionId);
          await expect(target.locator(".roster-now button")).toBeVisible();
          await target.locator(".roster-now button").press("Enter");
          await expect(nativeHost.locator(".xterm-helper-textarea")).toBeVisible();
          expect((await codexWindows(target)).find(window => window.id === currentWindow.id)?.session_id).toBe(sessionId);
        } else {
          // This is a manual launch: no Issue or current assignment is invented.
          await expect(target.locator(".roster-now, .roster-detail .roster-open")).toHaveCount(0);
        }
        await testInfo.attach(`member-links-${stage}`, { body: JSON.stringify({ member_id: memberId, name: memberName,
          session_id: sessionId, current_task_link: member.assignment?.session_id ? "exact assignment" : "n/a (manual launch has no current assignment)" }), contentType: "application/json" });
        return roster;
      };
      const row = page.locator(`.work-row[data-window-id="${firstWindow.id}"]`);
      await expect(row.locator(".terminal-text-preview")).toBeVisible();
      await row.getByRole("button", { name: "Open terminal", exact: true }).click();
      await expect(page.locator(`.works-full .works-terminal[data-window-id="${firstWindow.id}"] .xterm-helper-textarea`)).toBeVisible();
      await attachScreenshot(page, testInfo, `launched-${theme}`);
      const initialRoster = await checkMemberLinks(page, firstWindow, first.gwt_session_id!, "initial");
      await page.getByRole("button", { name: "Agents", exact: true }).click();
      const enabledBefore = new Set(initialRoster.members.filter(member => member.enabled).map(member => member.agent_member_id));
      const beforeAdd = await cursor(page);
      await page.locator('.roster-capacity [data-action="add"]').click();
      const added = await rosterAfter(page, beforeAdd, initialRoster.revision);
      expect(added.capacity).toBe(initialRoster.capacity + 1); await assertCapacity(added);
      await expect(page.locator(".roster-count")).toHaveText(String(added.capacity));
      const addedMember = added.members.find(member => member.enabled && !enabledBefore.has(member.agent_member_id))!;
      expect(addedMember?.name.trim()).toBeTruthy();
      await page.locator(`.roster-row[data-member-id="${addedMember.agent_member_id}"]`).click();
      const beforeRest = await cursor(page);
      await page.locator('.roster-detail [data-action="rest"]').click();
      const rested = await rosterAfter(page, beforeRest, added.revision);
      expect(rested.capacity).toBe(initialRoster.capacity); await assertCapacity(rested);
      await expect(page.locator(".roster-count")).toHaveText(String(rested.capacity));
      expect(rested.members.find(member => member.agent_member_id === addedMember.agent_member_id))
        .toMatchObject({ name: addedMember.name, enabled: false });
      const savedRosterIdentities = identities(rested);
      await page.getByRole("button", { name: "Works", exact: true }).click();
      if (!await page.locator(".works-full:not([hidden])").isVisible()) await row.locator(".work-open").click();

      // Window restart: the provider exits, then an explicit runtime restart
      // rebinds the same window through the Works full-screen terminal.
      // Let the SessionStart runtime state land first so it cannot race the exit.
      await expect.poll(async () => (await codexWindows(page)).find((entry) => entry.id === firstWindow.id)?.status,
        { timeout: 60_000, message: "SessionStart runtime state reaches the window" }).toBe("idle");
      // A provider failure retains its diagnostic catalog window, while Works drops the dead census member.
      const beforeExit = await cursor(page);
      process.kill(first.pid, "SIGTERM");
      await page.waitForFunction(({ after, id }) => (window as any).__gwtPlaywrightMessages?.some((entry: any) =>
        entry.sequence > after && entry.payload.kind === "terminal_status"
        && entry.payload.id === id && entry.payload.status === "error"),
      { after: beforeExit, id: firstWindow.id }, { timeout: 60_000 });
      await expect.poll(async () => (await codexWindows(page)).find(entry => entry.id === firstWindow.id)?.status,
        { timeout: 60_000 }).toBe("error");
      await expect(row).toHaveCount(0);
      const wizardCursor = await cursor(page);
      await sendLiveGwtEvent(page, { kind: "open_active_work_launch_wizard", branch_name: fixture.branch });
      const { wizard } = await latestWizard(page, wizardCursor, true);
      await testInfo.attach("quick-start-cache-view", {
        body: JSON.stringify(wizard.quick_start_entries), contentType: "application/json" });
      await expect(page.locator("#wizard-modal")).toBeVisible();
      await attachScreenshot(page, testInfo, `quick-start-${theme}`);
      await wizardAction(page, { kind: "cancel" });
      await sendLiveGwtEvent(page, { kind: "restart_window", id: firstWindow.id });
      const second = await launchAt(2);
      expect(second.argv.slice(second.argv.indexOf("resume"))).toContain(native);
      expect(second.resume_session_id).toBe(native);
      expect(second.provider_session_id).toBe(native);
      const secondWindow = await liveCodexWindow(page, second.gwt_session_id!);
      expect(secondWindow.id).toBe(firstWindow.id);
      if (await page.locator(".works-surface:not(.works-surface--preview) .works-full:not([hidden])").isVisible())
        await page.locator('.works-surface:not(.works-surface--preview) [data-action="back"]').click();
      await expect(row).toBeVisible();
      await row.getByRole("button", { name: "Open terminal", exact: true }).click();
      await expect(page.locator(".works-full-meta")).toContainText("Interactive");
      await expect(page.locator(`.works-full .works-terminal[data-window-id="${secondWindow.id}"] .xterm-helper-textarea`)).toBeVisible();
      const secondRecord = await syncedResumeId(second.gwt_session_id!);
      expect(tomlString(secondRecord, "agent_session_id")).toBe(native);
      expect(tomlString(secondRecord, "agent_member_id")).toBe(memberId);
      expect(tomlString(secondRecord, "agent_name")).toBe(memberName);
      await expect(page.locator(".works-full-name .work-member")).toHaveText(memberName);
      const secondRoster = await readRoster(page); await assertCapacity(secondRoster);
      expect(identities(secondRoster)).toEqual(savedRosterIdentities);
      expect(secondRecord, "the restarted Session stays eligible for the next app restore")
        .toMatch(/^restore_window_on_startup\s*=\s*true$/m);
      expect(tomlString(secondRecord, "restore_source_session_id")).toBe(first.gwt_session_id);
      expect(tomlString(secondRecord, "launch_origin")).toBe("user_restart");
      await attachScreenshot(page, testInfo, `window-restarted-${theme}`);
      expect(errors, "console/page errors before app restart").toEqual([]);

      // App restart: the checkout process stops with the agent window open and
      // the fresh process restores it with the same exact resume identity.
      await page.close();
      await fixture.restart();
      restored = await browserContext.newPage();
      errors = capture(restored);
      await gotoLiveGwt(restored, fixture.url, { enableTestBridge: true });
      await expect(restored.locator("html")).toHaveAttribute("data-theme", theme);
      await restored.getByRole("button", { name: "Works", exact: true }).click();
      if (await restored.locator(".works-surface:not(.works-surface--preview) .works-full:not([hidden])").isVisible())
        await restored.locator('.works-surface:not(.works-surface--preview) [data-action="back"]').click();
      const third = await launchAt(3);
      expect(third.argv.slice(third.argv.indexOf("resume"))).toContain(native);
      expect(third.resume_session_id).toBe(native);
      expect(third.provider_session_id).toBe(native);
      const thirdWindow = await liveCodexWindow(restored, third.gwt_session_id!);
      await expect(restored.locator(`.work-row[data-window-id="${thirdWindow.id}"] .terminal-text-preview`)).toBeVisible();
      const thirdRecord = await syncedResumeId(third.gwt_session_id!);
      expect(tomlString(thirdRecord, "agent_session_id")).toBe(native);
      expect(tomlString(thirdRecord, "agent_member_id")).toBe(memberId);
      expect(tomlString(thirdRecord, "agent_name")).toBe(memberName);
      expect(thirdRecord, "automatic restore preserves the live Session restore flag")
        .toMatch(/^restore_window_on_startup\s*=\s*true$/m);
      expect(tomlString(thirdRecord, "restore_source_session_id")).toBe(second.gwt_session_id);
      expect(tomlString(thirdRecord, "launch_origin")).toBe("automatic_restore");
      expect((await fixture.launches()).map((launch) => launch.provider_session_id)).toEqual([native, native, native]);
      await restored.locator(`.work-row[data-window-id="${thirdWindow.id}"] .work-open`).click();
      const restoredRoster = await checkMemberLinks(restored, thirdWindow, third.gwt_session_id!, "app-restored");
      expect(identities(restoredRoster)).toEqual(savedRosterIdentities);
      expect(restoredRoster.members.find(member => member.agent_member_id === addedMember.agent_member_id))
        .toMatchObject({ name: addedMember.name, enabled: false });
      await testInfo.attach("agent-member-continuity", { body: JSON.stringify({ member_id: memberId, name: memberName,
        sessions: [first.gwt_session_id, second.gwt_session_id, third.gwt_session_id], capacity: restoredRoster.capacity,
        retained_resting_member: addedMember.agent_member_id }), contentType: "application/json" });
      await attachScreenshot(restored, testInfo, `app-restarted-${theme}`);
      expect(errors, "console/page errors after app restart").toEqual([]);
    } finally {
      try {
        for (const current of [page, restored]) if (current && !current.isClosed()) {
          await testInfo.attach("local-agent-observations", { body: JSON.stringify(await current.evaluate(() =>
            ((window as any).__gwtPlaywrightMessages ?? [])
              .filter((entry: any) => entry.payload.kind === "workspace_state" || entry.payload.projection?.local_agent_activity)
              .slice(-10)), null, 2), contentType: "application/json" });
        }
        if (restored && !restored.isClosed()) await restored.close();
        if (!page.isClosed()) await page.close();
        await testInfo.attach("provider-launches", {
          body: JSON.stringify(await fixture.launches(), null, 2), contentType: "application/json",
        });
      } finally {
        await fixture.stop();
      }
    }
  });
});
