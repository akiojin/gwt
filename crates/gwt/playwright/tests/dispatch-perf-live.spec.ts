/** Issue #5538: real checkout UI smoke; opt-in authenticated eight-agent measurement. */
import { execFileSync } from 'node:child_process';
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { performance } from 'node:perf_hooks';
import { expect, test, type Page } from '@playwright/test';
import { clearLiveLaunchWizard, gotoLiveGwt, sendLiveGwtEvent } from './_helpers/live-gwt';

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL || '';
const HOME = process.env.GWT5538_HOME || '';
const FLEET = process.env.GWT5538_FLEET === '1';
type Project = { path: string; project_key: string; branch_name: string };
type Surface = { page: Page; id: string; session_id: string; project_path: string; heartbeat_shell_pid?: number };
type ProcessIdentity = { pid: number; parent_pid: number; name: string; started_at: string };
test.use({ headless: false });
test.describe('Issue #5538 dispatch measurement (isolated live checkout)', () => {
  test.skip(!BASE || !HOME, 'requires scripts/verify-issue-5538.ps1');
  test.setTimeout(FLEET ? 2_700_000 : 120_000);
  test('headed UI stays responsive with honest dispatch evidence', async ({ page }, info) => {
    const fixture = JSON.parse(await readFile(join(HOME, 'fixture.json'), 'utf8'));
    const projects: Project[] = JSON.parse(await readFile(join(HOME, 'projects.json'), 'utf8'));
    const theme = info.project.name.includes('light') ? 'light' : 'dark';
    // The long real-fleet run occurs once. Light still exercises the headed UI.
    const measureFleet = FLEET && theme === 'dark';
    const errors: string[] = [];
    const surfaces: Surface[] = [];
    const samples: unknown[] = [];
    let observationStart: string | null = null;
    let observationEnd: string | null = null;
    let elapsedMs = 0;
    let passed = false;
    const watchErrors = (surface: Page) => {
      surface.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      surface.on('pageerror', error => errors.push(String(error)));
    };
    watchErrors(page);
    try {
      expect(projects).toHaveLength(FLEET ? 8 : 1);
      await gotoLiveGwt(page, BASE, { enableTestBridge: true, projectKey: projects[0].project_key });
      await retainWorkspaceState(page);
      await clearLiveLaunchWizard(page);
      await page.evaluate(value => { document.documentElement.dataset.theme = value; }, theme);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      await page.locator('.op-rail').getByRole('button', { name: 'Settings', exact: true }).click();
      await expect(page.locator('.split-pane[data-surface="settings"][data-active="true"] .workspace-window[data-preset="settings"]').first()).toBeVisible();
      await sendLiveGwtEvent(page, { kind: 'list_windows' });
      expect(await page.evaluate(() => document.visibilityState)).toBe('visible');
      if (!measureFleet) {
        await expect.poll(async () => (await profileRows(fixture.gui_pid, null, null)).dispatch_summary_observed,
          { timeout: 90_000, message: 'real checkout collector writes a typed minute summary, including zero-stall windows' }).toBe(true);
      }
      if (measureFleet) {
        for (const [index, project] of projects.entries()) {
          const surface = index === 0 ? page : await page.context().newPage();
          if (index) {
            watchErrors(surface);
            await gotoLiveGwt(surface, BASE, { enableTestBridge: true, projectKey: project.project_key });
            await retainWorkspaceState(surface);
            await clearLiveLaunchWizard(surface);
          }
          const launched = await launchCodex(surface, project);
          surfaces.push({ page: surface, ...launched, project_path: project.path });
          await ownedProcesses(fixture.gui_pid);
        }
        for (const launched of surfaces) {
          // Codex folds tool output in its TUI. Read only markers from actual
          // tool-result records, never model prose or the command's input.
          await expect.poll(async () => (await providerMarkers(launched.session_id)).ready, { timeout: 360_000,
            message: 'real Codex executes read-only heartbeat fixture' }).toBe(true);
          launched.heartbeat_shell_pid = (await providerMarkers(launched.session_id)).shell;
        }
        await writeFile(join(HOME, 'fleet-barrier'), 'start\n');
        for (const surface of surfaces) {
          await expect.poll(async () => (await heartbeat(surface))?.tick > 0, { timeout: 90_000 }).toBe(true);
        }
        const initial = await identities(fixture.gui_pid, surfaces);
        const progress = new Map(initial.map(identity => [identity.session_id,
          { tick: identity.heartbeat_tick, at: performance.now() }]));
        observationStart = new Date().toISOString();
        const clock = performance.now();
        let completeWindows = 0;
        do {
          const current = await identities(fixture.gui_pid, surfaces);
          expect(current.map(identity => identity.child)).toEqual(initial.map(identity => identity.child));
          for (const identity of current) {
            const previous = progress.get(identity.session_id)!;
            if (identity.heartbeat_tick > previous.tick) progress.set(identity.session_id,
              { tick: identity.heartbeat_tick, at: performance.now() });
            expect(performance.now() - progress.get(identity.session_id)!.at,
              'each real heartbeat keeps advancing').toBeLessThan(60_000);
          }
          for (const surface of surfaces) {
            await expect(surface.page.locator(`.workspace-window[data-id="${surface.id}"], .workspace-window[data-id$="::${surface.id}"]`)).toBeAttached();
            expect((await terminal(surface.page, surface.id)).length, 'real provider TUI remains mounted').toBeGreaterThan(0);
            expect((await heartbeat(surface))?.completed).toBe(false);
          }
          samples.push({ observed_at: new Date().toISOString(), elapsed_ms: performance.now() - clock,
            agent_count: current.length, agents: current });
          expect(errors).toEqual([]);
          completeWindows = (await profileRows(fixture.gui_pid, observationStart, null)).complete_window_count;
          await writeFile(join(HOME, 'fleet-progress.json'), JSON.stringify({
            observed_at: new Date().toISOString(), elapsed_ms: performance.now() - clock,
            agent_count: current.length, complete_window_count: completeWindows, agents: current,
          }, null, 2) + '\n');
          if (performance.now() - clock > 150_000) expect(completeWindows, 'collector has produced a full owned window').toBeGreaterThan(0);
          expect(performance.now() - clock, 'thirty complete collector windows arrive within 35 minutes').toBeLessThan(2_100_000);
          if (performance.now() - clock < 1_800_000 || completeWindows < 30) await page.waitForTimeout(5_000);
        } while (performance.now() - clock < 1_800_000 || completeWindows < 30);
        elapsedMs = performance.now() - clock;
        observationEnd = new Date().toISOString();
        expect(elapsedMs).toBeGreaterThanOrEqual(1_800_000);
      }
      expect(errors).toEqual([]);
      await info.attach(`dispatch-${theme}`, { body: await page.screenshot(), contentType: 'image/png' });
      passed = true;
    } finally {
      try {
      const profile = await profileRows(fixture.gui_pid, observationStart, observationEnd);
      const completed = passed;
      if (measureFleet) passed = passed && profile.complete_window_count >= 30
        && profile.dispatch_summary_rows.every((row: any) => row.dispatch.dropped_windows === 0);
      const evidence = { issue: 5538, theme, passed, fixture, workload: measureFleet
        ? '8 real builtin Codex agents executing a fixture command; 5s heartbeat and private progress receipts'
        : 'short headed UI smoke; no provider agent launched',
      observation_started_at: observationStart, observation_ended_at: observationEnd, elapsed_ms: elapsedMs,
      fleet_samples: samples, ...profile, browser_errors: errors,
      user_verification_result: 'n/a (autonomous)', agent_visual_check: passed ? 'pass' : 'fail' };
      await writeFile(join(HOME, `dispatch-${theme}.json`), JSON.stringify(evidence, null, 2) + '\n');
      await info.attach(`dispatch-evidence-${theme}`, { body: JSON.stringify(evidence), contentType: 'application/json' });
      if (measureFleet && completed) expect(passed, 'measured thirty full windows without dropped summaries').toBe(true);
      } finally {
      for (const surface of surfaces) {
        await sendLiveGwtEvent(surface.page, { kind: 'close_window', id: surface.id }).catch(() => undefined);
      }
      for (const surface of surfaces) if (surface.page !== page) await surface.page.close();
      }
    }
  });
});

