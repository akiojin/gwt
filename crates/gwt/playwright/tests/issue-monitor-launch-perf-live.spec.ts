/** Issue #5596: real Monitor delivery completion + PTY handoff, offline providers. */
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { expect, test } from '@playwright/test';
import { gotoLiveGwt, sendLiveGwtEvent } from './_helpers/live-gwt';

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL || '';
const HOME = process.env.GWT5596_HOME || '';
test.skip(!BASE || !HOME, 'requires scripts/verify-issue-5596.ps1');
test.use({ headless: false });
test('Issue #5596: twelve admitted launches keep completion handlers within budget', async ({ page }, info) => {
  test.setTimeout(600_000);
  const fixture = JSON.parse(await readFile(join(HOME, 'fixture.json'), 'utf8'));
  const theme = info.project.name.includes('light') ? 'light' : 'dark';
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  let passed = false;
  let measurement: any = {};
  try {
    expect(fixture.issues).toHaveLength(12);
    expect(fixture.checkout_binary.replaceAll('\\', '/')).toContain('/target/debug/gwt.exe');
    await page.addInitScript(theme => localStorage.setItem('gwt:ui:theme', theme), theme);
    await page.bringToFront();
    await gotoLiveGwt(page, BASE, { enableTestBridge: true, projectKey: fixture.project_key });
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    await expect.poll(async () => {
      const providers = await launches();
      // Startup admission can wait for a provider handshake before opening
      // another PTY. Release each recorder as the twelve admitted requests
      // drain through their per-project materialization lane.
      await Promise.all(providers.map(provider => writeFile(join(HOME, `ready-${provider.session_id}`), 'start\n')));
      return providers.length;
    }, {
      timeout: 300_000, message: 'twelve real local provider PTYs from persisted Monitor deliveries',
    }).toBe(12);
    const providers = await launches();
    expect(new Set(providers.map(entry => entry.session_id)).size).toBe(12);
    expect(new Set(providers.map(entry => entry.pid)).size).toBe(12);
    for (const issue of fixture.issues) {
      expect(providers.filter(entry => entry.argv.join(' ').includes(`#${issue}`)), `one owner prompt for #${issue}`).toHaveLength(1);
    }
    for (const provider of providers) {
      expect(provider.cwd.replaceAll('\\', '/')).toContain(HOME.replaceAll('\\', '/'));
      expect(() => process.kill(provider.pid, 0), 'owned provider is alive').not.toThrow();
      await writeFile(join(HOME, `ready-${provider.session_id}`), 'start\n');
    }
    await expect.poll(async () => {
      measurement = await observe(fixture);
      return Promise.all(providers.map(async provider => {
        try {
          const hook = JSON.parse(await readFile(join(HOME, `hook-${provider.session_id}.json`), 'utf8'));
          return hook.success === true
            && measurement.user_prompt_authenticated_sessions.includes(provider.session_id)
            && measurement.user_prompt_matched_sessions.includes(provider.session_id);
        } catch { return false; }
      }));
    }, { timeout: 120_000, message: 'all twelve exact UserPromptSubmit hooks are authenticated and matched by the GUI' }).toEqual(Array(12).fill(true));
    const windowListCursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
    await sendLiveGwtEvent(page, { kind: 'list_windows' });
    await expect.poll(() => page.evaluate(({ sessions, cursor }) => {
      const entry = ((window as any).__gwtPlaywrightMessages || []).filter((entry: any) => entry.sequence > cursor && entry.payload.kind === 'window_list').at(-1);
      const windows = entry?.payload.windows || [];
      return sessions.filter(session => windows.some((window: any) => window.session_id === session && ['running', 'idle'].includes(window.status))).length;
    }, { sessions: providers.map(provider => provider.session_id), cursor: windowListCursor }), { timeout: 120_000, message: 'twelve provider sessions are bound to running Work windows' }).toBe(12);
    // The summary contains only >50 ms handlers. Wait for a complete owned
    // minute after the final completion, so an absent target row has meaning.
    await expect.poll(async () => {
      measurement = await observe(fixture);
      return measurement.target_count >= 12 && measurement.summary_covers_completion;
    }, { timeout: 180_000, message: 'collector flushes the minute containing all completion handlers' }).toBe(true);
    expect(await launches(), 'exactly twelve provider receipts remain after the measurement window').toHaveLength(12);
    expect(errors, 'zero console/page errors').toEqual([]);
    expect(measurement.summary_dropped_windows, 'no missing collector windows').toBe(0);
    expect(measurement.slow_target_count, 'all completed target handlers are at most 50 ms').toBe(0);
    expect(measurement.target_rolling_minute_upper_bound_ms, 'all target handler time in every rolling minute is at most 200 ms').toBeLessThanOrEqual(200);
    expect(measurement.gui_event_loop_stall_count, 'zero owned GUI stall ledger entries').toBe(0);
    await info.attach(`launch-performance-${theme}`, { body: await page.screenshot({ animations: 'disabled' }), contentType: 'image/png' });
    passed = true;
  } finally {
    // Baseline also asserts thresholds and exits nonzero on failure. The phase
    // label changes evidence provenance only; it never converts a failure to PASS.
    measurement = await observe(fixture);
    const evidence = {
      issue: 5596, phase: fixture.phase, theme, passed, fixture, ...measurement,
      launches: await launches(), browser_errors: errors,
      'User Verification Result': 'n/a (autonomous)', 'Agent Visual Check': passed ? 'pass' : 'fail',
      limitations: 'Offline already-admitted delivery replay measures real GUI completion and PTY materialization. It excludes upstream claim/admission and model execution. gui_dispatch_summary retains only handlers over 50 ms; debug dispatch timing provides all event totals/max at integer millisecond resolution. The 200 ms assertion uses a conservative +1 ms per handler upper bound. Both themes inspect the same owned launch batch.',
    };
    const json = JSON.stringify(evidence, null, 2) + '\n';
    await writeFile(join(fixture.evidence_directory, `launch-${theme}.json`), json);
    await info.attach(`launch-evidence-${theme}`, { body: json, contentType: 'application/json' });
  }
});

