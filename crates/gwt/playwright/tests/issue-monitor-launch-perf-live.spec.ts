/** Issue #5596: real Monitor delivery completion + PTY handoff, offline providers. */
import { readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { join } from 'node:path';
import { expect, test } from '@playwright/test';
import { gotoLiveGwt, sendLiveGwtEvent } from './_helpers/live-gwt';

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL || '';
const HOME = process.env.GWT5596_HOME || '';
const SEQUENTIAL = process.env.GWT5596_SEQUENTIAL === '1';
const LAUNCH_COUNT = Number(process.env.GWT5596_LAUNCH_COUNT || '10');
test.skip(!BASE || !HOME, 'requires scripts/verify-issue-5596.ps1');
test.use({ headless: false });
test(SEQUENTIAL ? `Issue #6096/#6393: ${LAUNCH_COUNT} sequential admitted launches finish within 120 seconds each`
  : 'Issue #5596: twelve admitted launches keep completion handlers within budget', async ({ page }, info) => {
  test.setTimeout(SEQUENTIAL ? 1_400_000 : 600_000);
  const fixture = JSON.parse(await readFile(join(HOME, 'fixture.json'), 'utf8'));
  const count = fixture.sequential ? fixture.launch_count ?? 10 : 12;
  const theme = info.project.name.includes('light') ? 'light' : 'dark';
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  let passed = false;
  let measurement: any = {};
  const cargoLoadPeriod: any = {};
  try {
    expect(fixture.issues).toHaveLength(count);
    expect(fixture.expected_deliveries).toHaveLength(count);
    expect(fixture.checkout_binary.replaceAll('\\', '/')).toContain('/target/debug/gwt.exe');
    await page.addInitScript(theme => localStorage.setItem('gwt:ui:theme', theme), theme);
    await page.bringToFront();
    await gotoLiveGwt(page, BASE, { enableTestBridge: true, projectKey: fixture.project_key });
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    if (fixture.cargo_load) cargoLoadPeriod.activated = await setCargoLoad(fixture, true);
    if (fixture.sequential) await sequentialLaunches(page, fixture);
    // Issue #6393 times window materialization under load. Provider/hook
    // assertions still run below, after all three window intervals finish.
    if (fixture.cargo_load) cargoLoadPeriod.windows_completed = await setCargoLoad(fixture, false);
    await expect.poll(async () => {
      const providers = await launches();
      // Startup admission can wait for a provider handshake before opening
      // another PTY. Release each recorder as the twelve admitted requests
      // drain through their per-project materialization lane.
      await Promise.all(providers.map(provider => writeFile(join(HOME, `ready-${provider.session_id}`), 'start\n')));
      return providers.length;
    }, {
      timeout: 300_000, message: `${count} real local provider PTYs from persisted Monitor deliveries`,
    }).toBe(count);
    const providers = await launches();
    expect(new Set(providers.map(entry => entry.session_id)).size).toBe(count);
    expect(new Set(providers.map(entry => entry.pid)).size).toBe(count);
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
            && measurement.user_prompt_matched_sessions.includes(provider.session_id)
            && measurement.durable_launch_ack_complete
            && measurement.durable_launch_ack_rows.some((ack: any) => ack.complete
              && provider.argv.join(' ').includes(`#${ack.issue_number}`)
              && ack.matched_session_ids.includes(provider.session_id));
        } catch { return false; }
      }));
    }, { timeout: 120_000, message: 'all exact UserPromptSubmit hooks are authenticated, matched and durably acknowledged by the GUI' }).toEqual(Array(count).fill(true));
    const windowListCursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
    await sendLiveGwtEvent(page, { kind: 'list_windows' });
    await expect.poll(() => page.evaluate(({ sessions, cursor }) => {
      const entry = ((window as any).__gwtPlaywrightMessages || []).filter((entry: any) => entry.sequence > cursor && entry.payload.kind === 'window_list').at(-1);
      const windows = entry?.payload.windows || [];
      return sessions.filter(session => windows.some((window: any) => window.session_id === session && ['running', 'idle'].includes(window.status))).length;
    }, { sessions: providers.map(provider => provider.session_id), cursor: windowListCursor }), { timeout: 120_000, message: 'provider sessions are bound to running Work windows' }).toBe(count);
    // The summary contains only >50 ms handlers. Wait for a complete owned
    // minute after the final completion, so an absent target row has meaning.
    if (!fixture.sequential) {
      await expect.poll(async () => {
        measurement = await observe(fixture);
        return measurement.target_count >= count && measurement.summary_covers_completion;
      }, { timeout: 180_000, message: 'collector flushes the minute containing all completion handlers' }).toBe(true);
      expect(measurement.summary_dropped_windows, 'no missing collector windows').toBe(0);
      expect(measurement.slow_target_count, 'all completed target handlers are at most 50 ms').toBe(0);
      expect(measurement.target_rolling_minute_upper_bound_ms, 'all target handler time in every rolling minute is at most 200 ms').toBeLessThanOrEqual(200);
      expect(measurement.gui_event_loop_stall_count, 'zero owned GUI stall ledger entries').toBe(0);
    }
    expect(await launches(), 'exactly the admitted provider receipts remain after measurement').toHaveLength(count);
    expect(errors, 'zero console/page errors').toEqual([]);
    expect(measurement.failed_issue_rows, 'no failed Monitor deliveries').toEqual([]);
    await info.attach(`launch-performance-${theme}`, { body: await page.screenshot({ animations: 'disabled' }), contentType: 'image/png' });
    passed = true;
  } finally {
    // A second theme starts a fresh browser; its setup is outside launch timing.
    if (fixture.cargo_load) cargoLoadPeriod.paused = await setCargoLoad(fixture, false);
    // Baseline also asserts thresholds and exits nonzero on failure. The phase
    // label changes evidence provenance only; it never converts a failure to PASS.
    measurement = await observe(fixture);
    const terminalDiagnostics = await page.evaluate((ids: string[]) => ids.map(id => {
      const messages = ((window as any).__gwtPlaywrightMessages || []).filter((entry: any) =>
        entry.payload.kind === 'terminal_output' && entry.payload.id === id);
      const output = messages.map((entry: any) => {
        try { return atob(entry.payload.data_base64 || ''); } catch { return ''; }
      }).join('');
      return { id, metrics: (window as any).__gwtTerminalTestApi?.metrics(id),
        cursor_query_seen: output.includes('\u001b[6n'), output_bytes: output.length,
        missing_shell_image: /pwsh\.dll.*(?:not exist|not found)/i.test(output) };
    }), measurement.durable_launch_ack_rows.filter((row: any) => row.window_id).map((row: any) => row.window_id));
    const evidence = {
      issue: fixture.cargo_load ? 6393 : fixture.sequential ? 6096 : 5596, phase: fixture.phase, theme, passed, fixture, ...measurement,
      cargo_load_period: fixture.cargo_load ? cargoLoadPeriod : undefined,
      terminal_diagnostics: terminalDiagnostics,
      sequential_launches: fixture.sequential ? JSON.parse(await readFile(join(HOME, 'sequential-launches.json'), 'utf8').catch(() => '[]')) : undefined,
      launches: await launches(), browser_errors: errors,
      'User Verification Result': 'n/a (autonomous)', 'Agent Visual Check': passed ? 'pass' : 'fail',
      limitations: 'Offline already-admitted delivery replay excludes upstream claim/admission and model execution. Sequential mode times provider/hook handoff within 120 seconds. Cargo-load mode instead times exact durable claim/delivery materialization, a session-bound running window and native terminal runtime presence within 120 seconds; hidden-terminal fit is recorded separately because it waits for reveal. All provider/hook assertions remain required after the three timed windows and load pause. The real build script records identical Cargo/build-script PIDs and increasing CPU/work counters across each loaded window interval. Both themes inspect the same owned launch batch.',
    };
    const json = JSON.stringify(evidence, null, 2) + '\n';
    await writeFile(join(fixture.evidence_directory, `launch-${theme}.json`), json);
    await info.attach(`launch-evidence-${theme}`, { body: json, contentType: 'application/json' });
  }
});