async function terminal(page: Page, id: string): Promise<string> {
  return page.evaluate(id => {
    const node = document.querySelector<HTMLElement>(`.workspace-window[data-id="${id}"], .workspace-window[data-id$="::${id}"]`);
    return String((window as any).__gwtTerminalTestApi?.bufferText?.(node?.dataset.id || id) || '');
  }, id);
}

async function retainWorkspaceState(page: Page): Promise<void> {
  await page.evaluate(() => {
    const state = window as any;
    state.__gwt5538Workspace = state.__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === 'workspace_state')?.payload.workspace;
    for (const socket of state.__gwtPlaywrightSockets as WebSocket[]) socket.addEventListener('message', (event: MessageEvent) => {
      const payload = JSON.parse(String(event.data));
      if (payload.kind === 'workspace_state') state.__gwt5538Workspace = payload.workspace;
    });
  });
}

async function providerMarkers(sessionId: string): Promise<{ ready: boolean; tick: number; shell: number; done: boolean }> {
  const directory = join(HOME, '.codex', 'sessions');
  const files = (await readdir(directory, { recursive: true }).catch(() => []))
    .filter(file => file.endsWith('.jsonl'));
  const result = { ready: false, tick: 0, shell: 0, done: false };
  for (const file of files) for (const line of (await readFile(join(directory, file), 'utf8')).split('\n')) {
    let record;
    try { record = JSON.parse(line); } catch { continue; }
    const payload = record.payload;
    if (!['function_call_output', 'custom_tool_call_output'].includes(payload?.type)) continue;
    const output = JSON.stringify(payload.output);
    const ready = output.match(new RegExp(`GWT5538_READY session=${sessionId} shell=(\\d+)`));
    if (ready) { result.ready = true; result.shell = Number(ready[1]); }
    for (const marker of output.matchAll(new RegExp(`GWT5538_TICK session=${sessionId} shell=(\\d+) tick=(\\d+)`, 'g'))) {
      if (Number(marker[2]) >= result.tick) { result.shell = Number(marker[1]); result.tick = Number(marker[2]); }
    }
    if ((ready || output.includes(`GWT5538_TICK session=${sessionId} `)) && output.includes('GWT5538_DONE')) result.done = true;
  }
  return result;
}

