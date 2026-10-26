/* SPEC #1921 T447 / SC-069: exact relaunch continuity on an isolated checkout.
 * gwt runs from this checkout with a fresh HOME on a throwaway project
 * repository; provider executables are argv recorders, so no remote conversation or quota is involved. The
 * contract under test ends where gwt hands off to the provider:
 *   (a) the exact native resume identity reaches the provider argv, and
 *   (b) after a window restart and an app restart the relaunched window is
 *       rebound to that same exact session.
 */
import { chmod, readdir, readFile, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { startExactRelaunchFixture, type ExactRelaunchInvocation } from "./_helpers/exact-relaunch";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

type AgentWindow = { id: string; session_id: string; status: string; agent_id: string };
type RosterMember = { agent_member_id: string; agent_id: string; name: string; enabled: boolean;
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
  await expect.poll(async () => {
    const entry = (await codexWindows(page)).find(entry => entry.session_id === sessionId && ["running", "idle"].includes(entry.status));
    const activity = await page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .map(({ payload }: any) => payload.kind === "local_agent_activity" ? payload.activity : payload.projection?.local_agent_activity)
      .filter((snapshot: any) => snapshot?.project_key === location.pathname.split("/")[2] && snapshot.host_id
        && Number.isFinite(Date.parse(snapshot.observed_at)))
      .sort((left: any, right: any) => Date.parse(left.observed_at) - Date.parse(right.observed_at)).at(-1));
    return entry && activity?.agents?.some((agent: any) => agent.window_id === entry.id
      && agent.session_id === sessionId && agent.runtime_incarnation > 0);
  }, { timeout: 60_000, message: `live child and window bound to Session ${sessionId}` }).toBeTruthy();
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

  test("named registration persists its UUID through rejection, reload and native restart", async ({ page }, info) => {
    const fixture = await startExactRelaunchFixture(info);
    const errors = capture(page);
    const theme = info.project.name.includes("light") ? "light" : "dark";
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
    const surface = page.locator(".agents-surface:not(.agents-surface--preview)");
    const name = surface.getByRole("textbox", { name: "Name for new member", exact: true });
    const register = surface.locator('.roster-capacity [data-action="add"]');
    try {
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await page.locator('.op-rail__surface[data-surface="agents"]').click();
      const before = await readRoster(page);
      await expect(register).toBeDisabled();
      await name.fill(` ${before.members[0].name.toLowerCase()} `);
      await expect(name).toHaveAttribute("aria-invalid", "true");
      await expect(register).toBeDisabled();
      await name.fill("  Maple contributor  ");
      await surface.getByRole("combobox", { name: "Tool for new member", exact: true }).selectOption("codex");
      const sequence = await cursor(page);
      await register.click();
      const added = await rosterAfter(page, sequence, before.revision);
      expect(added.error).toBeNull();
      expect(added.members).toHaveLength(before.members.length + 1);
      const member = added.members.find(member => member.name === "Maple contributor")!;
      expect(member.agent_member_id).toMatch(/^[0-9a-f-]{36}$/);
      expect(member.agent_id).toBe("codex");
      expect(member.assignment).toBeNull();
      await expect(name).toHaveValue("");
      // A stale writer must not replace the registered identity or this draft.
      await name.fill("Retry contributor");
      const staleSequence = await cursor(page);
      await sendLiveGwtEvent(page, { kind: "agent_roster", expected_revision: before.revision,
        action: { kind: "add", agent_id: "codex", name: "Retry contributor" } });
      const rejected: AgentRoster = await page.waitForFunction(after =>
        (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.sequence > after
          && entry.payload.kind === "agent_roster" && entry.payload.roster.error?.includes("changed"))?.payload.roster,
      staleSequence).then(handle => handle.jsonValue());
      expect(rejected.error).toContain("changed");
      expect(rejected.revision).toBe(added.revision);
      expect(rejected.members).toEqual(added.members);
      await expect(name).toHaveValue("Retry contributor");
      const retrySequence = await cursor(page);
      await register.click();
      const retried = await rosterAfter(page, retrySequence, added.revision);
      expect(retried.error).toBeNull();
      expect(retried.members.find(member => member.name === "Retry contributor")?.agent_member_id).toBeTruthy();
      await page.setViewportSize({ width: 400, height: 1000 });
      await expect.poll(() => surface.evaluate(node => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await page.screenshot({ path: info.outputPath(`named-registration-${theme}.png`) });
      await page.reload();
      await page.locator('.op-rail__surface[data-surface="agents"]').click();
      expect((await readRoster(page)).members.find(value => value.agent_member_id === member.agent_member_id)).toEqual(member);
      const restarted = await fixture.restart();
      await gotoLiveGwt(page, restarted, { enableTestBridge: true });
      await page.locator('.op-rail__surface[data-surface="agents"]').click();
      const restored = await readRoster(page);
      expect(restored.members.find(value => value.agent_member_id === member.agent_member_id)).toEqual(member);
      expect(restored.members.find(value => value.name === "Retry contributor")?.agent_member_id)
        .toBe(retried.members.find(value => value.name === "Retry contributor")?.agent_member_id);
      expect(errors, "actual registration/reload/restart console and page errors").toEqual([]);
    } finally {
      await fixture.stop();
    }
  });

  test("Works output lines survive saved settings, reload and owned native restart", async ({ page }, info) => {
    const lines = ["body01", "body02", "body03", "body04", "body05", "body06", "body07", "body08", "<x>raw", "footer", "> input", "last"];
    const fixture = await startExactRelaunchFixture(info, {
      async prepare({ bin }) {
        const path = join(bin, "codex");
        const provider = await readFile(path, "utf8");
        const command = "  if (line.trim() === 'GWT_FIXTURE_EXIT')";
        expect(provider).toContain(command);
        await writeFile(path, provider.replace(command, `  if (line.trim() === 'GWT_FIXTURE_LINES') {
    process.stdout.write('\\u001b[2J\\u001b[H');
    for (const text of ${JSON.stringify(lines)}) console.log(text);
    return;
  }
` + command));
      },
    });
    const errors = capture(page);
    const theme = info.project.name.includes("light") ? "light" : "dark";
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
    const surface = page.locator(".works-surface:not(.works-surface--preview)");
    const config = join(fixture.home, ".gwt", "config.toml");
    try {
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await page.locator('.op-rail__surface[data-surface="works"]').click();
      await expect(surface).toHaveCSS("--works-preview-lines", "3");
      await page.locator('.op-rail__surface[data-surface="settings"]').click();
      const setting = page.getByRole("spinbutton", { name: "Works output lines", exact: true });
      await expect(setting).toHaveValue("3");
      await setting.fill("10"); await setting.press("Tab");
      await expect(page.locator('[data-role="system-settings-status"]')).toContainText("Saved system settings");
      expect(await readFile(config, "utf8")).toMatch(/^works_preview_lines\s*=\s*10$/m);
      await page.screenshot({ path: info.outputPath("settings-native-works-output-lines.png") });
      await page.locator('.op-rail__surface[data-surface="works"]').click();
      await expect(surface).toHaveCSS("--works-preview-lines", "10");
      await launchCodex(page, fixture.branch);
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 90_000 }).toBe(1);
      const launch = (await fixture.launches())[0];
      const session = launch.gwt_session_id!;
      expect(session).toBeTruthy();
      const checkOutput = async (stage: string, observedSession = session) => {
        const window = await liveCodexWindow(page, observedSession);
        await page.locator('.op-rail__surface[data-surface="works"]').click();
        if (await surface.locator(".works-full").isVisible()) await surface.locator('[data-action="back"]').click();
        const row = surface.locator(`.work-row[data-window-id="${window.id}"]`);
        await expect(row).toBeVisible({ timeout: 60_000 });
        await sendLiveGwtEvent(page, { kind: "terminal_input", id: window.id, data: "GWT_FIXTURE_LINES\r" });
        await expect(row.locator(".terminal-text-preview")).toHaveText(lines.slice(-10).join("\n"), { useInnerText: false });
        await expect(row.locator(".terminal-text-preview x")).toHaveCount(0);
        await expect(surface).toHaveCSS("--works-preview-lines", "10");
        await expect(surface.locator(".works-counts [data-count]")).toHaveCount(4);
        await expect(surface.locator(".works-today [data-metric]")).toHaveCount(6);
        await expect(row.locator(".work-stats [data-stat]")).toHaveCount(8);
        await row.locator(".work-tail").click(); await expect(surface.locator(".works-full")).toBeHidden();
        await row.getByRole("button", { name: "Open terminal" }).click();
        await expect(surface.locator(".works-full .xterm-helper-textarea")).toBeVisible();
        await surface.locator('[data-action="back"]').click();
        await page.screenshot({ path: info.outputPath(`works-output-lines-native-${stage}.png`) });
      };
      await checkOutput("saved");
      // Remove only this fixture's non-agent Settings host. A restored host
      // would fetch System settings on its own and mask Works-only bootstrap.
      const beforeClose = await cursor(page);
      await sendLiveGwtEvent(page, { kind: "list_windows" });
      const windowList = await page.waitForFunction(after => ((window as any).__gwtPlaywrightMessages || [])
        .findLast((entry: any) => entry.sequence > after && entry.payload.kind === "window_list")?.payload.windows,
      beforeClose).then(handle => handle.jsonValue());
      const settingsHost = windowList.filter((window: any) => window.preset === "settings");
      expect(settingsHost).toHaveLength(1);
      expect(settingsHost[0].session_id ?? null).toBeNull();
      expect(settingsHost[0].agent_id ?? null).toBeNull();
      expect(settingsHost[0].is_pm ?? false).toBe(false);
      const currentWindow = await liveCodexWindow(page, session);
      const launchesBeforeClose = (await fixture.launches()).length;
      const closeCursor = await cursor(page);
      await sendLiveGwtEvent(page, { kind: "close_window", id: settingsHost[0].id });
      await page.waitForFunction(({ after, id }) => ((window as any).__gwtPlaywrightMessages || []).some((entry: any) =>
        entry.sequence > after && entry.payload.kind === "workspace_state"
        && !entry.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows).some((window: any) => window.id === id)),
      { after: closeCursor, id: settingsHost[0].id });
      await expect(page.locator('.workspace-window[data-preset="settings"]')).toHaveCount(0);
      expect((await liveCodexWindow(page, session)).id).toBe(currentWindow.id);
      expect((await fixture.launches()).length).toBe(launchesBeforeClose);
      await page.reload();
      await expect(surface).toHaveCSS("--works-preview-lines", "10");
      await checkOutput("reload");
      await expect(page.locator('.workspace-window[data-preset="settings"]')).toHaveCount(0);
      const oldPid = fixture.ownedPid;
      const restarted = await fixture.restart();
      expect(fixture.ownedPid).not.toBe(oldPid);
      await gotoLiveGwt(page, restarted, { enableTestBridge: true });
      await page.locator('.op-rail__surface[data-surface="works"]').click();
      await expect(surface).toHaveCSS("--works-preview-lines", "10");
      expect(await readFile(config, "utf8")).toMatch(/^works_preview_lines\s*=\s*10$/m);
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 90_000 }).toBe(2);
      const resumed = (await fixture.launches())[1];
      expect(resumed.gwt_session_id).toBeTruthy();
      expect(resumed.gwt_session_id).not.toBe(session);
      expect(resumed.resume_session_id).toBe(launch.provider_session_id);
      expect(resumed.provider_session_id).toBe(launch.provider_session_id);
      await checkOutput("restart", resumed.gwt_session_id!);
      const restoredRecord = await sessionRecord(fixture.home, resumed.gwt_session_id!);
      expect(tomlString(restoredRecord, "agent_session_id")).toBe(launch.provider_session_id);
      expect(tomlString(restoredRecord, "restore_source_session_id")).toBe(session);
      expect(tomlString(restoredRecord, "launch_origin")).toBe("automatic_restore");
      await info.attach("works-output-lines-restore-continuity", { body: JSON.stringify({
        source_session: session, restored_session: resumed.gwt_session_id,
        native_conversation: resumed.provider_session_id, settings_host_restored: false,
      }), contentType: "application/json" });
      await expect(page.locator('.workspace-window[data-preset="settings"]')).toHaveCount(0);
      await page.setViewportSize({ width: 400, height: 1000 });
      expect(await surface.evaluate(node => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
      await page.screenshot({ path: info.outputPath("works-output-lines-native-400.png") });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      expect(errors, "real settings/terminal/restart console and page errors").toEqual([]);
    } finally {
      await fixture.stop();
    }
  });

  test("Works shows the real PM and four implementation children after reload", async ({ page }, info) => {
    test.setTimeout(420_000);
    const purpose = "Review active work and coordinate delivery";
    const fixture = await startExactRelaunchFixture(info, {
      monitorPreferences: { max_active_agents: 4 },
      async prepare({ home, bin, project, env }) {
        const git = (...args: string[]) => execFileSync("git", args, { cwd: project, env, encoding: "utf8" });
        git("clone", "--bare", "--quiet", project, join(home, "origin.git"));
        git("remote", "add", "origin", join(home, "origin.git")); git("fetch", "--quiet", "origin");
        for (let i = 1; i <= 4; i += 1) git("branch", `worker-${i}`);
        let provider = await readFile(join(bin, "codex"), "utf8");
        provider = provider.replace(/const native = resumed \|\| requested \|\| [^;]+;/,
          "const native = resumed || requested || require('node:crypto').randomUUID();");
        provider = provider.replace("  if (line.trim() === 'GWT_FIXTURE_EXIT')", `
  if (['GWT_FIXTURE_BEGIN', 'GWT_FIXTURE_IDLE', 'GWT_FIXTURE_PM_BEGIN'].includes(line.trim())) {
    const event = line.trim() === 'GWT_FIXTURE_IDLE' ? 'Stop' : 'UserPromptSubmit';
    const hook = require('node:child_process').spawnSync(process.env.GWT_BIN_PATH,
      ['hook', 'event', event], {cwd:process.cwd(),env:process.env,encoding:'utf8',timeout:30000,
        input:JSON.stringify({session_id:native,hook_event_name:event,cwd:process.cwd()})});
    if (hook.status !== 0) { console.log('WORKS_HOOK_FAILED'); return; }
    if (line.trim() === 'GWT_FIXTURE_PM_BEGIN') {
      const result = require('node:child_process').spawnSync(process.env.GWT_BIN_PATH, [], {
        cwd:process.cwd(),env:process.env,encoding:'utf8',timeout:30000,
        input:JSON.stringify({schema_version:1,operation:'pm.status.set',params:{
          items:[{kind:'working',text:${JSON.stringify(purpose)},started_at:new Date().toISOString()}],
          next_trigger:'Agent update'}})});
      if (result.status !== 0 || !JSON.parse(result.stdout).ok) { console.log('WORKS_STATUS_FAILED'); return; }
    }
    console.log('WORKS_STATE ' + event); return;
  }
  if (line.trim() === 'GWT_FIXTURE_EXIT')`);
        let claude = provider.replace("codex-cli 0.116.0", "2.1.0 (Claude Code)")
          .replace("'.codex', 'hooks.json'", "'.claude', 'settings.local.json'")
          .replace("const resumed = resumeAt < 0 ? null :", "const resumed = option('--resume') || (resumeAt < 0 ? null :")
          .replace(" ?? null);\nconst requested", " ?? null));\nconst requested");
        const transcriptStart = claude.indexOf("const directory = path.join(process.env.HOME, '.codex'");
        const transcriptEnd = claude.indexOf("fs.appendFileSync(", transcriptStart);
        claude = claude.slice(0, transcriptStart) + `
const directory = path.join(process.env.HOME, '.claude', 'projects', 'works-fixture');
fs.mkdirSync(directory, {recursive:true});
const rollout = path.join(directory, native + '.jsonl');
for (const age of [60000, 0]) fs.appendFileSync(rollout, JSON.stringify({
  type:'assistant',sessionId:native,cwd:process.cwd(),timestamp:new Date(Date.now()-age).toISOString(),
  message:{id:require('node:crypto').randomUUID(),model:'claude-sonnet-4-5',usage:{
    input_tokens:80,output_tokens:12,cache_read_input_tokens:40,cache_creation_input_tokens:5}}}) + '\\n');
` + claude.slice(transcriptEnd);
        await writeFile(join(bin, "claude"), claude);
        await chmod(join(bin, "claude"), 0o755);
        execFileSync(process.execPath, ["--check", join(bin, "claude")]);
        // All five real children need measured transcript data before a fleet total is known.
        provider = provider.replace("// Like the real provider,", `
for (const [index, age] of [60000, 0].entries()) fs.appendFileSync(rollout, JSON.stringify({
  timestamp:new Date(Date.now()-age).toISOString(),type:'event_msg',payload:{type:'token_count',info:{
    total_token_usage:{input_tokens:100*(index+1),cached_input_tokens:20*(index+1),output_tokens:10*(index+1)},
    last_token_usage:{input_tokens:100,cached_input_tokens:20,output_tokens:10},model_context_window:1000}}
}) + '\\n');
// Like the real provider,`);
        await writeFile(join(bin, "codex"), provider);
        execFileSync(process.execPath, ["--check", join(bin, "codex")]);
      },
    });
    const errors = capture(page);
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const main = page.getByRole("region", { name: "Main pane", exact: true });
    const rows = main.locator(".works-surface:not(.works-surface--preview) .work-row");
    let workspace: any = null, activity: any = null, usage: any = null;
    const activityObservations: any[] = [];
    // Observe the real wire independently of the bridge's bounded event log.
    page.on("websocket", socket => {
      if (!new URL(socket.url()).searchParams.has("repo_hash")) return;
      socket.on("framereceived", ({ payload }) => {
        try {
          const event = JSON.parse(String(payload));
          if (event.kind === "workspace_state") workspace = event.workspace;
          const snapshot = event.kind === "local_agent_activity" ? event.activity : event.projection?.local_agent_activity;
          if (snapshot) activityObservations.push({ kind: event.kind, activity: snapshot });
          if (snapshot && Number.isFinite(Date.parse(snapshot.observed_at))
            && (!activity || Date.parse(snapshot.observed_at) >= Date.parse(activity.observed_at))) activity = snapshot;
          if (event.kind === "provider_usage") usage = event.sessions;
        }
        catch { /* Native binary terminal frames are unrelated to membership. */ }
      });
    });
    const windows = async () => workspace?.tabs.flatMap((tab: any) => tab.workspace.windows)
      .filter((window: any) => window.session_id) ?? [];
    const input = (id: string, command: string) => sendLiveGwtEvent(page, { kind: "terminal_input", id, data: command + "\r" });
    const assertAggregates = async () => {
      await expect.poll(() => usage?.filter((session: any) => session.state.kind === "ok"
        && session.telemetry?.quality === "measured").length, { timeout: 45_000 }).toBe(5);
      const format = (value: number) => new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value);
      await expect.poll(async () => {
        const sum = (key: string) => usage.reduce((total: number, session: any) => total + session.telemetry[key], 0);
        const input = sum("today_input_tokens"), output = sum("today_output_tokens");
        const cache = sum("today_cache_read_tokens") + usage.filter((session: any) => session.provider === "claude_code")
          .reduce((total: number, session: any) => total + session.telemetry.today_cache_creation_tokens, 0);
        const buckets = activity?.agents.map((agent: any) => usage.find((session: any) => session.session_id === agent.session_id)
          ?.telemetry.minute_buckets.at(-1)) ?? [];
        if (buckets.length !== 5 || buckets.some((bucket: any) => !Number.isFinite(bucket?.tokens))
          || new Set(buckets.map((bucket: any) => bucket.started_at)).size !== 1) return false;
        const expected = { "today-total": input + output, "today-input": input, "today-cache": cache,
          "today-output": output, "fleet-now": buckets.reduce((total: number, bucket: any) => total + bucket.tokens, 0) };
        for (const [metric, value] of Object.entries(expected)) {
          if (await main.locator('[data-metric="' + metric + '"]').textContent() !== format(value)) return false;
        }
        return true;
      }, { timeout: 45_000, message: "Today and Fleet match all five measured wire conversations" }).toBe(true);
      await expect(main.locator('[data-metric="today-cost"]')).toHaveText("—");
      await expect(main.locator('[data-metric="today-landed"]')).toHaveText("—");
      await expect.poll(async () => Number(await main.locator(".works-fleet svg").getAttribute("data-known-count"))).toBeGreaterThan(0);
    };
    try {
      await page.setViewportSize({ width: 1600, height: 1500 });
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true, suppressProjectSurfaces: true });
      await sendLiveGwtEvent(page, { kind: "open_pm_agent" });
      await expect.poll(async () => (await windows()).filter((window: any) => window.is_pm).length, { timeout: 90_000 }).toBe(1);
      for (let i = 1; i <= 4; i += 1) {
        await launchCodex(page, `worker-${i}`);
        await expect.poll(async () => (await windows()).filter((window: any) => window.agent_id === "codex").length,
          { timeout: 60_000 }).toBe(i);
      }
      await expect.poll(async () => (await windows()).length, { timeout: 60_000 }).toBe(5);
      const live = await windows(), pm = live.find((window: any) => window.is_pm);
      expect(live).toHaveLength(5);
      const pmName = pm.agent_name;
      expect(typeof pmName === "string" && pmName.trim().length > 0).toBe(true);
      await expect.poll(async () => (await fixture.launches()).length).toBe(5);
      for (const worker of live.filter((window: any) => !window.is_pm)) await input(worker.id, "GWT_FIXTURE_BEGIN");
      await input(pm.id, "GWT_FIXTURE_PM_BEGIN");
      await page.locator('.op-rail__surface[data-surface="works"]').click();
      await expect(rows).toHaveCount(5, { timeout: 60_000 });
      await expect(rows.last()).toBeInViewport({ ratio: 1 });
      expect(await rows.evaluateAll(nodes => nodes.map(node => (node as HTMLElement).dataset.windowId)))
        .toEqual([pm.id, ...live.filter((window: any) => !window.is_pm).map((window: any) => window.id)]);
      const first = rows.first();
      await expect(first.locator(".work-name")).toHaveText(pmName);
      await expect(first.locator(".work-name-host .window-role-badge")).toHaveText("PM");
      await expect(first.locator(".work-purpose")).toHaveText(purpose, { timeout: 30_000 });
      await expect(first.locator(".work-identity")).toHaveText("PM · claude");
      await expect(first).toHaveAttribute("data-state", "running");
      await expect(main.locator('[data-count="running"]')).toHaveText("5");
      await expect(main.locator(".works-active")).toHaveText("Active 5");
      await expect(first.locator(".work-model")).toHaveText("claude-sonnet-4-5", { timeout: 45_000 });
      for (const metric of ["input", "cache", "output"]) await expect(first.locator(`[data-stat="${metric}"]`)).toHaveText(/^[1-9]/);
      await assertAggregates();
      await expect.poll(async () => Number(await first.locator("svg").getAttribute("data-known-count"))).toBeGreaterThan(0);
      await expect.poll(() => first.locator("svg circle").evaluateAll(nodes => nodes.some(node => Number(node.getAttribute("cy")) < 53))).toBe(true);
      const pmWorktree = tomlString(await sessionRecord(fixture.home, pm.session_id), "worktree_path");
      expect(pmWorktree).toBeTruthy();
      const pmBranch = execFileSync("git", ["-C", pmWorktree!, "branch", "--show-current"], { encoding: "utf8" }).trim();
      await expect(first.locator('[data-stat="branch"]')).toHaveText(pmBranch);
      for (const worker of live.filter((window: any) => !window.is_pm)) {
        const launch = (await fixture.launches()).find(entry => entry.gwt_session_id === worker.session_id)!;
        const branch = execFileSync("git", ["-C", launch.cwd, "branch", "--show-current"], { encoding: "utf8" }).trim();
        await expect(main.locator(`.work-row[data-window-id="${worker.id}"] [data-stat="branch"]`)).toHaveText(branch);
      }
      await page.screenshot({ path: info.outputPath("works-live-five.png") });
      await input(pm.id, "GWT_FIXTURE_IDLE");
      await expect(first).toHaveAttribute("data-state", "idle");
      await input(pm.id, "GWT_FIXTURE_PM_BEGIN");
      await expect(first).toHaveAttribute("data-state", "running");
      usage = null; activity = null;
      await page.reload();
      await expect(rows).toHaveCount(5, { timeout: 60_000 });
      await expect(rows.first().locator(".work-name")).toHaveText(pmName);
      await expect(rows.first().locator(".work-name-host .window-role-badge")).toHaveText("PM");
      await expect(rows.first().locator(".work-purpose")).toHaveText(purpose);
      await expect(rows.first().locator(".work-identity")).toHaveText("PM · claude");
      await expect(rows.first()).toHaveAttribute("data-state", "running");
      await expect(main.locator(".works-active")).toHaveText("Active 5");
      await assertAggregates();
      const restarted = await fixture.restart(); workspace = null; usage = null; activity = null;
      await gotoLiveGwt(page, restarted, { enableTestBridge: true, suppressProjectSurfaces: true });
      await page.locator('.op-rail__surface[data-surface="works"]').click();
      await expect(rows).toHaveCount(5, { timeout: 90_000 });
      await expect(rows.first().locator(".work-name")).toHaveText(pmName);
      await expect(rows.first().locator(".work-name-host .window-role-badge")).toHaveText("PM");
      await expect(rows.first().locator(".work-purpose")).toHaveText(purpose);
      await expect(rows.first().locator(".work-identity")).toHaveText("PM · claude");
      await expect(rows.first()).toHaveAttribute("data-state", "idle");
      await expect(rows.first().locator(".work-model")).toHaveText("claude-sonnet-4-5", { timeout: 45_000 });
      for (const metric of ["input", "cache", "output"]) await expect(rows.first().locator(`[data-stat="${metric}"]`)).toHaveText(/^[1-9]/);
      await expect.poll(async () => Number(await rows.first().locator("svg").getAttribute("data-known-count"))).toBeGreaterThan(0);
      await expect.poll(() => rows.first().locator("svg circle").evaluateAll(nodes => nodes.some(node => Number(node.getAttribute("cy")) < 53))).toBe(true);
      await expect(rows.first().locator('[data-stat="branch"]')).toHaveText(pmBranch);
      await assertAggregates();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      expect(errors).toEqual([]);
      await page.screenshot({ path: info.outputPath("works-live-five-reloaded.png") });
      const images: { name: string; image: string }[] = [];
      for (const surface of ["pm", "agents", "works"]) {
        if (surface === "pm") await page.locator("#op-pm-entry").click();
        else await page.locator(`.op-rail__surface[data-surface="${surface}"]`).click();
        await expect(main).toHaveAttribute("data-surface", surface);
        if (surface === "pm") {
          const header = main.locator('.workspace-window[data-pm="true"]');
          await expect(header.locator(".title-text")).toHaveText(pmName);
          await expect(header.locator(".window-role-badge")).toHaveText("PM");
        } else if (surface === "agents") {
          await expect(main.locator(".roster-pm-identity .roster-pm-name")).toHaveText(pmName);
          await expect(main.locator(".roster-pm-identity .window-role-badge")).toHaveText("PM");
          await expect(main.locator(".roster-list .roster-row")).toHaveCount(4);
          await expect(main.locator(".roster-count")).toHaveText("4");
        } else {
          await expect(rows).toHaveCount(5);
          await expect(rows.first().locator(".work-name")).toHaveText(pmName);
          await expect(rows.first().locator(".work-name-host .window-role-badge")).toHaveText("PM");
        }
        images.push({ name: surface, image: (await page.screenshot({ path: info.outputPath(`pm-identity-${surface}.png`) })).toString("base64") });
        if (surface !== "pm") {
          await page.setViewportSize({ width: 400, height: 1000 });
          expect(await main.evaluate(node => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
          expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
          await page.screenshot({ path: info.outputPath(`native-${surface}-400.png`) });
          await page.setViewportSize({ width: 1600, height: 1500 });
        }
      }
      const sheet = await page.context().newPage();
      try {
        await sheet.setViewportSize({ width: 4832, height: 1540 });
        await sheet.setContent(`<main style="display:flex;gap:16px">${images.map(({ name, image }) =>
          `<figure style="margin:0"><figcaption>${name} · ${theme} · same isolated GUI</figcaption><img width="1600" height="1500" src="data:image/png;base64,${image}"></figure>`).join("")}</main>`);
        await sheet.screenshot({ path: info.outputPath("pm-identity-three-surfaces.png"), fullPage: true });
      } finally { await sheet.close(); }
      expect(errors).toEqual([]);
      const current = await windows(), sessions = new Set(current.map((window: any) => window.session_id));
      const children = [...new Map((await fixture.launches()).filter(launch => sessions.has(launch.gwt_session_id))
        .map(launch => [launch.gwt_session_id, launch])).values()];
      expect(children).toHaveLength(5);
      for (const child of children) expect(() => process.kill(child.pid, 0)).not.toThrow();
      await info.attach("real-child-membership", { body: JSON.stringify({ windows: current, children }), contentType: "application/json" });
    } finally {
      try {
        await info.attach("works-source-state", { body: JSON.stringify({ workspace, activity, usage, activityObservations }), contentType: "application/json" });
      } finally { await fixture.stop(); }
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
      const savedCandidate = { agent_member_id: memberId, agent_id: "codex", model: "gpt-6-sol", reasoning: "medium" };
      const beforeCandidateSave = await cursor(page);
      await sendLiveGwtEvent(page, { kind: "issue_monitor_profiles_set", request_id: 477743, profiles: [savedCandidate] });
      const saved = await page.waitForFunction(after => ((window as any).__gwtPlaywrightMessages || []).find((entry: any) =>
        entry.sequence > after && entry.payload.request_id === 477743
        && ["issue_monitor_profiles", "issue_monitor_profiles_write_failed"].includes(entry.payload.kind))?.payload,
      beforeCandidateSave).then(handle => handle.jsonValue());
      expect(saved.kind, saved.message).toBe("issue_monitor_profiles");
      expect(saved.profiles).toContainEqual(expect.objectContaining(savedCandidate));
      const identities = (roster: AgentRoster) => roster.members.map(member => ({
        agent_member_id: member.agent_member_id, agent_id: member.agent_id, name: member.name, enabled: member.enabled,
      })).sort((left, right) => left.agent_member_id.localeCompare(right.agent_member_id));
      const assertCapacity = async (roster: AgentRoster) => {
        expect(roster.error).toBeFalsy();
        expect(roster.capacity).toBe(roster.members.filter(member => member.enabled).length);
        const preferences = join(fixture.home, ".gwt", "projects", roster.project_key, "project-state", "issue-monitor.json");
        await expect.poll(async () => JSON.parse(await readFile(preferences, "utf8")).max_active_agents,
          { timeout: 30_000, message: "persisted max_active equals the latest roster capacity" }).toBe(roster.capacity);
        expect(JSON.parse(await readFile(preferences, "utf8")).launch_profiles,
          "selected member, fixed tool and model settings survive native restart").toContainEqual(expect.objectContaining(savedCandidate));
      };
      const checkMemberLinks = async (target: Page, currentWindow: AgentWindow, sessionId: string, stage: string) => {
        const roster = await readRoster(target);
        await assertCapacity(roster);
        const member = roster.members.find(member => member.agent_member_id === memberId)!;
        expect(member).toMatchObject({ name: memberName, agent_id: "codex", enabled: true });
        expect(currentWindow.agent_id).toBe("codex");
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
      await expect(row).toHaveCount(1);
      if (await page.locator(".works-surface:not(.works-surface--preview) .works-full:not([hidden])").isVisible())
        await page.locator('.works-surface:not(.works-surface--preview) [data-action="back"]').click();
      await expect(row.locator(".terminal-text-preview")).toBeVisible();
      await row.getByRole("button", { name: "Open terminal", exact: true }).click();
      await expect(page.locator(`.works-full .works-terminal[data-window-id="${firstWindow.id}"] .xterm-helper-textarea`)).toBeVisible();
      await attachScreenshot(page, testInfo, `launched-${theme}`);
      const initialRoster = await checkMemberLinks(page, firstWindow, first.gwt_session_id!, "initial");
      await page.getByRole("button", { name: "Agents", exact: true }).click();
      const enabledBefore = new Set(initialRoster.members.filter(member => member.enabled).map(member => member.agent_member_id));
      await page.getByRole("combobox", { name: "Tool for new member", exact: true }).selectOption("codex");
      await page.getByLabel("Name for new member", { exact: true }).fill("Relaunch helper");
      const beforeAdd = await cursor(page);
      await page.locator('.roster-capacity [data-action="add"]').click();
      const added = await rosterAfter(page, beforeAdd, initialRoster.revision);
      expect(added.capacity).toBe(initialRoster.capacity + 1); await assertCapacity(added);
      await expect(page.locator(".roster-count")).toHaveText(String(added.capacity));
      const addedMember = added.members.find(member => member.enabled && !enabledBefore.has(member.agent_member_id))!;
      expect(addedMember?.name.trim()).toBeTruthy();
      expect(addedMember.agent_id).toBe("codex");
      await page.locator(`.roster-row[data-member-id="${addedMember.agent_member_id}"]`).click();
      const beforeRest = await cursor(page);
      await page.locator('.roster-detail [data-action="rest"]').click();
      const rested = await rosterAfter(page, beforeRest, added.revision);
      expect(rested.capacity).toBe(initialRoster.capacity); await assertCapacity(rested);
      await expect(page.locator(".roster-count")).toHaveText(String(rested.capacity));
      expect(rested.members.find(member => member.agent_member_id === addedMember.agent_member_id))
        .toMatchObject({ name: addedMember.name, agent_id: "codex", enabled: false });
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
      const third = await launchAt(3);
      expect(third.argv.slice(third.argv.indexOf("resume"))).toContain(native);
      expect(third.resume_session_id).toBe(native);
      expect(third.provider_session_id).toBe(native);
      const thirdWindow = await liveCodexWindow(restored, third.gwt_session_id!);
      await expect(restored.locator(`.work-row[data-window-id="${thirdWindow.id}"]`)).toHaveCount(1);
      if (await restored.locator(".works-surface:not(.works-surface--preview) .works-full:not([hidden])").isVisible())
        await restored.locator('.works-surface:not(.works-surface--preview) [data-action="back"]').click();
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
        .toMatchObject({ name: addedMember.name, agent_id: "codex", enabled: false });
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
              .filter((entry: any) => ["workspace_state", "local_agent_activity", "agent_roster"].includes(entry.payload.kind) || entry.payload.projection?.local_agent_activity)
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
