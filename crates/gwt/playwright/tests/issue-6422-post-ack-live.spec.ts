/** Investigation only: identical policy, low/high load, ACK and provider separated. */
import { execFile } from 'node:child_process';
import { readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { expect, test } from '@playwright/test';
import { gotoLiveGwt, sendLiveGwtEvent } from './_helpers/live-gwt';

const HOME = process.env.GWT5596_HOME || '';
const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL || '';
test.skip(!HOME || !BASE, 'requires verify-issue-5596.ps1 -Sequential -CargoLoad -LaunchCount 2 -PostAckDiagnostics and one selected theme per fresh fixture');
test.use({ headless: false });

test('Issue #6422: observe ACK to provider under unchanged low/high resource policy', async ({ page }, info) => {
  test.setTimeout(600_000);
  const fixture = JSON.parse(await readFile(join(HOME, 'fixture.json'), 'utf8'));
  expect(fixture.post_ack_diagnostics).toBe(true);
  const theme = info.project.name.includes('light') ? 'light' : 'dark';
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  const measurements: any[] = [];
  let passed = false;
  try {
    await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
    await page.bringToFront();
    await gotoLiveGwt(page, BASE, { enableTestBridge: true, projectKey: fixture.project_key });
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    const observer = JSON.parse(await readFile(join(fixture.evidence_directory, 'observer-ready.json'), 'utf8'));
    expect(observer.priority, 'observer outside Agent policy runs at canonical Normal priority').toBe('Normal');
    expect(observer.current_job.error, 'unknown Job observation is not evidence of an uncapped observer').toBeUndefined();
    expect(typeof observer.current_job.member).toBe('boolean');
    if (observer.current_job.member) {
      expect(observer.current_job.cpu_query_error).toBe(0);
      expect(observer.current_job.limit_query_error).toBe(0);
      expect(typeof observer.current_job.cpu_flags).toBe('number');
      expect(observer.current_job.cpu_flags & 5, 'observer current Job has no enabled hard CPU cap').not.toBe(5);
    }
    await enqueue(fixture, null);
    for (const [index, load] of ['low', 'high'].entries()) {
      const expected = fixture.expected_deliveries[index];
      const before = await setLoad(load === 'high');
      expect(before.workers).toBe(load === 'high' ? 30 : 0);
      const measurement: any = { ...expected, load, load_before: before,
        enqueued_at: new Date().toISOString(), observations: [] };
      measurements.push(measurement);
      await enqueue(fixture, expected);
      // The 120 seconds are an observation window, not a provider performance SLA.
      let until = Date.now() + 120_000;
      do {
        const snapshot = await observe(fixture, expected);
        measurement.observations.push(snapshot);
        if (snapshot.ack && !measurement.ack_observed_at) {
          measurement.ack_observed_at = snapshot.observed_at;
          if (load === 'high') until = Date.now() + 120_000;
          const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
          await sendLiveGwtEvent(page, { kind: 'list_windows' });
          await expect.poll(async () => {
            measurement.bound_window = await page.evaluate(({ cursor, id }: any) => {
              const reply = ((window as any).__gwtPlaywrightMessages || []).filter((entry: any) => entry.sequence > cursor && entry.payload.kind === 'window_list').at(-1);
              const candidate = reply?.payload.windows?.find((window: any) => window.id === id && window.session_id && ['running', 'idle'].includes(window.status));
              const metrics = (window as any).__gwtTerminalTestApi?.metrics(id);
              return candidate && metrics?.hasRuntime ? { id, session_id: candidate.session_id, status: candidate.status, metrics } : null;
            }, { cursor, id: snapshot.ack.window_id });
            return !!measurement.bound_window;
          }, { timeout: 30_000, message: 'exact ACK has a bound running native terminal runtime during the measured load' }).toBe(true);
          measurement.bound_window_observed_at = new Date().toISOString();
        }
        if (snapshot.provider && !measurement.provider_observed_at) {
          measurement.provider_observed_at = snapshot.observed_at;
          measurement.provider = snapshot.provider;
        }
        if (snapshot.hook_complete && !measurement.hook_observed_at) measurement.hook_observed_at = snapshot.observed_at;
        if (load === 'low' && snapshot.hook_complete) break;
        await new Promise(resolve => setTimeout(resolve, 500));
      } while (Date.now() < until);
      measurement.loaded_observation_ended_at = new Date().toISOString();
      measurement.load_at_end = await progress();
      measurement.load_paused = await setLoad(false);
      expect(measurement.ack_observed_at, 'ACK was observed before pausing the measured load').toBeTruthy();
      if (load === 'high') expect(Date.parse(measurement.loaded_observation_ended_at) - Date.parse(measurement.ack_observed_at), 'full post-ACK observation under load').toBeGreaterThanOrEqual(120_000);
      // Investigation has no new provider SLA. Preserve the actual outcome,
      // including a monitored timeout/rollback, instead of asserting recovery.
      const recoveryUntil = Date.now() + 30_000;
      let final: any;
      do {
        const snapshot = await observe(fixture, expected);
        measurement.observations.push(snapshot);
        final = snapshot;
        if (snapshot.hook_complete || !snapshot.ack) break;
        await new Promise(resolve => setTimeout(resolve, 500));
      } while (Date.now() < recoveryUntil);
      measurement.outcome = final.hook_complete ? 'first-hook-complete'
        : !final.ack ? 'materialization-rolled-back' : 'first-hook-unobserved-after-recovery-window';
      measurement.recovery_observation_ended_at = new Date().toISOString();
      measurement.provider ||= final.provider;
      if (load === 'low') {
        expect(final.hook_complete, 'the low-load control must authenticate and match the first hook').toBe(true);
        const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
        await sendLiveGwtEvent(page, { kind: 'list_windows' });
        await expect.poll(() => page.evaluate(({ cursor, session }: any) => {
          const reply = ((window as any).__gwtPlaywrightMessages || []).filter((entry: any) => entry.sequence > cursor && entry.payload.kind === 'window_list').at(-1);
          return reply?.payload.windows?.some((window: any) => window.session_id === session && ['running', 'idle'].includes(window.status));
        }, { cursor, session: final.provider.session_id })).toBe(true);
      }
      if (load === 'high') {
        expect(measurement.load_at_end.pid).toBe(before.pid);
        expect(measurement.load_at_end.workers).toBe(30);
        expect(measurement.load_at_end.work_units).toBeGreaterThan(before.work_units);
      }
      const samples = (await readFile(join(fixture.evidence_directory, 'processes.jsonl'), 'utf8')).split(/\r?\n/)
        .flatMap(line => { try { return [JSON.parse(line)]; } catch { return []; } })
        .filter(row => row.started_at >= measurement.enqueued_at && row.completed_at <= measurement.loaded_observation_ended_at);
      expect(samples.length, 'OS observer actually sampled the measured interval').toBeGreaterThan(1);
      const build = samples.flatMap(row => row.processes.filter((entry: any) => entry.pid === before.pid));
      expect(build.length, 'same real Cargo build-script was observed').toBeGreaterThan(1);
      expect(build.every(entry => entry.priority === 'Normal')).toBe(true);
      if (load === 'high') expect(build.at(-1).cpu_ms).toBeGreaterThan(build[0].cpu_ms);
    }
    expect(errors, 'zero console/page errors').toEqual([]);
    await info.attach(`post-ack-${theme}`, { body: await page.screenshot(), contentType: 'image/png' });
    passed = true;
  } finally {
    await setLoad(false);
    const raw = (await logs(fixture)).filter(row => ['gwt.launch.timing', 'gwt.hook.delivery', 'gwt.process.summary', 'gwt_input_trace'].includes(row.target)
      || row.fields?.stage === 'monitor_startup_timeout');
    const allowed = ['message', 'phase', 'thread', 'elapsed_ms', 'captured_at', 'occurred_at', 'source_event', 'stage', 'state', 'outcome', 'route', 'gwt_session_id', 'session_id', 'window_id', 'issue_number', 'delivery_id', 'claim_id', 'pid', 'child_pid', 'root_pid', 'parent_pid', 'event', 'bytes', 'error'];
    const timeline = raw.map(row => ({ timestamp: row.timestamp, target: row.target,
      fields: Object.fromEntries(Object.entries(row.fields || {}).filter(([key]) => allowed.includes(key))),
      spans: (row.spans || []).map((span: any) => Object.fromEntries(Object.entries(span).filter(([key]) => ['name', 'issue_number', 'delivery_id', 'window_id'].includes(key)))) }));
    const terminal = await page.evaluate(() => {
      const messages = ((window as any).__gwtPlaywrightMessages || []).filter((row: any) => row.payload.kind === 'terminal_output');
      const ids = [...new Set(messages.map((row: any) => row.payload.id))];
      return ids.map(id => {
        const output = messages.filter((row: any) => row.payload.id === id).map((row: any) => atob(row.payload.data_base64 || '')).join('');
        return { id, output_bytes: output.length, dsr_queries: output.split('\u001b[6n').length - 1,
          metrics: (window as any).__gwtTerminalTestApi?.metrics(id) };
      });
    }).catch(() => []);
    const evidence = { issue: 6422, theme, passed, fixture, measurements, timeline, terminal, browser_errors: errors,
      'User Verification Result': 'n/a (autonomous)', 'Agent Visual Check': passed ? 'pass' : 'fail',
      limitations: 'Offline already-admitted deliveries and a native recorder provider; upstream claims, real model execution and provider internals are excluded. Receipt observation times are upper bounds, not exact main-entry times. ProcessThread snapshots are not a continuous scheduler/stack trace. No direct PTY release timestamp exists. Both load states share the same GUI/binaries/policy; other shared-host work remains uncontrolled.' };
    const json = JSON.stringify(evidence, null, 2) + '\n';
    await writeFile(join(fixture.evidence_directory, `post-ack-${theme}.json`), json);
    await info.attach(`post-ack-evidence-${theme}`, { body: json, contentType: 'application/json' });
  }
});

async function progress(): Promise<any> {
  const lines = (await readFile(join(HOME, 'cargo-load/progress.jsonl'), 'utf8')).trim().split(/\r?\n/);
  for (const line of lines.reverse()) { try { return JSON.parse(line); } catch { /* writer can have a partial tail */ } }
  throw new Error('No complete Cargo progress row');
}

async function setLoad(active: boolean): Promise<any> {
  const path = join(HOME, 'cargo-load/start');
  if (active) await writeFile(path, 'start\n'); else await rm(path, { force: true });
  let row: any;
  await expect.poll(async () => { row = await progress(); return row.workers; }, { timeout: 30_000 })
    .toBe(active ? 30 : 0);
  return { ...row, observed_at: new Date().toISOString() };
}

async function enqueue(fixture: any, expected: any): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const child = execFile(join(HOME, 'bin/enqueue.exe'), expected ? [] : ['--ready'], {
      cwd: fixture.project, timeout: 120_000, windowsHide: true,
    }, (error, _stdout, stderr) => error ? reject(new Error(`enqueue failed: ${stderr || error.message}`)) : resolve());
    if (expected) child.stdin!.end(JSON.stringify({ ...expected, branch_name: `work/issue-${expected.issue_number}`,
      linked_issue_kind: 'issue', claim_owner: 'fixture', launch_session_strategy: 'fresh_required', created_at: new Date().toISOString() }));
  });
}