async function heartbeat(surface: Surface): Promise<any> {
  return readFile(join(surface.project_path, 'heartbeat-progress.json'), 'utf8')
    .then(value => JSON.parse(value)).catch(error => { if (error.code === 'ENOENT') return null; throw error; });
}

async function wizard(page: Page, detail: unknown, requireOpen = false): Promise<any> {
  return page.evaluate(({ detail, requireOpen }) => {
    const socket = [...((window as any).__gwtPlaywrightSockets as WebSocket[])].reverse()
      .find(socket => socket.readyState === WebSocket.OPEN && new URL(socket.url).searchParams.has('repo_hash'));
    if (!socket) throw new Error('project socket missing');
    return new Promise((resolve, reject) => {
      const cleanup = () => { clearTimeout(timer); socket.removeEventListener('message', reply); };
      const reply = (event: MessageEvent) => {
        const payload = JSON.parse(String(event.data));
        if (payload.kind === 'launch_wizard_open_error') { cleanup(); reject(new Error(payload.message || payload.error || 'wizard open refused')); return; }
        if (payload.kind !== 'launch_wizard_state' || (requireOpen && !payload.wizard)
            || (payload.wizard && (payload.wizard.is_hydrating || payload.wizard.runtime_resolution_pending
              || payload.wizard.launch_materialization_pending))) return;
        cleanup(); resolve(payload);
      };
      const timer = setTimeout(() => { cleanup(); reject(new Error('wizard timed out')); }, 90_000);
      socket.addEventListener('message', reply);
      window.dispatchEvent(new CustomEvent('__gwt_test_send', { detail }));
    });
  }, { detail, requireOpen });
}