async function sequentialLaunches(page: any, fixture: any): Promise<void> {
  const receiptsPath = join(HOME, 'sequential-launches.json');
  const retained = await readFile(receiptsPath, 'utf8').catch(() => null);
  // Dark and light inspect the same launches. A partial prior run remains
  // failed evidence; the second theme cannot silently retry or replace it.
  const receipts: any[] = retained ? JSON.parse(retained) : [];
  if (!retained) {
    // Confirm the normal daemon control lane before starting any delivery's
    // timer. Every subsequent enqueue includes its own acknowledged scan wake.
    await new Promise<void>((resolve, reject) => {
      execFile(join(HOME, 'bin/enqueue.exe'), ['--ready'], {
        cwd: fixture.project, timeout: 30_000, windowsHide: true,
      }, error => error ? reject(error) : resolve());
    });
    for (const expected of fixture.expected_deliveries) {
      const loadBefore = fixture.cargo_load ? await cargoLoadSample(fixture) : undefined;
      const enqueuedAt = new Date().toISOString();
      const receipt: any = { ...expected, enqueued_at: enqueuedAt, complete: false, cargo_load_before: loadBefore,
        timing_boundary: fixture.cargo_load ? 'durable-materialization-and-ready-native-window' : 'authenticated-provider-handoff' };
      receipts.push(receipt);
      await writeFile(receiptsPath, JSON.stringify(receipts, null, 2));
      try {
        // The helper links this checkout's production locked prefs mutation.
        // No shell, external GitHub write or whole-file read/replace is used.
        await new Promise<void>((resolve, reject) => {
          const child = execFile(join(HOME, 'bin/enqueue.exe'), [], {
            cwd: fixture.project, timeout: 120_000, windowsHide: true,
          }, (error, _stdout, stderr) => error
            ? reject(new Error(`fixture enqueue failed: ${stderr.trim() || error.message}`))
            : resolve());
          child.stdin!.end(JSON.stringify({
            ...expected, branch_name: `work/issue-${expected.issue_number}`, linked_issue_kind: 'issue',
            claim_owner: 'fixture', launch_session_strategy: 'fresh_required', created_at: enqueuedAt,
          }));
        });
        const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
        await expect.poll(async () => {
          if (fixture.cargo_load) {
            // An already-started recorder must not hold the next launch's
            // admission handshake while we measure windows independently.
            await Promise.all((await launches()).map(provider => writeFile(join(HOME, `ready-${provider.session_id}`), 'start\n')));
            const measurement = await observe(fixture);
            expect(measurement.failed_issue_rows, 'no failed Monitor deliveries under Cargo load').toEqual([]);
            const ack = measurement.durable_launch_ack_rows.find((ack: any) => ack.issue_number === expected.issue_number);
            if (!ack?.window_id || ack.confirmation_delivery_id !== expected.delivery_id
              || ack.confirmation_claim_id !== expected.claim_id || ack.launched_claim_id !== expected.claim_id
              || ack.launch_binding !== expected.issue_number || !ack.launched_issue_matches) return false;
            await sendLiveGwtEvent(page, { kind: 'list_windows' });
            const bound = await page.evaluate(({ id, cursor }: any) => {
              const reply = ((window as any).__gwtPlaywrightMessages || []).filter((entry: any) => entry.sequence > cursor && entry.payload.kind === 'window_list').at(-1);
              const candidate = reply?.payload.windows?.find((entry: any) => entry.id === id && entry.session_id && ['running', 'idle'].includes(entry.status));
              // SPEC-2008 FR-057 intentionally defers fit for hidden hosts.
              const terminal = (window as any).__gwtTerminalTestApi?.metrics(id);
              return candidate && terminal?.hasRuntime ? { candidate, terminal } : null;
            }, { id: ack.window_id, cursor });
            if (!bound) return false;
            Object.assign(receipt, { window_id: ack.window_id, session_id: bound.candidate.session_id,
              materialization: ack, terminal_metrics: bound.terminal });
            return true;
          }
          const providers = (await launches()).filter(provider => provider.argv.join(' ').includes(`#${expected.issue_number}`));
          if (providers.length !== 1) return false;
          const provider = providers[0];
          receipt.provider_observed_at ||= new Date().toISOString();
          await writeFile(join(HOME, `ready-${provider.session_id}`), 'start\n');
          const measurement = await observe(fixture);
          expect(measurement.failed_issue_rows, 'no failed rows during a sequential launch').toEqual([]);
          const ack = measurement.durable_launch_ack_rows.find((ack: any) => ack.issue_number === expected.issue_number);
          if (!ack?.complete || !ack.matched_session_ids.includes(provider.session_id)) return false;
          const hook = JSON.parse(await readFile(join(HOME, `hook-${provider.session_id}.json`), 'utf8').catch(() => '{}'));
          if (!hook.success || !measurement.user_prompt_authenticated_sessions.includes(provider.session_id)) return false;
          await sendLiveGwtEvent(page, { kind: 'list_windows' });
          const bound = await page.evaluate(({ session, cursor }: any) => {
            const reply = ((window as any).__gwtPlaywrightMessages || []).filter((entry: any) => entry.sequence > cursor && entry.payload.kind === 'window_list').at(-1);
            return reply?.payload.windows?.some((window: any) => window.session_id === session && ['running', 'idle'].includes(window.status));
          }, { session: provider.session_id, cursor });
          if (bound) Object.assign(receipt, { window_id: ack.window_id, session_id: provider.session_id });
          return bound === true;
        }, { timeout: Math.max(1, 120_000 - (Date.now() - Date.parse(enqueuedAt))), message: `#${expected.issue_number} exact ACK and live window within 120 seconds` }).toBe(true);
        receipt.completed_at = new Date().toISOString();
        receipt.elapsed_ms = Date.now() - Date.parse(enqueuedAt);
        expect(receipt.elapsed_ms, 'enqueue-to-bound-ACK must be strictly under 120 seconds').toBeLessThan(120_000);
        if (fixture.cargo_load) receipt.cargo_load_after = await cargoLoadSample(fixture);
        receipt.complete = true;
      } catch (error) {
        receipt.failure = error instanceof Error ? error.message : String(error);
        receipt.elapsed_ms = Date.now() - Date.parse(enqueuedAt);
        receipt.timed_out = /timeout|timed out/i.test(receipt.failure);
        throw error;
      } finally {
        receipt.timed_out ||= !receipt.complete && Date.now() - Date.parse(enqueuedAt) >= 120_000;
        await writeFile(receiptsPath, JSON.stringify(receipts, null, 2));
      }
    }
  }
  expect(receipts, 'all configured sequential launches measured').toHaveLength(fixture.launch_count ?? 10);
  expect(receipts.filter(receipt => receipt.timed_out), 'zero timed out launches').toHaveLength(0);
  for (const receipt of receipts) {
    expect(receipt.complete, `#${receipt.issue_number} completed`).toBe(true);
    expect(receipt.elapsed_ms).toBeLessThan(120_000);
    if (fixture.cargo_load) {
      expect(receipt.cargo_load_after.workload.work_units, 'concurrent Cargo build made CPU-work progress').toBeGreaterThan(receipt.cargo_load_before.workload.work_units);
      const before = receipt.cargo_load_before.processes;
      const after = receipt.cargo_load_after.processes;
      for (const process of before) {
        expect(after.find((entry: any) => entry.pid === process.pid)?.started_at, 'same live owned Cargo/build-script process spans launch').toBe(process.started_at);
      }
      expect(after.find((entry: any) => entry.pid === receipt.cargo_load_after.workload.pid)?.cpu_ms, 'actual build-script process consumed CPU during launch')
        .toBeGreaterThan(before.find((entry: any) => entry.pid === receipt.cargo_load_before.workload.pid)?.cpu_ms);
    }
  }
}

