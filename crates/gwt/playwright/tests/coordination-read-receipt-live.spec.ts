/* Issue #3713: explicit Board observation gates agent lifecycle requests.
 * The checkout GUI, daemon, Sessions, PTYs, and Board storage are real. Only
 * the provider executable is replaced with a command recorder that invokes
 * gwtd with the identity and capability supplied by the actual launch.
 */
import { spawnSync } from "node:child_process";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { startExactRelaunchFixture } from "./_helpers/exact-relaunch";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

type AgentWindow = { id: string; session_id: string; status: string };
type Envelope = { ok: boolean; output?: string; error?: unknown };

test.use({ headless: false, viewport: { width: 1440, height: 1000 } });

async function cursor(page: Page): Promise<number> {
  return page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
}

async function wizardState(page: Page, after: number, requireOpen = false) {
  return (await page.waitForFunction(({ after, requireOpen }) => {
    const entry = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
      entry.sequence > after && entry.payload.kind === "launch_wizard_state"
      && (!requireOpen || entry.payload.wizard)
      && (!entry.payload.wizard || (!entry.payload.wizard.is_hydrating
        && !entry.payload.wizard.runtime_resolution_pending
        && !entry.payload.wizard.launch_materialization_pending)));
    return entry ? { wizard: entry.payload.wizard } : null;
  }, { after, requireOpen }, { timeout: 30_000 })).jsonValue();
}

async function wizardAction(page: Page, action: unknown) {
  const after = await cursor(page);
  await sendLiveGwtEvent(page, { kind: "launch_wizard_action", action,
    bounds: { x: 80, y: 80, width: 700, height: 440 } });
  return wizardState(page, after);
}

async function launchAgent(page: Page, branch: string) {
  const after = await cursor(page);
  // Use the current Agent Kanban route and select an existing fixture branch.
  // gwt owns any necessary Launch materialization inside the isolated project.
  const owner = page.locator('.workspace-window[data-preset="agent_kanban"]').first();
  if (await owner.count() === 0) {
    await sendLiveGwtEvent(page, { kind: "create_window", preset: "agent_kanban",
      bounds: { x: 96, y: 96, width: 880, height: 520 } });
  }
  await expect(owner).toBeVisible();
  await sendLiveGwtEvent(page, { kind: "open_agent_kanban_launch_wizard",
    board_id: await owner.getAttribute("data-id"), lane_id: "active" });
  await wizardState(page, after, true);
  await wizardAction(page, { kind: "set_launch_path", path: "manual_setup" });
  await wizardAction(page, { kind: "select_existing_branch", branch_name: branch });
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

async function agentWindows(page: Page): Promise<AgentWindow[]> {
  return page.evaluate(() => {
    const state = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
      entry.payload.kind === "workspace_state");
    return (state?.payload.workspace.tabs ?? []).flatMap((tab: any) => tab.workspace.windows)
      .filter((entry: any) => entry.agent_id === "codex")
      .map(({ id, session_id, status }: any) => ({ id, session_id, status }));
  });
}