async function launchCodex(page: Page, project: Project): Promise<{ id: string; session_id: string }> {
  await wizard(page, { kind: 'open_active_work_launch_wizard', branch_name: project.branch_name, linked_issue_number: null }, true);
  const action = (action: unknown) => wizard(page, { kind: 'launch_wizard_action', action,
    bounds: { x: 40, y: 40, width: 650, height: 360 } });
  await action({ kind: 'set_launch_path', path: 'manual_setup' });
  await action({ kind: 'set_branch_mode', create_new: false });
  await action({ kind: 'set_agent', agent_id: 'codex' });
  await action({ kind: 'clear_linked_issue' });
  await action({ kind: 'set_skip_permissions', enabled: false });
  await action({ kind: 'set_initial_prompt', value: `This is an authorized measurement task in a disposable scratch fixture, with no implementation owner. Read README.txt and heartbeat.ps1, then execute pwsh -NoProfile -File "${project.path.replaceAll('\\', '/')}/heartbeat.ps1" and wait for it to finish (up to 40 minutes after a harness barrier). The command reads a barrier, prints every five seconds and writes only its private heartbeat-progress receipt in this fixture. Do not invoke Work/Board mutations: this manual measurement Session has no execution owner, and the heartbeat does not depend on Work authority. If pane.name is refused or times out, record the refusal once and continue the independently authorized measurement command without retrying that mutation. Do not write source, Git state, GitHub, credentials or settings; do not launch agents or perform implementation work. Keep waiting for this command; do not replace it with a shorter synthetic producer. After completion, report the observed marker only.` });
  let state = await action({ kind: 'set_runtime_target', target: 'Host' });
  for (let step = 0; step < 12 && state.wizard; step++) {
    expect(state.wizard.error).toBeFalsy();
    expect(state.wizard.holder_decision).toBeFalsy();
    expect(state.wizard.primary_action_enabled, state.wizard.primary_action_disabled_reason).toBe(true);
    state = await action({ kind: 'submit' });
  }
  expect(state.wizard).toBeNull();
  return page.waitForFunction(() => {
    const agents = (window as any).__gwt5538Workspace?.tabs.flatMap((tab: any) => tab.workspace.windows)
      .filter((pane: any) => pane.agent_id === 'codex');
    if (agents?.length !== 1 || !agents[0].session_id) return null;
    if (agents[0].status === 'error') throw new Error('Codex launch failed');
    return { id: agents[0].id, session_id: agents[0].session_id };
  }, null, { timeout: 90_000 }).then(handle => handle.jsonValue());
}

async function ownedProcesses(guiPid: number): Promise<ProcessIdentity[]> {
  // Fixed numeric PID only; never read/log process command lines or credentials.
  expect(Number.isInteger(guiPid) && guiPid > 0).toBe(true);
  const command = `Get-CimInstance Win32_Process | Where-Object {$_.CreationDate} | Select-Object @{n='pid';e={$_.ProcessId}},@{n='parent_pid';e={$_.ParentProcessId}},@{n='name';e={$_.Name}},@{n='started_at';e={$_.CreationDate.ToUniversalTime().ToString('o')}} | ConvertTo-Json -Compress -AsArray`;
  const all: ProcessIdentity[] = JSON.parse(execFileSync('pwsh', ['-NoProfile', '-Command', command], { encoding: 'utf8', timeout: 30_000 }));
  const descendants = new Set<number>([guiPid]);
  for (let changed = true; changed;) {
    changed = false;
    for (const process of all) if (descendants.has(process.parent_pid) && !descendants.has(process.pid)) {
      descendants.add(process.pid); changed = true;
    }
  }
  const owned = all.filter(process => descendants.has(process.pid) && process.pid !== guiPid);
  const receiptPath = join(HOME, 'owned-processes.json');
  const previous: ProcessIdentity[] = JSON.parse(await readFile(receiptPath, 'utf8').catch(() => '[]'));
  const receipts = new Map([...previous, ...owned].map(identity => [`${identity.pid}:${identity.started_at}`, identity]));
  await writeFile(receiptPath, JSON.stringify([...receipts.values()], null, 2) + '\n');
  return owned;
}