async function setCargoLoad(fixture: any, active: boolean): Promise<any> {
  const controlPath = join(HOME, 'cargo-load/start');
  if (active) await writeFile(controlPath, 'start\n');
  else await rm(controlPath, { force: true });
  let workload: any;
  const observed = expect.poll(async () => {
    workload = (await jsonRows(join(HOME, 'cargo-load'), /^progress\.jsonl$/)).at(-1);
    expect(() => process.kill(fixture.cargo_load.cargo_pid, 0), 'same owned Cargo process remains alive').not.toThrow();
    if (workload) expect(() => process.kill(workload.pid, 0), 'same owned build-script remains alive').not.toThrow();
    return workload?.workers ?? -1;
  }, { timeout: 30_000, message: active ? 'Cargo CPU workload active before agent launch' : 'Cargo CPU workload idle before next browser startup' });
  if (active) await observed.toBeGreaterThan(0);
  else await observed.toBe(0);
  return { observed_at: new Date().toISOString(), workload };
}

async function cargoLoadSample(fixture: any): Promise<any> {
  const workload = (await jsonRows(join(HOME, 'cargo-load'), /^progress\.jsonl$/)).at(-1);
  expect(workload?.workers, 'Cargo build CPU workload is ready').toBeGreaterThan(0);
  const pids = [fixture.cargo_load.cargo_pid, workload.pid];
  for (const pid of pids) {
    expect(Number.isSafeInteger(pid) && pid > 0, 'exact owned process PID').toBe(true);
    expect(() => process.kill(pid, 0), 'owned Cargo/build-script process is alive').not.toThrow();
  }
  // OS CPU counters distinguish an active build from a sleeping Cargo parent.
  // Sampling runs outside the measured enqueue-to-live-window interval.
  const command = `Get-Process -Id ${pids.join(',')} -ErrorAction Stop | Select-Object @{n='pid';e={$_.Id}},@{n='cpu_ms';e={$_.TotalProcessorTime.TotalMilliseconds}},@{n='started_at';e={$_.StartTime.ToUniversalTime().ToString('o')}} | ConvertTo-Json -Compress -AsArray`;
  const processes = await new Promise<any[]>((resolve, reject) => {
    execFile('pwsh.exe', ['-NoProfile', '-NonInteractive', '-Command', command], {
      cwd: fixture.project, timeout: 30_000, windowsHide: true,
    }, (error, stdout) => {
      if (error) reject(error);
      else { try { resolve(JSON.parse(stdout)); } catch (error) { reject(error); } }
    });
  });
  return { observed_at: new Date().toISOString(), workload, processes };
}

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
  let prefs: any = {};
  let prefsReadError: string | null = null;
  try {
    prefs = JSON.parse(await readFile(join(HOME, '.gwt/projects', fixture.project_key, 'project-state/issue-monitor.json'), 'utf8'));
    if (!prefs || typeof prefs !== 'object' || Array.isArray(prefs)) {
      prefs = {};
      prefsReadError = 'invalid_shape';
    }
  } catch (error) {
    prefsReadError = error instanceof SyntaxError ? 'invalid_json' : 'read_error';
  }
  const matchedReceipts = promptReceipts.filter(row => row.fields.stage === 'window'
    && row.fields.outcome === 'matched' && row.fields.route === 'live');
  const durableAckRows = (fixture.expected_deliveries || []).map((expected: any) => {
    const confirmation = prefs.launch_confirmations?.[expected.issue_number];
    const windowId = confirmation?.window_id ?? null;
    const binding = prefs.launch_bindings?.[windowId] ?? null;
    const launchedClaim = prefs.launched_claims?.[expected.issue_number] ?? null;
    const launchedIssueMatches = (prefs.launched_issues || []).some((launched: any) =>
      launched.issue_number === expected.issue_number && launched.window_id === windowId);
    const matchedSessions = [...new Set(matchedReceipts.filter(row => row.fields.window_id === windowId)
      .map(row => row.fields.gwt_session_id))];
    return {
      issue_number: expected.issue_number, delivery_id: expected.delivery_id, claim_id: expected.claim_id,
      window_id: windowId, confirmation_delivery_id: confirmation?.delivery_id ?? null,
      confirmation_claim_id: confirmation?.claim_id ?? null, launch_binding: binding,
      launched_claim_id: launchedClaim, launched_issue_matches: launchedIssueMatches,
      matched_session_ids: matchedSessions,
      complete: typeof expected.delivery_id === 'string' && expected.delivery_id.length > 0
        && typeof expected.claim_id === 'string' && expected.claim_id.length > 0
        && typeof windowId === 'string' && windowId.length > 0
        && confirmation.delivery_id === expected.delivery_id && confirmation.claim_id === expected.claim_id
        && binding === expected.issue_number && launchedClaim === expected.claim_id
        && launchedIssueMatches && matchedSessions.length === 1,
    };
  });
  const durableAckCounts = {
    launched_issues: (prefs.launched_issues || []).length,
    launch_bindings: Object.keys(prefs.launch_bindings || {}).length,
    launched_claims: Object.keys(prefs.launched_claims || {}).length,
    launch_confirmations: Object.keys(prefs.launch_confirmations || {}).length,
    pending_launch_deliveries: (prefs.pending_launch_deliveries || []).length,
    launching_issues: (prefs.launching_issues || []).length,
    failed_issues: (prefs.failed_issues || []).length,
  };
  const count = fixture.expected_deliveries.length;
  const durableAckComplete = prefsReadError === null && durableAckRows.length === count
    && fixture.issues.every((issue: number) => durableAckRows.filter((ack: any) => ack.issue_number === issue).length === 1)
    && durableAckRows.every((ack: any) => ack.complete)
    && new Set(durableAckRows.map((ack: any) => ack.window_id)).size === count
    && durableAckCounts.launched_issues === count && durableAckCounts.launch_bindings === count
    && durableAckCounts.launched_claims === count && durableAckCounts.launch_confirmations === count
    && durableAckCounts.pending_launch_deliveries === 0 && durableAckCounts.launching_issues === 0
    && durableAckCounts.failed_issues === 0;
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
    durable_launch_ack_rows: durableAckRows,
    durable_launch_ack_counts: durableAckCounts,
    durable_launch_ack_complete: durableAckComplete,
    durable_launch_ack_read_error: prefsReadError,
    failed_issue_rows: prefs.failed_issues || [],
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