test.describe("Board read receipts (isolated checkout)", () => {
  test.skip(process.env.GWT_PLAYWRIGHT_COORDINATION_HARNESS !== "1",
    "requires GWT_PLAYWRIGHT_COORDINATION_HARNESS=1 and checkout binaries");
  test.skip(process.platform === "win32", "the provider command recorder requires POSIX executables");
  test.setTimeout(240_000);

  test("agent close preserves unread target evidence until explicit read, while human close remains available", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const screenshot = async (name: string) => {
      const path = testInfo.outputPath(`${name}.png`);
      await page.screenshot({ path });
      await testInfo.attach(name, { path, contentType: "image/png" });
    };
    const fixture = await startExactRelaunchFixture(testInfo, {
      async prepare({ home, bin, project }) {
        // Normal launch keeps one live agent per branch. Both branches are
        // seeded in the throwaway repository; the user's checkout is untouched.
        const branch = spawnSync("git", ["branch", "peer"], { cwd: project, encoding: "utf8" });
        if (branch.status !== 0) throw new Error("Could not prepare existing peer fixture branch");
        const requests = join(home, "coordination-requests");
        const results = join(home, "coordination-results");
        await Promise.all([mkdir(requests), mkdir(results)]);
        const provider = `#!${process.execPath}
const fs = require('node:fs');
const path = require('node:path');
const cp = require('node:child_process');
if (process.argv.includes('--version') || process.argv.includes('-V')) {
  console.log('codex-cli 0.116.0'); process.exit(0);
}
const session = process.env.GWT_SESSION_ID;
fs.appendFileSync(${JSON.stringify(join(home, "provider-argv.jsonl"))}, JSON.stringify({pid:process.pid,gwt_session_id:session}) + '\\n');
// Capabilities and transport credentials remain inside this launched process.
// The parent receives only a command's public JSON stdout, never its environment.
const requestPath = path.join(${JSON.stringify(requests)}, session + '.json');
let completed;
setInterval(() => {
  if (!fs.existsSync(requestPath)) return;
  const request = JSON.parse(fs.readFileSync(requestPath, 'utf8'));
  if (request.id === completed) return;
  completed = request.id;
  const reply = cp.spawnSync(process.env.GWT_BIN_PATH, [], {
    input: JSON.stringify(request.envelope), encoding:'utf8', timeout:60000,
    stdio:['pipe','pipe','ignore'], env:process.env,
  });
  const result = path.join(${JSON.stringify(results)}, request.id + '.json');
  fs.writeFileSync(result + '.tmp', reply.stdout || JSON.stringify({ok:false,error:'recorder invocation failed'}));
  fs.renameSync(result + '.tmp', result);
}, 100);
console.log('GWT_COORDINATION_RECORDER_READY');
`;
        // The writer exits before gwt spawns, so writable executable FDs
        // cannot be inherited by sibling processes (Issue #5028).
        const result = spawnSync(process.execPath, ["-e",
          "const fs=require('node:fs');fs.writeFileSync(process.argv[1],fs.readFileSync(0));fs.chmodSync(process.argv[1],0o755);",
          join(bin, "codex")], { input: provider, encoding: "utf8" });
        if (result.status !== 0) throw new Error("Could not prepare provider command recorder");
      },
    });
    let commandId = 0;
    const command = async (session: string, operation: string, params: Record<string, unknown>): Promise<Envelope> => {
      const id = `command-${++commandId}`;
      const requestPath = join(fixture.home, "coordination-requests", `${session}.json`);
      await writeFile(`${requestPath}.tmp`,
        JSON.stringify({ id, envelope: { schema_version: 1, operation, params } }));
      await rename(`${requestPath}.tmp`, requestPath);
      const resultFile = join(fixture.home, "coordination-results", `${id}.json`);
      await expect.poll(async () => readFile(resultFile, "utf8").catch(() => ""),
        { timeout: 70_000, message: `${operation} answers from launched Session` }).not.toBe("");
      const result = JSON.parse(await readFile(resultFile, "utf8"));
      await testInfo.attach(`${id}-${operation}`, {
        body: JSON.stringify(result), contentType: "application/json",
      });
      return result;
    };
    const launchedWindow = async (count: number) => {
      await expect.poll(async () => (await fixture.launches()).length,
        { timeout: 60_000, message: `provider launch #${count}` }).toBe(count);
      const session = (await fixture.launches())[count - 1].gwt_session_id!;
      expect(session).toBeTruthy();
      await expect.poll(async () => (await agentWindows(page)).find(window =>
        window.session_id === session && ["running", "idle"].includes(window.status)),
      { timeout: 60_000, message: "actual launched agent pane is running" }).toBeTruthy();
      const window = (await agentWindows(page)).find(window => window.session_id === session)!;
      await sendLiveGwtEvent(page, { kind: "undock_agent_window", id: window.id,
        geometry: { x: count === 1 ? 48 : 704, y: 100, width: 630, height: 440 } });
      return window;
    };
    try {
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      const theme = testInfo.project.name.includes("light") ? "light" : "dark";
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await launchAgent(page, fixture.branch);
      const caller = await launchedWindow(1);
      await launchAgent(page, "peer");
      const target = await launchedWindow(2);
      expect(target.session_id).not.toBe(caller.session_id);
      const targetPane = page.locator(`.workspace-window[data-id="${target.id}"]`);
      await expect(targetPane).toBeVisible();

      const evidence = "Issue #3713 live target evidence must survive an unread agent close";
      const posted = await command(target.session_id, "board.post", {
        kind: "status", body: evidence, broadcast: true,
      });
      expect(posted.ok, JSON.stringify(posted)).toBe(true);
      const refused = await command(caller.session_id, "pane.close", { id: target.id });
      expect(refused.ok).toBe(false);
      expect(JSON.stringify(refused)).toContain("board.show");
      await expect(targetPane).toBeVisible();
      await expect.poll(async () => (await agentWindows(page)).some(window => window.id === target.id)).toBe(true);
      await screenshot(`unread-target-preserved-${theme}`);

      const observed = await command(caller.session_id, "board.show", { all: true });
      expect(observed.ok, JSON.stringify(observed)).toBe(true);
      expect(observed.output).toContain(evidence);
      const closed = await command(caller.session_id, "pane.close", { id: target.id });
      expect(closed.ok, JSON.stringify(closed)).toBe(true);
      await expect(targetPane).toHaveCount(0);

      // The fixed surface has no human canvas-close escape. Explicit runtime
      // cleanup still follows the read-receipt contract through pane.close.
      const callerPane = page.locator(`.workspace-window[data-id="${caller.id}"]`);
      await expect(callerPane.locator('[data-action="close"]')).toHaveCount(0);
      await expect(page.locator("#window-close-confirm-modal")).toHaveCount(0);
      const cleanup = await command(caller.session_id, "pane.close", { id: caller.id });
      expect(cleanup.ok, JSON.stringify(cleanup)).toBe(true);
      await expect(callerPane).toHaveCount(0);
      await screenshot(`explicit-read-retry-and-fixed-surface-${theme}`);
      expect(errors, "console and page errors during lifecycle/read receipt checks").toEqual([]);
    } finally {
      await fixture.stop();
    }
  });
});