async function identities(guiPid: number, surfaces: Surface[]): Promise<any[]> {
  const all = await ownedProcesses(guiPid);
  const codex = all.filter(process => process.name.toLowerCase() === 'codex.exe');
  expect(codex, 'exactly eight owned real Codex processes').toHaveLength(8);
  const runtimeDirectory = join(HOME, '.gwt', 'sessions', 'runtime', String(guiPid));
  const runtimeFiles = (await readdir(runtimeDirectory)).filter(file => file.endsWith('.json'));
  const result = [];
  for (const surface of surfaces) {
    const projectAgents = await surface.page.evaluate(() => {
      return (window as any).__gwt5538Workspace?.tabs.flatMap((tab: any) => tab.workspace.windows)
        .filter((pane: any) => pane.preset === 'agent').map((pane: any) => ({ id: pane.id, agent_id: pane.agent_id }));
    });
    expect(projectAgents, 'one builtin Codex agent per isolated Project').toEqual([{ id: surface.id, agent_id: 'codex' }]);
    expect(runtimeFiles).toContain(`${surface.session_id}.json`);
    const runtime = JSON.parse(await readFile(join(runtimeDirectory, `${surface.session_id}.json`), 'utf8'));
    const child = codex.find(process => process.pid === runtime.child_pid);
    // On Windows the PTY child can be a shell wrapper; correlate its descendants.
    const branch = new Set<number>([runtime.child_pid]);
    for (let changed = true; changed;) {
      changed = false;
      for (const process of all) if (branch.has(process.parent_pid) && !branch.has(process.pid)) { branch.add(process.pid); changed = true; }
    }
    const providers = codex.filter(process => branch.has(process.pid));
    expect(providers, `one real provider for ${surface.id}`).toHaveLength(1);
    expect(runtime.status).toBe('Running');
    const marker = await heartbeat(surface);
    expect(marker?.session_id).toBe(surface.session_id);
    expect(marker?.shell_pid).toBe(surface.heartbeat_shell_pid);
    expect(marker.tick, 'a real heartbeat shell belongs to this exact session').toBeGreaterThan(0);
    expect(branch.has(marker.shell_pid), 'heartbeat shell belongs to the owned provider subtree').toBe(true);
    const lineage = new Map<number, ProcessIdentity>();
    for (const leaf of [providers[0].pid, marker.shell_pid]) {
      for (let current = all.find(process => process.pid === leaf); current;
        current = all.find(process => process.pid === current!.parent_pid)) {
        lineage.set(current.pid, current);
        if (current.parent_pid === guiPid) break;
      }
    }
    result.push({ window_id: surface.id, session_id: surface.session_id, runtime_child_pid: runtime.child_pid,
      runtime_child_started_at: runtime.child_started_at, source_event: runtime.source_event, child: child || providers[0],
      heartbeat_shell_pid: marker.shell_pid, heartbeat_tick: marker.tick,
      heartbeat_elapsed_ms: marker.elapsed_ms, heartbeat_observed_at: marker.observed_at,
      process_lineage: [...lineage.values()] });
  }
  expect(new Set(result.map(row => row.child.pid)).size).toBe(8);
  return result;
}

async function profileRows(pid: number, start: string | null, end: string | null): Promise<any> {
  const directory = join(HOME, '.gwt', 'logs', 'perf');
  const files = (await readdir(directory).catch(() => [])).filter(file => /^perf-.*\.jsonl$/.test(file));
  const rows: any[] = [];
  for (const file of files) for (const line of (await readFile(join(directory, file), 'utf8')).split('\n')) {
    if (!line.trim()) continue;
    let record;
    try { record = JSON.parse(line); } catch { continue; } // writer's final incomplete line is not evidence
    if (record.type === 'gui_dispatch_summary' && record.dispatch?.pid === pid) rows.push(record);
  }
  const from = start ? Date.parse(start) : -Infinity;
  const until = end ? Date.parse(end) : Infinity;
  const complete = rows.filter(row => {
    const ended = Date.parse(row.dispatch.window_ended_at);
    return ended <= until && ended - row.dispatch.window_ms >= from && row.dispatch.window_ms >= 60_000;
  });
  const groups = new Map<string, { event: string; count: number; total_ms: number; max_ms: number }>();
  for (const row of complete) {
    if (row.dispatch.count === 0) continue; // Empty-minute liveness row, not an invented event kind.
    const group = groups.get(row.target) || { event: row.target, count: 0, total_ms: 0, max_ms: 0 };
    group.count += row.dispatch.count; group.total_ms += row.dispatch.total_ms;
    group.max_ms = Math.max(group.max_ms, row.dispatch.max_ms); groups.set(row.target, group);
  }
  const windows = new Map(complete.map(row => [row.dispatch.window_ended_at, row.dispatch.window_ms]));
  return { dispatch_summary_observed: rows.length > 0, dispatch_summary_rows: complete,
    complete_window_count: windows.size, complete_window_ms: [...windows.values()].reduce((sum, value) => sum + value, 0),
    warmup_or_boundary_rows: rows.filter(row => !complete.includes(row)),
    top10: [...groups.values()].sort((a, b) => b.total_ms - a.total_ms).slice(0, 10),
    limitations: 'Only completed >50ms handlers are profiled. Full summary windows strictly inside the measurement interval form top10; warmup and boundary rows are retained separately. An absent summary is not proof of zero stalls or of collector operation. This heartbeat workload writes private progress receipts and does not cover implementation builds.' };
}