async function launches(): Promise<any[]> {
  const files = (await readdir(HOME)).filter(file => /^provider-launch-.*\.json$/.test(file));
  return Promise.all(files.map(file => readFile(join(HOME, file), 'utf8').then(JSON.parse)));
}

async function jsonRows(directory: string, pattern: RegExp): Promise<any[]> {
  const rows: any[] = [];
  for (const name of (await readdir(directory).catch(() => [])).filter(name => pattern.test(name))) {
    for (const line of (await readFile(join(directory, name), 'utf8')).split(/\r?\n/)) {
      if (!line.trim()) continue;
      try { rows.push(JSON.parse(line)); } catch { /* an incomplete final writer line is not evidence */ }
    }
  }
  return rows;
}

async function observe(fixture: any): Promise<any> {
  const directories = [join(HOME, '.gwt/logs'), join(HOME, '.gwt/projects', fixture.project_key, 'logs')];
  const logs = (await Promise.all(directories.map(directory => jsonRows(directory, /^gwt\.log\./)))).flat();
  const started = Date.parse(fixture.gui_started_at);
  const promptReceipts = logs.filter(row => row.target === 'gwt.hook.delivery' && Date.parse(row.timestamp) >= started
    && row.fields?.source_event === 'UserPromptSubmit');
  const dispatch = logs.filter(row => row.target === 'gwt.frontend.timing' && Date.parse(row.timestamp) >= started
    && (row.fields?.message === 'GUI event loop dispatch handled' || /blocked the GUI event loop for \d+ms/.test(row.fields?.message || '')));
  const targets = dispatch.filter(row => row.fields.event === 'IssueMonitorLaunchPrepared');
  const summaries = (await jsonRows(join(HOME, '.gwt/logs/perf'), /^perf-.*\.jsonl$/))
    .filter(row => row.type === 'gui_dispatch_summary' && row.dispatch?.pid === fixture.gui_pid);
  const ledger = (await jsonRows(join(HOME, '.gwt/logs/errors'), /^errors\..*\.jsonl$/))
    .filter(row => row.kind === 'gui_event_loop_stall' && Number(row.context?.pid) === fixture.gui_pid);
  const launchPhaseRows = logs.filter(row => row.target === 'gwt.launch.timing' && Date.parse(row.timestamp) >= started);
  const launchPhases = new Map<string, any>();
  for (const row of launchPhaseRows) {
    const { phase, thread, elapsed_ms: elapsedMs } = row.fields || {};
    if (!Number.isFinite(elapsedMs)) continue;
    const key = JSON.stringify([phase ?? null, thread ?? null]);
    const group = launchPhases.get(key) || { phase: phase ?? null, thread: thread ?? null, count: 0, total_ms: 0, max_ms: 0 };
    group.count++; group.total_ms += elapsedMs; group.max_ms = Math.max(group.max_ms, elapsedMs); launchPhases.set(key, group);
  }
  const groups = new Map<string, any>();
  for (const row of dispatch) {
    const value = row.fields.elapsed_ms;
    const group = groups.get(row.fields.event) || { event: row.fields.event, count: 0, total_ms: 0, max_ms: 0 };
    group.count++; group.total_ms += value; group.max_ms = Math.max(group.max_ms, value); groups.set(group.event, group);
  }
  const firstCompletion = Math.min(Infinity, ...targets.map(row => Date.parse(row.timestamp)));
  const lastCompletion = Math.max(0, ...targets.map(row => Date.parse(row.timestamp)));
  const windows = [...new Map(summaries.map(row => [row.dispatch.window_ended_at, {
    ended_at: row.dispatch.window_ended_at,
    end_ms: Date.parse(row.dispatch.window_ended_at),
    start_ms: Date.parse(row.dispatch.window_ended_at) - row.dispatch.window_ms,
    window_ms: row.dispatch.window_ms,
  }])).values()].sort((a, b) => a.start_ms - b.start_ms);
  const relevantWindows = windows.filter(window => window.end_ms >= firstCompletion && window.start_ms <= lastCompletion);
  // UTC and monotonic clocks are read consecutively at each boundary. A small
  // serialization/rounding tolerance cannot hide a missing sixty-second window.
  const coversCompletions = relevantWindows.length > 0
    && relevantWindows[0].start_ms <= firstCompletion
    && relevantWindows.at(-1)!.end_ms >= lastCompletion
    && relevantWindows.every((window, index) => window.window_ms >= 60_000
      && (!index || window.start_ms - relevantWindows[index - 1].end_ms <= 10));
  let rollingUpperBound = 0;
  for (const end of targets.map(row => Date.parse(row.timestamp))) {
    rollingUpperBound = Math.max(rollingUpperBound, targets.filter(row => Date.parse(row.timestamp) > end - 60_000 && Date.parse(row.timestamp) <= end)
      .reduce((sum, row) => sum + row.fields.elapsed_ms + 1, 0));
  }
  return {
    user_prompt_authenticated_sessions: [...new Set(promptReceipts.filter(row => row.fields.stage === 'receipt'
      && row.fields.outcome === 'authenticated').map(row => row.fields.gwt_session_id))],
    user_prompt_matched_sessions: [...new Set(promptReceipts.filter(row => row.fields.stage === 'window'
      && row.fields.outcome === 'matched' && row.fields.route === 'live').map(row => row.fields.gwt_session_id))],
    user_prompt_receipt_rows: promptReceipts,
    target_count: targets.length,
    target_rolling_minute_upper_bound_ms: rollingUpperBound,
    all_event_handlers: [...groups.values()].sort((a, b) => b.total_ms - a.total_ms),
    launch_phase_rows: launchPhaseRows,
    launch_phases: [...launchPhases.values()].sort((a, b) => b.total_ms - a.total_ms),
    target_handler_rows: targets,
    gui_dispatch_summary: summaries,
    summary_windows_covering_completions: relevantWindows,
    summary_covers_completion: coversCompletions,
    summary_dropped_windows: summaries.reduce((sum, row) => sum + row.dispatch.dropped_windows, 0),
    slow_target_count: summaries.filter(row => row.target === 'handler:IssueMonitorLaunchPrepared').reduce((sum, row) => sum + row.dispatch.count, 0),
    gui_event_loop_stall_count: ledger.length,
    gui_event_loop_stall_rows: ledger,
    dispatch_stall_warning_count: logs.filter(row => row.target === 'gwt.frontend.timing' && Date.parse(row.timestamp) >= started
      && (/blocked the GUI event loop for \d+ms/.test(row.fields?.message || '') || row.fields?.message === 'frontend dispatch still running')).length,
  };
}