async function logs(fixture: any): Promise<any[]> {
  const rows: any[] = [];
  for (const dir of [join(HOME, '.gwt/logs'), join(HOME, '.gwt/projects', fixture.project_key, 'logs')]) {
    for (const file of (await readdir(dir).catch(() => [])).filter(name => /^gwt\.log\./.test(name))) {
      for (const line of (await readFile(join(dir, file), 'utf8')).split(/\r?\n/)) {
        try { rows.push(JSON.parse(line)); } catch { /* incomplete tail remains unobserved */ }
      }
    }
  }
  return rows;
}

async function observe(fixture: any, expected: any): Promise<any> {
  const prefs = JSON.parse(await readFile(join(HOME, '.gwt/projects', fixture.project_key, 'project-state/issue-monitor.json'), 'utf8'));
  const confirmation = prefs.launch_confirmations?.[expected.issue_number];
  const windowId = confirmation?.window_id;
  const ack = confirmation?.delivery_id === expected.delivery_id && confirmation?.claim_id === expected.claim_id
    && prefs.launch_bindings?.[windowId] === expected.issue_number && prefs.launched_claims?.[expected.issue_number] === expected.claim_id
    && prefs.launched_issues?.some((row: any) => row.issue_number === expected.issue_number && row.window_id === windowId)
    ? { window_id: windowId, ...confirmation } : null;
  let provider: any = null;
  let hook: any = null;
  for (const file of (await readdir(HOME)).filter(name => /^provider-launch-.*\.json$/.test(name))) {
    const candidate = JSON.parse(await readFile(join(HOME, file), 'utf8'));
    if (!candidate.argv.join(' ').includes(`#${expected.issue_number}`)) continue;
    provider = { pid: candidate.pid, session_id: candidate.session_id, receipt_created_at: (await stat(join(HOME, file))).birthtime.toISOString() };
    await writeFile(join(HOME, `ready-${provider.session_id}`), 'start\n');
    try { hook = JSON.parse(await readFile(join(HOME, `hook-${provider.session_id}.json`), 'utf8')); } catch { /* completion is not yet observed */ }
  }
  const receipts = (await logs(fixture)).filter(row => row.target === 'gwt.hook.delivery'
    && row.fields?.gwt_session_id === provider?.session_id && row.fields?.source_event === 'UserPromptSubmit');
  const authenticated = receipts.some(row => row.fields?.outcome === 'authenticated');
  const matched = receipts.some(row => row.fields?.outcome === 'matched' && row.fields?.route === 'live' && row.fields?.window_id === windowId);
  return { observed_at: new Date().toISOString(), ack, provider, hook_complete: !!(ack && hook?.success && authenticated && matched),
    authenticated, matched, failed_issues: (prefs.failed_issues || []).map((row: any) => row.issue_number) };
}
