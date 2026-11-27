/** Issue #7015: real update-quiescence observations ignore saved pane remnants.
 * Cold startup normalizes saved Starting rows to Stopped, so this complements
 * the Rust synchronous-launch-failure regression; it is not its RED proof.
 * No staged installer, update apply, or synthetic backend event is used here.
 */
import { randomUUID } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { startExactRelaunchFixture, writeFixtureNodeCommand } from "./_helpers/exact-relaunch";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

type Blocker = { kind: string; window_id?: string; state?: string };
type RestartImpact = { blocking: Blocker[]; error?: string | null };

async function restartImpact(page: Page): Promise<RestartImpact> {
  const requestId = randomUUID();
  await sendLiveGwtEvent(page, { kind: "get_update_restart_impact", request_id: requestId });
  const result = await page.waitForFunction(requestId =>
    ((window as any).__gwtPlaywrightMessages ?? []).findLast(({ payload }: any) =>
      payload.kind === "update_restart_impact" && payload.request_id === requestId)?.payload,
  requestId, { timeout: 30_000 }).then(handle => handle.jsonValue());
  expect(result.error, "real backend can inspect update blockers").toBeFalsy();
  return result;
}

async function windows(page: Page): Promise<any[]> {
  const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
  await sendLiveGwtEvent(page, { kind: "list_windows" });
  return page.waitForFunction(after =>
    ((window as any).__gwtPlaywrightMessages ?? []).findLast((entry: any) =>
      entry.sequence > after && entry.payload.kind === "window_list")?.payload.windows,
  after, { timeout: 30_000 }).then(handle => handle.jsonValue());
}

test.use({ headless: false, viewport: { width: 1440, height: 1000 } });
test.skip(process.env.GWT_PLAYWRIGHT_UPDATE_DRAIN_GHOST !== "1",
  "requires freshly built checkout gwt and gwtd binaries");

