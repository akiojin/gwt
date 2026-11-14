/* Issue #3932: real checkout/backend/PTY delivery in both Operator themes.
 * Only Codex and gh are fixture executables. Each run owns a throwaway project
 * and fresh HOME; the actual launched fake Session becomes the registered PM.
 */
import { spawnSync } from "node:child_process";
import { readFile, rename, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { startExactRelaunchFixture, type ExactRelaunchInvocation } from "./_helpers/exact-relaunch";
import { gotoLiveGwt, openLiveLaunchWizardForBranch, sendLiveGwtEvent } from "./_helpers/live-gwt";

type InputReceipt = { session: string; line: string; at: string };
type HookReceipt = { session: string; sequence: number; event: string; ok: boolean; at: string };

async function jsonLines<T>(path: string): Promise<T[]> {
  return (await readFile(path, "utf8").catch(() => "")).split("\n").filter(Boolean).map(line => JSON.parse(line));
}

async function cursor(page: Page): Promise<number> {
  return page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
}

async function wizardState(page: Page, after: number, requireOpen = false) {
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
  return wizardState(page, after);
}

async function launchCodex(page: Page, branch: string) {
  const after = await cursor(page);
  await openLiveLaunchWizardForBranch(page, branch);
  await wizardState(page, after, true);
  await wizardAction(page, { kind: "set_launch_path", path: "manual_setup" });
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

async function waitForSessionState(page: Page, session: string, expected: string) {
  await page.waitForFunction(({ session, expected }) => {
    const messages = (window as any).__gwtPlaywrightMessages ?? [];
    const workspace = messages.findLast((entry: any) => entry.payload.kind === "workspace_state")?.payload.workspace;
    const pane = (workspace?.tabs ?? []).flatMap((tab: any) => tab.workspace.windows)
      .find((entry: any) => entry.session_id === session);
    if (!pane) return false;
    const state = messages.findLast((entry: any) => entry.payload.kind === "window_state"
      && entry.payload.window_id === pane.id)?.payload.state ?? pane.status;
    return state === expected;
  }, { session, expected }, { timeout: 60_000 });
}

test.describe("Agent idle PM wake (isolated checkout)", () => {
  test.skip(process.env.GWT_PLAYWRIGHT_AGENT_IDLE_WAKE !== "1",
    "GWT_PLAYWRIGHT_AGENT_IDLE_WAKE=1 is not set; checkout idle-wake E2E skipped");
  test.skip(process.platform === "win32", "the fake provider fixture requires POSIX executables");
  test.setTimeout(240_000); // The first scheduled Monitor tick is 300 seconds.
  test.use({ viewport: { width: 1440, height: 1000 } });

  test("running to idle reaches the PM before a tick and dedupes the pane", async ({ page }, testInfo) => {
    let preferences = "";
    let inputLog = "";
    let hookLog = "";
    const theme = testInfo.project.name.endsWith("light") ? "light" : "dark";
    const fixture = await startExactRelaunchFixture(testInfo, {
      async prepare({ home, bin, project }) {
        inputLog = join(home, "provider-input.jsonl");
        hookLog = join(home, "provider-hooks.jsonl");
        // Extend the existing recorder rather than inventing a second launch
        // fixture. Commands exercise the worktree's actual generated hooks.
        const path = join(bin, "codex");
        const source = await readFile(path, "utf8");
        const marker = "readline.createInterface({input:process.stdin}).on('line', line => {";
        expect(source).toContain(marker);
        const control = `
let sequence = 0;
setInterval(() => {
  const control = path.join(process.env.HOME, 'hook-' + process.env.GWT_SESSION_ID + '.json');
  if (!fs.existsSync(control)) return;
  const command = JSON.parse(fs.readFileSync(control, 'utf8'));
  if (command.sequence <= sequence) return;
  sequence = command.sequence;
  const event = command.event;
  const hooks = JSON.parse(fs.readFileSync(path.join(process.cwd(), '.codex', 'hooks.json'), 'utf8'));
  const groups = hooks.hooks?.[event] ?? [];
  let ok = groups.length > 0;
  const input = JSON.stringify({session_id:native,hook_event_name:event,source:'startup',cwd:process.cwd(),transcript_path:rollout,prompt:'fixture turn',tool_name:'Bash',tool_input:{command:'true'}});
  for (const group of groups) for (const hook of group.hooks ?? []) {
    if (hook.type === 'command') {
      const result = require('node:child_process').spawnSync('/bin/sh', ['-c', hook.command], {input, stdio:['pipe','ignore','inherit'], timeout:60000});
      ok = ok && result.status === 0;
    }
  }
  fs.appendFileSync(${JSON.stringify(hookLog)}, JSON.stringify({session:process.env.GWT_SESSION_ID,sequence,event,ok,at:new Date().toISOString()}) + '\\n');
}, 100);
${marker}
  fs.appendFileSync(${JSON.stringify(inputLog)}, JSON.stringify({session:process.env.GWT_SESSION_ID,line,at:new Date().toISOString()}) + '\\n');`;
        // Child-only executable writes avoid leaking writable FDs into gwt.
        const result = spawnSync(process.execPath, ["-e",
          "require('node:fs').writeFileSync(process.argv[1], require('node:fs').readFileSync(0));", path],
        { input: source.replace(marker, control), encoding: "utf8" });
        expect(result.status, result.stderr).toBe(0);
        // Any background GitHub probe remains entirely local to this fixture.
        await writeFile(join(bin, "gh"), `#!${process.execPath}\nconsole.log(process.argv.includes('--version') ? 'gh version 2.80.0' : '[]');\n`, { mode: 0o755 });
        // A second launch on one branch focuses its existing Session. Only
        // this throwaway project receives the second branch for the Agent.
        const branch = spawnSync("git", ["branch", "fixture-idle-agent"], { cwd: project, encoding: "utf8" });
        expect(branch.status, branch.stderr).toBe(0);
      },
      async beforeStart(context) {
        preferences = context.preferences;
        await writeFile(join(preferences, "pm.json"), JSON.stringify({
          settings: { auto_start: false, loop_interval_secs: 10 },
        }));
      },
    });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const launches: ExactRelaunchInvocation[] = [];
    const wakeReceipts = async (session: string) => (await jsonLines<InputReceipt>(inputLog))
      .filter(receipt => receipt.session === session && receipt.line.includes("[gwt] Agent idle:"));
    let hookSequence = 0;
    const hook = async (session: string, event: string) => {
      const sequence = ++hookSequence;
      const path = join(fixture.home, `hook-${session}.json`);
      await writeFile(`${path}.next`, JSON.stringify({ sequence, event }));
      await rename(`${path}.next`, path);
      await expect.poll(async () => (await jsonLines<HookReceipt>(hookLog))
        .find(receipt => receipt.session === session && receipt.sequence === sequence),
      { timeout: 75_000, message: `${event} generated hook receipt` }).toMatchObject({ ok: true });
    };
    try {
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      for (let index = 0; index < 2; index += 1) {
        await launchCodex(page, index === 0 ? fixture.branch : "fixture-idle-agent");
        await expect.poll(async () => (await fixture.launches()).length, { timeout: 75_000 }).toBe(index + 1);
        launches.push((await fixture.launches())[index]);
        expect(launches[index].gwt_session_id).toBeTruthy();
        await waitForSessionState(page, launches[index].gwt_session_id!, "idle");
      }
      const pm = launches[0].gwt_session_id!;
      const agent = launches[1].gwt_session_id!;
      // Register only a Session actually launched by this isolated backend.
      await writeFile(join(preferences, "pm.json"), JSON.stringify({
        registration: { session_id: pm, agent_id: "codex", worktree_path: fixture.project },
        settings: { auto_start: false, loop_interval_secs: 10 },
      }));
      const monitor = JSON.parse(await readFile(join(preferences, "issue-monitor.json"), "utf8"));
      await writeFile(join(preferences, "issue-monitor.json"), JSON.stringify({ ...monitor, enabled: true }));
      await writeFile(join(preferences, "pm-loop.json"), "{}");
      expect(await wakeReceipts(pm), "initial SessionStart is not an idle edge").toHaveLength(0);

      await hook(agent, "UserPromptSubmit");
      await waitForSessionState(page, agent, "running");
      const edgeAt = Date.now();
      await hook(agent, "SessionStart");
      await waitForSessionState(page, agent, "idle");
      await expect.poll(async () => (await wakeReceipts(pm)).length,
        { timeout: 30_000, message: "idle edge reaches the actual PM PTY without a Monitor tick" }).toBe(1);
      expect(Date.now() - edgeAt).toBeLessThan(30_000);

      await hook(agent, "SessionStart");
      // Cross the 10-second quiet floor: a duplicate accidentally retained in
      // pending must not become another wake when the clock opens again.
      await page.waitForTimeout(12_000);
      expect(await wakeReceipts(pm), "continuous idle is delivered once").toHaveLength(1);
      await hook(agent, "UserPromptSubmit");
      await waitForSessionState(page, agent, "running");
      await hook(agent, "SessionStart");
      await waitForSessionState(page, agent, "idle");
      await expect.poll(async () => (await wakeReceipts(pm)).length, { timeout: 30_000 }).toBe(2);
      expect((await fixture.launches()).map(launch => launch.pid)).toEqual(launches.map(launch => launch.pid));
      expect(errors, "console/page errors").toEqual([]);
      await testInfo.attach(`agent-idle-wake-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
    } finally {
      try {
        await testInfo.attach("agent-idle-wake-evidence", { body: JSON.stringify({
          home: fixture.home, preferences, inputLog, hookLog, theme, errors,
          launches: await fixture.launches(), inputs: await jsonLines<InputReceipt>(inputLog),
          hooks: await jsonLines<HookReceipt>(hookLog),
        }, null, 2), contentType: "application/json" });
      } finally {
        await fixture.stop();
        await expect.poll(() => launches.every(launch =>
          spawnSync("ps", ["-p", String(launch.pid), "-o", "pid="], { encoding: "utf8" }).status === 1),
        { timeout: 10_000, message: "all owned fake provider processes have exited" }).toBe(true);
      }
    }
  });
});