test("Issue #7015: only the real busy provider blocks update drain", async ({ page }, info) => {
  test.setTimeout(180_000);
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const ghosts = Array.from({ length: 9 }, (_, index) => `agent-ghost-${index + 1}`);
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const fixture = await startExactRelaunchFixture(info, {
    async prepare({ bin }) {
      const providerPath = join(bin, process.platform === "win32" ? "codex.cjs" : "codex");
      const provider = await readFile(providerPath, "utf8");
      const marker = "  if (line.trim() === 'GWT_FIXTURE_EXIT')";
      expect(provider).toContain(marker);
      // The real provider actor publishes its own ordinary generated hook.
      // The test runner never writes Session/runtime state or a hook receipt.
      const busy = String.raw`
  if (line.trim() === 'GWT_FIXTURE_BUSY') {
    const hooks = JSON.parse(fs.readFileSync(path.join(process.cwd(), '.codex/hooks.json'), 'utf8'));
    const input = JSON.stringify({session_id:native,hook_event_name:'UserPromptSubmit',
      cwd:process.cwd(),transcript_path:rollout,prompt:line,stop_hook_active:false});
    let commands = 0;
    for (const group of hooks.hooks?.UserPromptSubmit ?? []) for (const hook of group.hooks ?? []) {
      if (hook.type !== 'command') continue;
      const windows = process.platform === 'win32';
      const result = require('node:child_process').spawnSync(windows ? (process.env.ComSpec || 'cmd.exe') : '/bin/sh',
        windows ? ['/d','/s','/c',hook.command] : ['-c',hook.command],
        {input,cwd:process.cwd(),env:process.env,encoding:'utf8',timeout:60000,
          windowsHide:true,windowsVerbatimArguments:windows});
      if (result.error || result.status !== 0 || /"decision"\s*:\s*"block"/.test(result.stdout || ''))
        throw new Error('ordinary busy hook refused');
      commands++;
    }
    if (!commands) throw new Error('ordinary busy hook is missing');
    console.log('GWT_FIXTURE_BUSY_READY');
    return;
  }
`;
      await writeFixtureNodeCommand(bin, "codex", provider.replace(marker, busy + marker));
    },
    async beforeStart({ preferences }) {
      // Only saved presentation is fabricated: no Session, PID, owner pane,
      // execution ledger, launch capability, or delivery acknowledgement.
      await writeFile(join(dirname(preferences), "workspace.json"), JSON.stringify({
        windows: ghosts.map((id, index) => ({ id, title: `Saved Agent ${index + 1}`,
          preset: "agent", status: "starting", z_index: index + 1, agent_id: "codex",
          dynamic_title_detail: "Saved presentation without a native process" })),
        next_z_index: ghosts.length + 1,
      }));
    },
  });
  const observations: Record<string, unknown> = { theme, isolated_home: fixture.home, project: fixture.project };
  try {
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
    await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const saved = (await windows(page)).filter(window => ghosts.some(id => window.id.endsWith(`::${id}`)));
    expect(saved, "all saved remnants remain present for this observation").toHaveLength(ghosts.length);
    expect(saved.every(window => window.status === "stopped"), "cold restore pauses saved process rows").toBe(true);
    expect(saved.every(window => !window.session_id && !window.runtime_started_at_ms)).toBe(true);
    expect(await fixture.launches(), "saved rows do not launch a native provider").toEqual([]);
    const before = await restartImpact(page);
    expect(before.blocking.filter(blocker => blocker.kind === "active_pane")).toEqual([]);
    Object.assign(observations, { saved_windows: saved, before });

    await sendLiveGwtEvent(page, { kind: "create_window", preset: "codex",
      bounds: { x: 64, y: 64, width: 900, height: 520 } });
    await expect.poll(async () => (await fixture.launches()).length, { timeout: 60_000 }).toBe(1);
    const provider = (await fixture.launches())[0];
    expect(provider.pid).toBeGreaterThan(0);
    expect(() => process.kill(provider.pid, 0), "the native provider PID exists").not.toThrow();
    let live: any;
    await expect.poll(async () => {
      live = (await windows(page)).find(window => window.session_id === provider.gwt_session_id
        && ["idle", "running"].includes(window.status));
      return !!live;
    }, { timeout: 60_000 }).toBe(true);
    expect(ghosts.some(id => live.id.endsWith(`::${id}`))).toBe(false);
    await sendLiveGwtEvent(page, { kind: "terminal_input", id: live.id, data: "GWT_FIXTURE_BUSY\r" });
    let busy: RestartImpact;
    await expect.poll(async () => {
      busy = await restartImpact(page);
      return busy.blocking.filter(blocker => blocker.kind === "active_pane");
    }, { timeout: 60_000 }).toEqual([expect.objectContaining({ window_id: live.id, state: "running" })]);
    Object.assign(observations, { provider, live_window: live, busy: busy! });
    await info.attach(`update-drain-real-blocker-${theme}`, {
      body: await page.screenshot({ fullPage: true }), contentType: "image/png",
    });

    await sendLiveGwtEvent(page, { kind: "terminal_input", id: live.id, data: "GWT_FIXTURE_EXIT\r" });
    await expect.poll(() => {
      try { process.kill(provider.pid, 0); return false; } catch { return true; }
    }, { timeout: 30_000, message: "the fixture's exact provider process exits" }).toBe(true);
    let after: RestartImpact;
    await expect.poll(async () => {
      after = await restartImpact(page);
      return after.blocking.filter(blocker => blocker.kind === "active_pane");
    }, { timeout: 30_000 }).toEqual([]);
    Object.assign(observations, { after: after! });
    await info.attach(`update-drain-no-ghost-blocker-${theme}`, {
      body: await page.screenshot({ fullPage: true }), contentType: "image/png",
    });
    expect(errors, "console and page errors").toEqual([]);
  } finally {
    try { await page.close(); } finally {
      try { await fixture.stop(); } finally {
        await info.attach(`update-drain-ghost-observations-${theme}`, {
          body: JSON.stringify({ ...observations, errors }), contentType: "application/json",
        });
      }
    }
  }
});
