/* Issue #5016 SAFE-T4: runner contracts and opt-in real AgentBootstrap measurement. */
import { expect, test, type Page } from '@playwright/test';
import { execFile, spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { cpus, loadavg, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { gotoLiveGwt, openLiveLaunchWizardForBranch, sendLiveGwtEvent } from './_helpers/live-gwt';

const ROOT = process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd();
const RUNNER = resolve(ROOT, 'scripts/measure-pull-render.mjs');

test.describe('pull-render runner contract', () => {
  test('whole host CPU is a covariate without a bracket limit', () => {
    const first = { at: 0, busy_ms: 100, total_ms: 1000, logical_cpus: 1 };
    const last = { at: 2000, busy_ms: 810, total_ms: 2100, logical_cpus: 1 };
    expect(wholeHostCpuRatio(first, last)).toBeCloseTo(710 / 1100);
    expect(wholeHostCpuRatio(first, { ...last, busy_ms: 1200 })).toBe(1);
    expect(() => wholeHostCpuRatio(first, { ...last, busy_ms: 99 })).toThrow(/counter/);
    expect(() => wholeHostCpuRatio(first, { ...last, logical_cpus: 2 })).toThrow(/CPU count/);
    expect(rendererPids([{ type: 'renderer', id: 10 }, { type: 'renderer', id: 9 }])).toEqual([9, 10]);
    expect(() => rendererPids([{ type: 'renderer', id: 9 }, { type: 'renderer', id: 9 }])).toThrow(/Renderer/);
  });

  test('paired selection uses same age, actual time coverage and conservative clock hull', () => {
    const result = spawnSync(process.execPath, ['--input-type=module', '-e', `
      import assert from 'node:assert/strict';
      const { pairedComparison } = await import(process.argv[1]);
      const run = () => {
        const ticks = Array.from({ length: 1801 }, (_, k) => ({ at: k * 1000, busy_ms: 0, total_ms: k * 1000, logical_cpus: 1 }));
        return { sample_status: { observed: 1800, overrun: 0, missing: 0, missing_fraction: 0, overrun_fraction: 0 }, seconds: 1800, comparison_rule: 'observed_slot_coverage_host_covariates_v1', timing: { barrier_at: 0, ws: { start: 0 }, cpu: { start: 1, counter_start: 0 },
          browser_clock: [{ before: 0, after: 10, browser: 5 }, { before: 1800000, after: 1800010, browser: 1800005 }] },
          raw: { seconds: Array.from({ length: 1800 }, (_, k) => ({ status: 'observed', ws_at: (k + 1) * 1000, cpu_at: (k + 1) * 1000 + 1,
            ws_counter: k + 1, cpu_counter: 0, renderer_rss_bytes: 10, renderer_heap_bytes: 10,
            host: { method: 'whole_host_cpu_covariates_v1', loadavg: [0, 0, 0], start: ticks[k], end: ticks[k + 1], whole_host_cpu_ratio: 0 } })),
            latency: Array.from({ length: 9000 }, (_, k) => ({ index: k % 5, sequence: Math.floor(k / 5) + 1, receive: 0, dom: 1, frame: 2 })),
            long_tasks: [[1000, 60]] }, long_tasks: { count: 1, duration_ms: 60 } };
      };
      const high = (run, k) => {
        for (let j = k; j < 1800; j++) run.raw.seconds[j].host.end.busy_ms += 700;
        run.raw.seconds[k].host.whole_host_cpu_ratio = 0.7;
      };
      const a = run(), b = run(); high(a, 0); high(b, 1);
      const paired = pairedComparison(a, b);
      assert.deepEqual(paired.mask.slice(0, 3), [true, true, true]);
      assert.equal(paired.runs[0].metrics.dom_latency_ms.count, 9000);
      assert.equal(paired.runs[0].long_tasks.count, 1);
      const missing = run(), overrun = run();
      missing.raw.seconds[11].host.start = missing.raw.seconds[10].host.start;
      missing.raw.seconds[10] = { status: 'missing', deadline: 11000 };
      overrun.raw.seconds[20].status = 'overrun';
      missing.sample_status = { observed: 1799, overrun: 0, missing: 1, missing_fraction: 1 / 1800, overrun_fraction: 0 };
      overrun.sample_status = { observed: 1799, overrun: 1, missing: 0, missing_fraction: 0, overrun_fraction: 1 / 1800 };
      // CPU endpoints at +1ms overlap the next missing slot: exclude the whole preceding interval too.
      const incomplete = pairedComparison(missing, overrun);
      assert.equal(incomplete.mask[10], false);
      assert.equal(incomplete.mask[20], false);
      assert.equal(incomplete.runs[0].coverage.ws.retained_ms, 1796000);
      assert.equal(incomplete.runs[1].coverage.cpu.retained_ms, 1793999);
      assert.equal(incomplete.runs[0].metrics.dom_latency_ms.count, 8990);
      assert.equal(incomplete.runs[0].long_tasks.count, 1);
      const insufficientBand = run();
      for (let k = 0; k < 31; k++) insufficientBand.raw.seconds[k] = { status: 'missing', deadline: (k + 1) * 1000 };
      insufficientBand.sample_status = { observed: 1769, overrun: 0, missing: 31, missing_fraction: 31 / 1800, overrun_fraction: 0 };
      assert.throws(() => pairedComparison(insufficientBand, run()), /coverage/);
      const maskedRss = run(); high(maskedRss, 0); maskedRss.raw.seconds[0].renderer_rss_bytes = null;
      assert.throws(() => pairedComparison(maskedRss, b), /metric/);
      const missingLoad = run(); delete missingLoad.raw.seconds[0].host.loadavg;
      assert.throws(() => pairedComparison(missingLoad, b), /host metadata/);
      const badProvenance = run(); badProvenance.raw.seconds[0].host.whole_host_cpu_ratio = 0.9;
      assert.throws(() => pairedComparison(badProvenance, b), /host metadata/);
      const obsolete = run(); obsolete.comparison_rule = 'host_cpu_residual_v1';
      assert.throws(() => pairedComparison(obsolete, b), /host metadata/);
      const invalidCounter = run(); invalidCounter.raw.seconds[0].host.end.total_ms = -1;
      assert.throws(() => pairedComparison(invalidCounter, b), /host metadata/);
      const missingTask = run(); missingTask.raw.long_tasks = [];
      assert.throws(() => pairedComparison(missingTask, b), /Long task/);
      b.raw.long_tasks = [[2000, 60]];
      assert.equal(pairedComparison(a, b).runs[1].long_tasks.count, 1);
      for (let k = 2; k < 31; k++) high(b, k);
      assert.equal(pairedComparison(a, b).mask.every(Boolean), true);
      const shortCoverage = run(); shortCoverage.timing.ws.start = 200000;
      shortCoverage.raw.seconds.forEach(row => { row.ws_at += 200000; });
      assert.throws(() => pairedComparison(a, shortCoverage), /coverage/);
      const clock = run(); clock.timing.browser_clock[1].after += 1000;
      assert.throws(() => pairedComparison(run(), clock), /clock/);
    `, pathToFileURL(RUNNER).href], { encoding: 'utf8', timeout: 10_000 });
    expect(result.status, result.stderr).toBe(0);
  });

  test('rate snapshots are atomic and means are invariant to interval partitioning', () => {
    expect(typeof createWireCounter).toBe('function');
    const wire = createWireCounter();
    const report = JSON.stringify({ kind: 'pm_reports', body: '日本語' });
    wire.record('received', report);
    const firstWire = wire.read();
    wire.record('received', JSON.stringify({ kind: 'unknown_kind', secret: 'never retained' }));
    wire.record('received', Buffer.from([0, 255]));
    wire.record('sent', '{"kind":"load_pm_reports"}');
    wire.record('sent', '{"kind":"update_terminal_grid","cols":80,"rows":24}');
    expect(wire.read().sent).toHaveProperty('update_terminal_grid', { bytes: 51, frames: 1 });
    expect(firstWire.received.pm_reports).toEqual({ bytes: Buffer.byteLength(report), frames: 1 });
    expect(firstWire.received.other.frames).toBe(0);
    expect(wire.read().received.binary).toEqual({ bytes: 2, frames: 1 });
    expect(Object.values(wire.read().received).reduce((sum, entry) => sum + entry.bytes, 0))
      .toBe(Buffer.byteLength(report) + Buffer.byteLength('{"kind":"unknown_kind","secret":"never retained"}') + 2);
    expect(wire.read().sent.load_pm_reports.frames).toBe(1);
    expect(JSON.stringify(wire.read())).not.toMatch(/unknown_kind|never retained|日本語/);
    expect(parseTargetProcesses('1 Mon Oct 12 10:00:00 2026 00:00.00 100 S')).toHaveLength(1);
    let counter = 100;
    const first = captureRateSnapshot(() => counter, () => { counter = 200; return 1000; });
    expect(first).toEqual({ counter: 100, at: 1000 });
    const start = { counter: 0, at: 0 }, end = { counter: 100, at: 10_000 };
    expect(rateStatistics([100, 0], start, end).mean).toBe(10);
    expect(rateStatistics([100, ...Array(9).fill(0)], start, end).mean).toBe(10);
    expect(rateStatistics([100, 0], start, end, 100).mean).toBe(1000);
  });

  test('slow report CLI leaves the sampler and next report free and reaps owned children', async () => {
    const directory = mkdtempSync(join(tmpdir(), 'gwt-report-async-'));
    const binary = join(directory, 'gwtd-fixture');
    writeFileSync(binary, `#!${process.execPath}\nconst fs = require('node:fs'); process.stdin.resume(); process.stdin.on('end', () => fs.writeFileSync(process.env.REPORT_READY + '/' + process.pid + '.ready', '')); setInterval(() => {}, 1000);\n`, { mode: 0o755 });
    let clock = 1000;
    const poster = createReportPoster({ binary, project: directory, env: { ...process.env, REPORT_READY: directory }, now: () => clock });
    let pids: number[] = [];
    try {
      poster.post(1, 1000);
      let sampled = false;
      await Promise.resolve().then(() => { sampled = true; });
      expect(sampled).toBe(true);
      clock = 6000; poster.post(2, 6000);
      await expect.poll(() => readdirSync(directory).filter(name => name.endsWith('.ready')).length).toBe(2);
      expect(poster.records.map(record => record.started)).toEqual([1000, 6000]);
      pids = poster.records.map(record => record.pid!);
    } finally { await poster.stop(); rmSync(directory, { recursive: true, force: true }); }
    for (const pid of pids) expect(() => process.kill(pid, 0)).toThrow();
  });

  test('timing validation rejects missing boundaries and missed cadence while accepting jitter', () => {
    const run: any = { seconds: 10, samples: 10, ws_payload_bytes: 100, reports_posted: 2,
      metrics: { ws_payload_bytes_per_second: { mean: 10 }, backend_cpu_percent: { mean: 10 } },
      timing: { barrier_at: 0, ws: { start: 0, end: 10_000, counter_start: 0, counter_end: 100, start_lateness_ms: 0 },
        cpu: { start: 1, end: 10_001, counter_start: 0, counter_end: 1, initial_probe_duration_ms: 1, start_lateness_ms: 1 } },
      raw: { seconds: Array.from({ length: 10 }, (_, index) => ({
        status: 'observed', deadline: (index + 1) * 1000, ws_at: (index + 1) * 1000 + (index === 9 ? 0 : 50),
        ws_counter: (index + 1) * 10, cpu_at: (index + 1) * 1000 + (index === 9 ? 1 : 51),
        cpu_counter: (index + 1) / 10, cpu_probe_duration_ms: 1,
        collected_at: (index + 1) * 1000 + (index === 9 ? 2 : 52) })) },
      report_timings: [1000, 6000].map((deadline, index) => ({ sequence: index + 1, deadline,
        started: deadline + 50, finished: deadline + 200, pid: index + 100, exit_code: 0, timed_out: false })) };
    const validate = (value: unknown) => {
      const result = spawnSync(process.execPath, ['--input-type=module', '-e',
        `import(process.argv[1]).then(({validateMeasurementTiming}) => { let text = ''; process.stdin.on('data', x => text += x); process.stdin.on('end', () => console.log(JSON.stringify(validateMeasurementTiming(JSON.parse(text))))); });`,
        pathToFileURL(RUNNER).href], { input: JSON.stringify(value), encoding: 'utf8', timeout: 10_000 });
      expect(result.status, result.stderr).toBe(0); return JSON.parse(result.stdout);
    };
    expect(validate(run)).toEqual([]);
    const beforeEnd = structuredClone(run); beforeEnd.timing.ws.end -= 0.2;
    expect(validate(beforeEnd)).toContain('WS counter/window mismatch');
    const afterEnd = structuredClone(run); afterEnd.timing.ws.end += 0.2;
    afterEnd.metrics.ws_payload_bytes_per_second.mean = 100 / (afterEnd.timing.ws.end / 1000);
    expect(validate(afterEnd)).toEqual([]);
    expect(validate({ ...run, timing: undefined })).toContain('timing metadata missing');
    const missed = structuredClone(run); missed.raw.seconds[2].ws_at += 1000;
    expect(validate(missed)).toContain('sample cadence missed');
    const late = structuredClone(run); late.report_timings[1].started += 5000;
    expect(validate(late)).toContain('Reports cadence missed');
    expect(validate({ ...run, ws_payload_bytes: 101 })).toContain('WS counter/window mismatch');
    const lagging = structuredClone(run); lagging.timing.barrier_at = -1000;
    expect.soft(validate(lagging)).toContain('barrier start missed');
    const early = structuredClone(run); early.timing.barrier_at = 1;
    expect.soft(validate(early)).toContain('barrier clock mismatch');
    const overtime = structuredClone(run); overtime.report_timings[0].finished = overtime.report_timings[0].started + 30_001;
    expect.soft(validate(overtime)).toContain('Reports timeout');
    const missingTimeout = structuredClone(run); delete missingTimeout.report_timings[0].timed_out;
    expect.soft(validate(missingTimeout)).toContain('Reports completion/window mismatch');
  });

  test('overlapping cleanup callers await the same owned child exit', () => {
    const result = spawnSync(process.execPath, ['--input-type=module', '-e', `
      import assert from 'node:assert/strict';
      import { spawn } from 'node:child_process';
      import { once } from 'node:events';
      import { mkdtempSync, rmSync } from 'node:fs';
      import { cpus, loadavg, tmpdir } from 'node:os';
      import { join } from 'node:path';
      const { createMeasurementCleanup } = await import(process.argv[1]);
      const home = mkdtempSync(join(tmpdir(), 'gwt-cleanup-overlap-'));
      const grandchildSource = '(' + function() { process.on('SIGTERM', () => {}); setInterval(() => {}, 1000); process.send('ready'); }.toString() + ')()';
      const leaderSource = "const {spawn} = require('node:child_process'); const grandchild = spawn(process.execPath, ['-e', " + JSON.stringify(grandchildSource) + "], {stdio:['ignore','ignore','ignore','ipc']}); grandchild.once('message', () => process.send({ready: grandchild.pid})); process.on('SIGTERM', () => process.send('term')); process.on('message', () => process.exit(0));";
      const child = spawn(process.execPath, ['-e', leaderSource],
        { detached: true, stdio: ['ignore', 'ignore', 'ignore', 'ipc'] });
      let grandchildPid;
      const exited = once(child, 'exit');
      const originalKill = process.kill.bind(process);
      let emptyProbeEperm = false;
      const eperm = () => Object.assign(new Error('kill EPERM'), { code: 'EPERM' });
      try {
        grandchildPid = (await once(child, 'message'))[0].ready;
        assert.ok(Number.isInteger(grandchildPid));
        process.kill = (pid, signal = 0) => {
          if (pid === -child.pid && signal === 0) throw eperm();
          return originalKill(pid, signal);
        };
        await assert.rejects(createMeasurementCleanup({ getTestChild: () => child, agentDir: home, workloadFile: join(home, 'task.cjs') })(), /EPERM/);
        process.kill = (pid, signal = 0) => {
          try { return originalKill(pid, signal); }
          catch (error) {
            // Darwin can report EPERM while an empty pgrp is being removed.
            if (pid === -child.pid && signal === 0 && ['ESRCH', 'EPERM'].includes(error.code)) {
              emptyProbeEperm = true; throw eperm();
            }
            throw error;
          }
        };
        const cleanup = createMeasurementCleanup({ getTestChild: () => child, agentDir: home, workloadFile: join(home, 'task.cjs') });
        const terminating = once(child, 'message');
        const first = cleanup();
        assert.equal((await terminating)[0], 'term');
        let secondFinished = false;
        const second = cleanup().then(() => { secondFinished = true; });
        await Promise.resolve(); await Promise.resolve();
        assert.equal(secondFinished, false, 'overlapping cleanup must await ongoing teardown');
        child.send('release');
        await Promise.all([first, second]);
        assert.equal(child.exitCode, 0);
        assert.equal(emptyProbeEperm, true);
        assert.throws(() => process.kill(grandchildPid, 0), 'owned group must disappear after leader exits');
      } finally {
        process.kill = originalKill;
        if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        await exited;
        if (grandchildPid) {
          try { process.kill(grandchildPid, 'SIGKILL'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
          const deadline = performance.now() + 10000;
          while (performance.now() < deadline) {
            try { process.kill(grandchildPid, 0); } catch (error) { if (error.code === 'ESRCH') break; throw error; }
            await new Promise(resolve => setTimeout(resolve, 100));
          }
          assert.throws(() => process.kill(grandchildPid, 0), 'exact owned grandchild cleanup must finish');
        }
        rmSync(home, { recursive: true, force: true });
      }
    `, pathToFileURL(RUNNER).href], { encoding: 'utf8', timeout: 35_000 });
    expect(result.status, result.stderr).toBe(0);
  });

  test('cleanup reaps a TERM-resistant owned PTY and preserves a foreign identity', () => {
    const result = spawnSync(process.execPath, ['--input-type=module', '-e', `
      import assert from 'node:assert/strict';
      import { spawn } from 'node:child_process';
      import { once } from 'node:events';
      import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
      import { cpus, loadavg, tmpdir } from 'node:os';
      import { join } from 'node:path';
      const { createMeasurementCleanup } = await import(process.argv[1]);
      const home = mkdtempSync(join(tmpdir(), 'gwt-cleanup-pty-'));
      const task = join(home, 'task.cjs');
      writeFileSync(task, "let terms = 0; process.on('SIGTERM', () => { terms++; process.send('term'); }); process.on('message', () => process.send(terms)); process.send('ready');");
      const owned = spawn(process.execPath, [task, '0'], { detached: true, stdio: ['ignore', 'ignore', 'ignore', 'ipc'] });
      const foreign = spawn(process.execPath, [task, '9'], { detached: true, stdio: ['ignore', 'ignore', 'ignore', 'ipc'] });
      const exits = [once(owned, 'exit'), once(foreign, 'exit')];
      try {
        await Promise.all([once(owned, 'message'), once(foreign, 'message')]);
        writeFileSync(join(home, '0.json'), JSON.stringify({ index: 0, pid: owned.pid }));
        writeFileSync(join(home, '1.json'), JSON.stringify({ index: 1, pid: foreign.pid }));
        const terminating = once(owned, 'message');
        await createMeasurementCleanup({ getTestChild: () => undefined, agentDir: home, workloadFile: task })();
        assert.equal((await terminating)[0], 'term');
        assert.equal(owned.signalCode, 'SIGKILL', 'TERM-resistant owned PTY must exit and reap');
        process.kill(foreign.pid, 0);
        const status = once(foreign, 'message'); foreign.send('status');
        assert.equal((await status)[0], 0, 'mismatched identity must receive no signal');
      } finally {
        for (const child of [owned, foreign]) if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        await Promise.all(exits); rmSync(home, { recursive: true, force: true });
      }
    `, pathToFileURL(RUNNER).href], { encoding: 'utf8', timeout: 25_000 });
    expect(result.status, result.stderr).toBe(0);
  });

  test('short runs require smoke and validation cannot claim AC-17', async () => {
    const invoke = (...args: string[]) => spawnSync(process.execPath, [RUNNER, '--validate-only', ...args],
      { encoding: 'utf8', timeout: 10_000 });
    const short = invoke('--mode', 'before', '--seconds', '10');
    expect(short.status).toBe(1);
    expect(short.stderr).toContain('1800');
    const smoke = invoke('--mode', 'smoke', '--seconds', '10');
    expect(smoke.status, smoke.stderr).toBe(0);
    expect(JSON.parse(smoke.stdout)).toMatchObject({ mode: 'smoke', seconds: 10, ac17_pass: false });
    const full = invoke('--mode', 'before');
    expect(full.status, full.stderr).toBe(0);
    expect(JSON.parse(full.stdout)).toMatchObject({ seconds: 1800, agents: 5, ac17_pass: false });
  });

  test('comparison refuses incomplete, mismatched or regressed evidence', async () => {
    const compareMeasurements = (before: unknown, after: unknown) => {
      const result = spawnSync(process.execPath, ['--input-type=module', '-e',
        `import(process.argv[1]).then(({compareMeasurements}) => { let input = ''; process.stdin.on('data', value => input += value); process.stdin.on('end', () => console.log(JSON.stringify(compareMeasurements(...JSON.parse(input))))); });`,
        pathToFileURL(RUNNER).href], { input: JSON.stringify([before, after]), encoding: 'utf8', timeout: 10_000 });
      expect(result.status, result.stderr).toBe(0);
      return JSON.parse(result.stdout);
    };
    const metrics = Object.fromEntries(['backend_cpu_percent', 'renderer_rss_bytes', 'renderer_heap_bytes',
      'ws_payload_bytes_per_second', 'dom_latency_ms', 'render_frame_latency_ms'].map(name =>
      [name, { count: name.includes('latency') ? 9000 : 1800, mean: 10, p50: 10, p95: 10, max: 10 }]));
    const baseline = { sample_status: { observed: 1800, overrun: 0, missing: 0, missing_fraction: 0, overrun_fraction: 0 }, schema_version: 1, comparison_rule: 'observed_slot_coverage_host_covariates_v1', mode: 'before', complete: true, seconds: 1800,
      elapsed_seconds: 1800, agents: 5, environment: { host: 'same', browser: 'same', theme: 'dark',
        viewport: '1440x1000', profile: 'debug', workload: 'f'.repeat(64), harness: 'e'.repeat(64), fixture: 'd'.repeat(64) }, metrics,
      source_sha: 'a'.repeat(40), source_diff_sha256: 'c'.repeat(64), binary_sha256: 'b'.repeat(64), reports_posted: 360,
      ws_payload_bytes: 1000, long_tasks: { count: 0, duration_ms: 0 }, errors: 0,
      lost_markers: 0, dropped_pending: 0, samples: 1800,
      workload: Array.from({ length: 5 }, (_, index) => ({ index, pid: index + 100, sequence: 1800,
        bytes: 1800 * 8192, dom_sequence: 1800, render_sequence: 1800 })) };
    Object.assign(baseline, { cleanup_success: true, evidence_valid: true, invalid_reasons: [],
      timing: { barrier_at: 0, ws: { start: 0, end: 1_800_000, counter_start: 0, counter_end: 1000, start_lateness_ms: 0 },
        cpu: { start: 1, end: 1_800_001, counter_start: 0, counter_end: 180, initial_probe_duration_ms: 1, start_lateness_ms: 1 } },
      raw: { seconds: Array.from({ length: 1800 }, (_, index) => ({ status: 'observed', deadline: (index + 1) * 1000,
        ws_at: (index + 1) * 1000, ws_counter: (index + 1) * 1000 / 1800,
        cpu_at: (index + 1) * 1000 + 1, cpu_counter: (index + 1) / 10,
        cpu_probe_duration_ms: 1, collected_at: (index + 1) * 1000 + 2 })) },
      report_timings: Array.from({ length: 360 }, (_, index) => ({ sequence: index + 1,
        deadline: 1000 + index * 5000, started: 1000 + index * 5000, finished: 1001 + index * 5000,
        pid: index + 1000, exit_code: 0, timed_out: false })) });
    (baseline as any).timing.browser_clock = [{ before: 0, after: 10, browser: 5 }, { before: 1800000, after: 1800010, browser: 1800005 }];
    const hostTicks = Array.from({ length: 1801 }, (_, k) => ({ at: k * 1000, busy_ms: k * 200, total_ms: k * 1000, logical_cpus: 1 }));
    (baseline as any).raw.seconds.forEach((row: any, k: number) => Object.assign(row, { renderer_rss_bytes: 10, renderer_heap_bytes: 10,
      host: { method: 'whole_host_cpu_covariates_v1', loadavg: [0, 0, 0], start: hostTicks[k], end: hostTicks[k + 1], whole_host_cpu_ratio: 0.2 } }));
    (baseline as any).raw.latency = Array.from({ length: 9000 }, (_, k) => ({ index: k % 5, sequence: Math.floor(k / 5) + 1, receive: 0, dom: 10, frame: 10 }));
    (baseline as any).raw.long_tasks = [];
    baseline.metrics.ws_payload_bytes_per_second.mean = 1000 / 1800;
    const after = structuredClone(baseline);
    after.mode = 'after';
    after.ws_payload_bytes = 900;
    (after as any).timing.ws.counter_end = 900;
    (after as any).raw.seconds.forEach((row: any) => { row.ws_counter *= 0.9; });
    after.metrics.ws_payload_bytes_per_second.mean = 900 / 1800;
    expect(compareMeasurements(baseline, after)).toEqual([]);
    expect(compareMeasurements(baseline, { ...structuredClone(baseline), mode: 'after' })).toEqual([]);
    expect(compareMeasurements(baseline, { ...after, cleanup_success: false })).toContain('after evidence invalid');
    expect(compareMeasurements({ ...baseline, mode: 'smoke' }, after)).toContain('baseline is not a complete 1800-second before run');
    expect(compareMeasurements(baseline, { ...after, elapsed_seconds: 10 })).toContain('after is not a complete 1800-second after run');
    expect(compareMeasurements(baseline, { ...after, environment: { ...after.environment, browser: 'different' } }))
      .toContain('environment/workload mismatch');
    const worse = structuredClone(after);
    worse.metrics.dom_latency_ms.p95 = 11;
    (worse as any).raw.latency.forEach((row: any) => { row.dom = 11; row.frame = 11; });
    expect(compareMeasurements(baseline, worse)).toContain('dom_latency_ms.p95 regressed');
    const missing = structuredClone(after);
    missing.workload[4].render_sequence = 1799;
    expect(compareMeasurements(baseline, missing)).toContain('after workload is incomplete');
    missing.metrics.render_frame_latency_ms.count = 8999;
    expect(compareMeasurements(baseline, missing)).toContain('after latency observations are incomplete');
    expect(compareMeasurements({ ...baseline, source_sha: '' }, after)).toContain('baseline source/build identity is missing');
    expect(compareMeasurements({ ...baseline, source_diff_sha256: '' }, after)).toContain('baseline source/build identity is missing');
    expect(compareMeasurements({ ...baseline, environment: { ...baseline.environment, fixture: '' } }, after))
      .toContain('baseline fixture identity is missing');
  });

  test('fixed custom-agent task emits exactly 8192 bytes per sequence', async () => {
    const directory = mkdtempSync(join(tmpdir(), 'gwt-pull-task-test-'));
    try {
      writeFileSync(join(directory, 'start.json'), JSON.stringify({ start: Date.now() - 1000, seconds: 1 }));
      // Use Node's own ESM loader, independent of Playwright 1.49's TS loader.
      const source = spawnSync(process.execPath, ['--input-type=module', '-e',
        'import(process.argv[1]).then(m => process.stdout.write(m.WORKLOAD_SOURCE));', pathToFileURL(RUNNER).href],
      { encoding: 'utf8', timeout: 10_000 });
      expect(source.status, source.stderr).toBe(0);
      const script = join(directory, 'task.cjs');
      writeFileSync(script, source.stdout);
      const child = spawnSync(process.execPath, [script, '0'], { encoding: 'utf8', timeout: 10_000,
        env: { ...process.env, GWT_PULL_AGENT_DIR: directory, GWT_PULL_BARRIER: join(directory, 'start.json') } });
      expect(child.status, child.stderr).toBe(0);
      expect(Buffer.byteLength(child.stdout)).toBe(8192);
      expect(child.stdout.split('\r\n')).toHaveLength(17);
      expect(JSON.parse(readFileSync(join(directory, '0.json'), 'utf8'))).toMatchObject({ sequence: 1, bytes: 8192, complete: true });
    } finally { rmSync(directory, { recursive: true, force: true }); }
  });
});

// Installed before app.js. The only retained text is a 64-character marker
// boundary tail per fixed workload; payloads and console bodies are not saved.
function installMeasurementProbe() {
  (window as any).__gwtPlaywrightTestBridge = true;
  const NativeWebSocket = window.WebSocket;
  let active = false;
  let ids: string[] = [];
  const tails = ['', '', '', '', ''];
  const received = [0, 0, 0, 0, 0];
  const dom = [0, 0, 0, 0, 0];
  const rendered = [0, 0, 0, 0, 0];
  const pending = Array.from({ length: 5 }, () => new Map<number, { receive: number; assigned: boolean }>());
  const raw: Array<{ index: number; sequence: number; visible_sequence: number; receive: number; dom: number; frame: number }> = [];
  const histograms = [new Array(60_001).fill(0), new Array(60_001).fill(0)];
  const totals = [{ count: 0, sum: 0, max: 0 }, { count: 0, sum: 0, max: 0 }];
  let lost = 0;
  let dropped = 0;
  let coalesced = 0;
  let longCount = 0;
  let longDuration = 0;
  const longRaw: number[][] = [];
  let observer: MutationObserver;
  let longObserver: PerformanceObserver;
  const add = (series: number, value: number) => {
    if (value < 0 || value > 60_000) { dropped++; return; }
    histograms[series][Math.ceil(value)]++;
    totals[series].count++; totals[series].sum += value;
    totals[series].max = Math.max(totals[series].max, value);
  };
  window.WebSocket = new Proxy(NativeWebSocket, {
    construct(Target, args) {
      const socket = new Target(...args as ConstructorParameters<typeof WebSocket>);
      socket.addEventListener('message', event => {
        if (typeof event.data !== 'string') return;
        const at = performance.now();
        try {
          const payload = JSON.parse(event.data);
          const windows = payload.kind === 'workspace_state'
            ? (payload.workspace?.tabs ?? []).flatMap((tab: any) => (tab.workspace?.windows ?? [])
              .map((entry: any) => ({ ...entry, tabId: tab.id })))
            : payload.kind === 'window_list' ? payload.windows ?? [] : [];
          if (windows.length) (window as any).__gwtPullRenderWindowMetadata = windows.slice(-64)
            .map((entry: any) => ({ id: entry.id, agent_id: entry.agent_id ?? null, status: entry.status }));
          for (const entry of windows) if (/^pull-render-[0-4]$/.test(entry.agent_id ?? '')) {
            const id = entry.id.includes('::') || !entry.tabId ? entry.id : `${entry.tabId}::${entry.id}`;
            ((window as any).__gwtPullRenderWindows ??= {})[entry.agent_id] = { id, status: entry.status };
          }
          if (payload.kind === 'launch_wizard_state') {
            const wizard = payload.wizard;
            (window as any).__gwtPullRenderWizard = {
              sequence: ((window as any).__gwtPullRenderWizard?.sequence ?? 0) + 1,
              open: Boolean(wizard), runtime: wizard?.selected_runtime_target,
              agent: wizard?.selected_agent_id, path: wizard?.selected_launch_path, phase: wizard?.phase,
              branch: wizard?.branch_name,
              runtime_options: (wizard?.runtime_target_options ?? []).map((option: any) => option.value),
              busy: Boolean(wizard?.is_hydrating || wizard?.runtime_resolution_pending || wizard?.launch_materialization_pending),
              failed: Boolean(wizard?.error), enabled: Boolean(wizard?.primary_action_enabled),
            };
            return;
          }
          if (!active) return;
          if (payload.kind !== 'terminal_output') return;
          const index = ids.indexOf(payload.id);
          if (index < 0) return;
          const text = tails[index] + atob(payload.data_base64);
          const markers = text.matchAll(/GWT_PULL_5016_(\d)_(\d{4})/g);
          for (const marker of markers) {
            const sequence = Number(marker[2]);
            if (Number(marker[1]) !== index || sequence <= received[index]) continue;
            lost += Math.max(0, sequence - received[index] - 1);
            received[index] = sequence;
            if (pending[index].size >= 256) { dropped++; continue; }
            pending[index].set(sequence, { receive: at, assigned: false });
          }
          // Keep marker delimiters only, never the arbitrary body of a frame.
          tails[index] = '';
          for (let size = Math.min(32, text.length); size > 0; size--) {
            const suffix = text.slice(-size);
            if ('GWT_PULL_5016_'.startsWith(suffix) || /^GWT_PULL_5016_[0-4](?:_\d{0,4})?$/.test(suffix)) {
              tails[index] = suffix; break;
            }
          }
        } catch { dropped++; }
      });
      return socket;
    },
  });
  (window as any).__gwtPullRenderProbe = {
    start(windowIds: string[]) {
      ids = windowIds;
      for (const id of ids) if (!document.querySelector(`.workspace-window[data-id="${id}"] .xterm-rows`)) {
        throw new Error('Measurement requires the actual xterm DOM renderer');
      }
      observer = new MutationObserver(() => {
        if (!active) return;
        ids.forEach((id, index) => {
          const rows = document.querySelector(`.workspace-window[data-id="${id}"] .xterm-rows`);
          const matches = [...(rows?.textContent ?? '').matchAll(/GWT_PULL_5016_(\d)_(\d{4})/g)];
          const marker = matches.filter(match => Number(match[1]) === index).at(-1);
          if (!marker) return;
          const sequence = Number(marker[2]);
          if (sequence <= dom[index]) return;
          if (!pending[index].has(sequence)) { dropped++; return; }
          dom[index] = sequence;
          const domAt = performance.now();
          // One latest DOM commit can represent several ordered RAW updates.
          // Keep every update's latency without demanding a paint per packet.
          const batch = [...pending[index]].filter(([key, entry]) => key <= sequence && !entry.assigned);
          coalesced += Math.max(0, batch.length - 1);
          for (const [, entry] of batch) { entry.assigned = true; add(0, domAt - entry.receive); }
          requestAnimationFrame(() => {
            if (!active) return;
            const frame = performance.now();
            rendered[index] = Math.max(rendered[index], sequence);
            for (const [key, entry] of batch) {
              add(1, frame - entry.receive);
              raw.push({ index, sequence: key, visible_sequence: sequence, receive: entry.receive, dom: domAt, frame });
              pending[index].delete(key);
            }
            if (raw.length > 9000) { raw.pop(); dropped++; }
          });
        });
      });
      observer.observe(document.querySelector('#canvas-stage') ?? document.body,
        { subtree: true, childList: true, characterData: true });
      longObserver = new PerformanceObserver(list => { for (const entry of list.getEntries()) {
        longCount++; longDuration += entry.duration;
        if (longRaw.length < 40960) longRaw.push([entry.startTime, entry.duration]); else dropped++;
      } });
      longObserver.observe({ type: 'longtask', buffered: false });
      active = true;
    },
    stop() {
      active = false; observer?.disconnect(); longObserver?.disconnect();
      const stats = (series: number) => {
        const total = totals[series];
        const quantile = (fraction: number) => {
          let count = 0;
          for (let value = 0; value < histograms[series].length; value++) {
            count += histograms[series][value];
            if (count >= Math.ceil(total.count * fraction)) return value;
          }
          return null;
        };
        return { count: total.count, mean: total.count ? total.sum / total.count : null,
          p50: quantile(0.5), p95: quantile(0.95), max: total.count ? total.max : null };
      };
      return { received, dom, rendered, lost, dropped, coalesced, raw, domStats: stats(0), frameStats: stats(1),
        window_metadata: (window as any).__gwtPullRenderWindowMetadata ?? [],
        long_tasks: { count: longCount, duration_ms: longDuration }, long_raw: longRaw };
    },
  };
}

test('probe contract preserves marker splits and coalesced commits without saving frame bodies', async ({ page }) => {
  await page.setContent(Array.from({ length: 5 }, (_, index) =>
    `<div class="workspace-window" data-id="pane${index}"><div class="xterm-rows"></div></div>`).join(''));
  await page.evaluate(() => { window.WebSocket = class extends EventTarget {} as any; });
  await page.evaluate(installMeasurementProbe);
  const result = await page.evaluate(async () => {
    const probe = (window as any).__gwtPullRenderProbe;
    probe.start(Array.from({ length: 5 }, (_, index) => `pane${index}`));
    const socket = new WebSocket('ws://fixture.invalid');
    const secret = 'FRAME_BODY_MUST_NOT_BE_SAVED';
    const length = 'GWT_PULL_5016_2_0001'.length;
    for (let split = 1; split < length; split++) {
      const marker = `GWT_PULL_5016_2_${String(split).padStart(4, '0')}`;
      for (const text of [secret + marker.slice(0, split), marker.slice(split) + secret]) {
        socket.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ kind: 'terminal_output',
          id: 'pane2', data_base64: btoa(text) }) }));
      }
      document.querySelector('[data-id="pane2"] .xterm-rows')!.textContent = marker;
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }
    for (let sequence = 1; sequence <= 3; sequence++) {
      socket.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ kind: 'terminal_output',
        id: 'pane0', data_base64: btoa(`GWT_PULL_5016_0_${String(sequence).padStart(4, '0')}`) }) }));
    }
    document.querySelector('[data-id="pane0"] .xterm-rows')!.textContent = 'GWT_PULL_5016_0_0003';
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    return { ...probe.stop(), expected: length - 1 };
  });
  expect(result.received[2]).toBe(result.expected);
  expect(result.dom[2]).toBe(result.expected);
  expect(result.rendered[2]).toBe(result.expected);
  expect(result.received[0]).toBe(3);
  expect(result.dom[0]).toBe(3);
  expect(result.rendered[0]).toBe(3);
  expect(result.lost).toBe(0);
  expect(result.dropped).toBe(0);
  expect(result.coalesced).toBe(2);
  expect(result.domStats.count).toBe(result.expected + 3);
  expect(result.frameStats.count).toBe(result.expected + 3);
  expect(result.raw.filter((row: any) => row.index === 0).map((row: any) => [row.sequence, row.visible_sequence]))
    .toEqual([[1, 3], [2, 3], [3, 3]]);
  expect(JSON.stringify(result)).not.toContain('FRAME_BODY_MUST_NOT_BE_SAVED');
});

test('probe contract still detects a genuinely missing WS update', async ({ page }) => {
  await page.setContent(Array.from({ length: 5 }, (_, index) =>
    `<div class="workspace-window" data-id="pane${index}"><div class="xterm-rows"></div></div>`).join(''));
  await page.evaluate(() => { window.WebSocket = class extends EventTarget {} as any; });
  await page.evaluate(installMeasurementProbe);
  const result = await page.evaluate(async () => {
    const probe = (window as any).__gwtPullRenderProbe;
    probe.start(Array.from({ length: 5 }, (_, index) => `pane${index}`));
    const socket = new WebSocket('ws://fixture.invalid');
    socket.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ kind: 'terminal_output',
      id: 'pane0', data_base64: btoa('GWT_PULL_5016_0_0002') }) }));
    document.querySelector('[data-id="pane0"] .xterm-rows')!.textContent = 'GWT_PULL_5016_0_0002';
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    return probe.stop();
  });
  expect(result.lost).toBe(1);
  expect(result.coalesced).toBe(0);
  expect(result.domStats.count).toBe(1, 'a missing update never receives fabricated latency');
  expect(result.frameStats.count).toBe(1);
});

function numericStats(values: number[]) {
  const sorted = [...values].sort((left, right) => left - right);
  return { count: values.length, mean: values.length ? values.reduce((sum, value) => sum + value, 0) / values.length : null,
    p50: sorted[Math.max(0, Math.ceil(values.length * 0.5) - 1)] ?? null,
    p95: sorted[Math.max(0, Math.ceil(values.length * 0.95) - 1)] ?? null,
    max: sorted.at(-1) ?? null };
}

function captureRateSnapshot(readCounter: () => number, now = () => performance.now()) {
  const counter = readCounter();
  return { counter, at: now() };
}

// Fixed numeric buckets keep diagnostics bounded; payloads never enter artifacts.
function createWireCounter() {
  const kinds = ['terminal_output', 'terminal_snapshot', 'terminal_preview', 'pm_reports',
    'workspace_state', 'window_state', 'window_list', 'active_work_projection',
    'active_work_projection_patch', 'runtime_health', 'provider_usage', 'issue_monitor_status',
    'issue_monitor_inbox', 'hub_state', 'pm_status', 'system_status', 'project_index_status',
    'load_pm_reports', 'update_window_geometry', 'update_terminal_grid', 'terminal_input', 'other', 'binary'];
  const counters: Record<'received' | 'sent', Record<string, { bytes: number; frames: number }>> = {
    received: Object.fromEntries(kinds.map(kind => [kind, { bytes: 0, frames: 0 }])),
    sent: Object.fromEntries(kinds.map(kind => [kind, { bytes: 0, frames: 0 }])),
  };
  return {
    record(direction: 'received' | 'sent', payload: string | Buffer) {
      let kind = typeof payload === 'string' ? 'other' : 'binary';
      if (typeof payload === 'string') {
        try { const value = JSON.parse(payload); if (kinds.includes(value?.kind)) kind = value.kind; }
        catch { /* Unclassified text stays in the residual bucket. */ }
      }
      const bucket = counters[direction][kind];
      bucket.bytes += typeof payload === 'string' ? Buffer.byteLength(payload, 'utf8') : payload.length;
      bucket.frames++;
    },
    read: () => structuredClone(counters),
  };
}

function rateStatistics(values: number[], first: { counter: number; at: number }, last: { counter: number; at: number }, scale = 1) {
  return { ...numericStats(values), mean: last.at > first.at && last.counter >= first.counter
    ? scale * (last.counter - first.counter) / ((last.at - first.at) / 1000) : null };
}

type ReportTiming = { sequence: number; deadline: number; started: number; finished: number | null;
  pid: number | null; exit_code: number | null; timed_out: boolean };

function createReportPoster({ binary, project, env, now = () => performance.now() }: {
  binary: string; project: string; env: NodeJS.ProcessEnv; now?: () => number;
}) {
  const records: ReportTiming[] = [];
  const failures = new Set<string>();
  const pending = new Map<ChildProcess, Promise<void>>();
  let closed = false;
  async function stopChild(child: ChildProcess) {
    for (const signal of ['SIGTERM', 'SIGKILL'] as const) {
      if (child.exitCode !== null || child.signalCode !== null) return;
      child.kill(signal);
      const deadline = performance.now() + 10_000;
      while (child.exitCode === null && child.signalCode === null && performance.now() < deadline) {
        await new Promise(resolve => setTimeout(resolve, 100));
      }
    }
    if (child.exitCode === null && child.signalCode === null) throw new Error('Report child cleanup residue');
  }
  function post(sequence: number, deadline: number) {
    if (closed || pending.size >= 7) { failures.add('Reports pending limit or closed'); return; }
    const record: ReportTiming = { sequence, deadline, started: now(), finished: null, pid: null, exit_code: null, timed_out: false };
    records.push(record);
    const child = spawn(binary, [], { cwd: project, env, stdio: ['pipe', 'pipe', 'ignore'] });
    record.pid = child.pid ?? null;
    let output = '', failed = false;
    const done = new Promise<void>(resolve => {
      const timeout = setTimeout(() => {
        record.timed_out = true; failures.add('Reports timeout');
        stopChild(child).catch(() => failures.add('Report child cleanup residue')).finally(resolve);
      }, 30_000);
      child.on('error', () => { failed = true; failures.add('Reports child error'); });
      child.stdout!.on('data', chunk => {
        if (output.length + chunk.length <= 65_536) output += chunk.toString('utf8');
        else { failed = true; failures.add('Reports response bound exceeded'); }
      });
      child.once('close', code => {
        clearTimeout(timeout); record.finished = now(); record.exit_code = code;
        try { if (failed || code !== 0 || !JSON.parse(output).ok) failures.add('Reports post failed'); }
        catch { failures.add('Reports response invalid'); }
        output = ''; pending.delete(child); resolve();
      });
    });
    pending.set(child, done);
    child.stdin!.on('error', () => { failed = true; });
    child.stdin!.end(JSON.stringify({ schema_version: 1, operation: 'pm.report.post', params: {
      kind: 'progress', body: `# Fixed report ${sequence}\n\n${'fixed task '.repeat(40)}` } }));
  }
  return { records, failures, post,
    async finish() { await Promise.all([...pending.values()]); },
    async stop() {
      closed = true;
      for (const child of [...pending.keys()]) {
        try { await stopChild(child); } catch { failures.add('Report child cleanup residue'); }
      }
      if (!failures.has('Report child cleanup residue')) await Promise.all([...pending.values()]);
    },
  };
}

async function waitUntil(deadline: number) {
  while (performance.now() < deadline) await new Promise(resolve => setTimeout(resolve, Math.max(1, deadline - performance.now())));
}

// One asynchronous targeted PS sample per tick supplies GUI CPU and all renderer RSS.
// Whole-host CPU and load are diagnostic covariates only.
function parseTargetProcesses(text) {
  return text.trim().split('\n').filter(Boolean).map(line => {
    const match = line.trim().match(/^(\d+)\s+(.{24})\s+(\S+)\s+(\d+)\s+(\S+)$/);
    if (!match) throw new Error('Target process sample unavailable');
    const [, pid, birth, time, rss, state] = match;
    const [days, clock] = time.includes('-') ? time.split('-') : ['0', time];
    const cpu = Number(days) * 86400 + clock.split(':').reduce((sum, part) => sum * 60 + Number(part), 0);
    if (!Number.isFinite(cpu) || cpu < 0 || state.startsWith('Z') || state.startsWith('?')
      || (process.platform === 'darwin' && state.slice(1).includes('E'))) throw new Error('Target process CPU/RSS unavailable');
    return { pid: Number(pid), birth, cpu, rss: Number(rss) * 1024 };
  });
}
async function targetProcesses(pids: number[]): Promise<{ finish: number; rows: ReturnType<typeof parseTargetProcesses> }> {
  return new Promise((resolveSample, reject) => {
    execFile('ps', ['-p', pids.join(','), '-o', 'pid=,lstart=,time=,rss=,stat='],
      { encoding: 'utf8', timeout: 10_000, maxBuffer: 1024 * 1024, env: { ...process.env, LC_ALL: 'C' } },
      (error, stdout) => {
        const finish = performance.now();
        if (error) { reject(new Error('Measured target process sample failed')); return; }
        try {
          const rows = parseTargetProcesses(stdout);
          if (rows.length !== pids.length || new Set(rows.map(row => row.pid)).size !== pids.length
            || rows.some(row => !pids.includes(row.pid))) throw new Error('Measured target process disappeared');
          resolveSample({ finish, rows });
        } catch (failure) { reject(failure); }
      });
  });
}
function captureHostCounter() {
  const processors = cpus();
  if (!processors.length) throw new Error('Host CPU statistics unavailable');
  let busy_ms = 0, total_ms = 0;
  for (const processor of processors) {
    const { user, nice, sys, idle, irq } = processor.times;
    if (![user, nice, sys, idle, irq].every(value => Number.isFinite(value) && value >= 0)) throw new Error('Host CPU statistics unavailable');
    busy_ms += user + nice + sys + irq; total_ms += user + nice + sys + idle + irq;
  }
  return { at: performance.now(), busy_ms, total_ms, logical_cpus: processors.length };
}
function rendererPids(processInfo) {
  const pids = processInfo.filter(entry => entry.type === 'renderer').map(entry => entry.id).sort((a, b) => a - b);
  if (!pids.length || new Set(pids).size !== pids.length || pids.some(pid => !Number.isInteger(pid) || pid <= 0)) throw new Error('Renderer identities unavailable');
  return pids;
}
function wholeHostCpuRatio(start, end) {
  for (const counter of [start, end]) if (!counter || ![counter.at, counter.busy_ms, counter.total_ms].every(Number.isFinite)
    || counter.busy_ms < 0 || counter.total_ms < counter.busy_ms) throw new Error('Host CPU counter invalid');
  if (!Number.isInteger(start.logical_cpus) || start.logical_cpus <= 0 || end.logical_cpus !== start.logical_cpus) throw new Error('Host CPU count changed');
  if (end.at <= start.at || end.busy_ms < start.busy_ms || end.total_ms < start.total_ms) throw new Error('Host CPU counter decreased');
  const ratio = (end.busy_ms - start.busy_ms) / (end.total_ms - start.total_ms);
  if (!Number.isFinite(ratio) || ratio > 1) throw new Error('Host CPU counter interval invalid');
  return ratio;
}
async function browserClock(page: Page) {
  const before = performance.now();
  const browser = await page.evaluate(() => performance.now());
  return { before, after: performance.now(), browser };
}

async function launchWorkload(page: Page, branch: string, index: number): Promise<string> {
  const cursor = () => page.evaluate(() => (window as any).__gwtPullRenderWizard?.sequence ?? 0);
  const waitWizard = (after: number, expected: Record<string, unknown> = {}) => page.waitForFunction(({ sequence, expected }) => {
    const state = (window as any).__gwtPullRenderWizard;
    return state?.sequence > sequence && !state.busy && Object.entries(expected).every(([key, value]) => state[key] === value) ? state : null;
  }, { sequence: after, expected }, { timeout: 90_000 }).then(handle => handle.jsonValue());
  const action = async (value: any) => {
    const after = await cursor();
    await sendLiveGwtEvent(page, { kind: 'launch_wizard_action', action: value,
      bounds: { x: 30, y: 50, width: 440, height: 350 } });
    return waitWizard(after, value.kind === 'set_agent' ? { agent: value.agent_id }
      : value.kind === 'set_runtime_target' ? { runtime: 'host' }
        : value.kind === 'set_branch_name' ? { branch: value.value }
          : value.kind === 'use_start_method' ? { path: 'manual_setup', phase: 'settings' } : {});
  };
  const after = await cursor();
  await openLiveLaunchWizardForBranch(page, branch);
  await waitWizard(after, { open: true });
  const wizard = page.locator('#wizard-modal');
  await expect(wizard).toBeVisible({ timeout: 60_000 });
  await action({ kind: 'use_start_method', method: 'configure_and_start' });
  await action({ kind: 'set_branch_mode', create_new: true });
  await action({ kind: 'set_branch_name', value: `work/pull-render-${index}` });
  await expect(wizard.getByLabel('Agent', { exact: true })).toBeVisible();
  let state = await action({ kind: 'set_agent', agent_id: `pull-render-${index}` });
  for (let attempt = 0; attempt < 12 && state.open; attempt++) {
    if (state.failed) throw new Error(`Workload ${index} Launch Wizard refused`);
    expect(state.agent).toBe(`pull-render-${index}`);
    if (state.runtime !== 'host' && state.runtime_options.includes('host')) {
      state = await action({ kind: 'set_runtime_target', target: 'Host' });
    } else {
      if (!state.enabled) throw new Error(`Workload ${index} Launch Wizard action disabled`);
      state = await action({ kind: 'submit' });
    }
  }
  expect(state.open).toBe(false);
  await expect(wizard).toBeHidden({ timeout: 90_000 });
  await sendLiveGwtEvent(page, { kind: 'list_windows' });
  const id = await page.waitForFunction(agent => (window as any).__gwtPullRenderWindows?.[agent]?.id,
    `pull-render-${index}`, { timeout: 30_000 }).then(handle => handle.jsonValue());
  const geometry = { x: 30 + (index % 3) * 470, y: 50 + Math.floor(index / 3) * 400, width: 440, height: 350 };
  await sendLiveGwtEvent(page, { kind: 'undock_agent_window', id, geometry });
  await sendLiveGwtEvent(page, { kind: 'update_window_geometry', id, geometry, cols: 50, rows: 16 });
  await expect(page.locator(`.workspace-window[data-id="${id}"] .xterm-rows`)).toBeVisible();
  return id;
}

test('real AgentBootstrap measurement', async ({ page, browser }, info) => {
  test.skip(!process.env.GWT_PULL_RENDER_MANIFEST, 'opt in through measure-pull-render.mjs');
  const manifest = JSON.parse(readFileSync(process.env.GWT_PULL_RENDER_MANIFEST!, 'utf8'));
  test.setTimeout((manifest.seconds + 600) * 1000);
  // The runner chooses exactly one pinned Chromium theme and worker.
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.addInitScript(theme => localStorage.setItem('gwt:ui:theme', theme), manifest.theme);
  await page.addInitScript(installMeasurementProbe);
  let errorCount = 0;
  page.on('pageerror', () => errorCount++);
  page.on('console', message => { if (message.type() === 'error') errorCount++; });
  let measuring = false;
  let bytes = 0;
  let sentBytes = 0;
  const wire = createWireCounter();
  const captureWs = () => {
    const sample = { ...captureRateSnapshot(() => bytes), wire: wire.read() };
    if (Object.values(sample.wire.received).reduce((sum, bucket) => sum + bucket.bytes, 0) !== sample.counter
      || Object.values(sample.wire.sent).reduce((sum, bucket) => sum + bucket.bytes, 0) !== sentBytes) {
      invalidReasons.push('WS kind byte conservation failed');
    }
    return sample;
  };
  page.on('websocket', socket => {
    socket.on('framereceived', ({ payload }) => { if (measuring) {
      bytes += typeof payload === 'string' ? Buffer.byteLength(payload, 'utf8') : payload.length;
      wire.record('received', payload);
    } });
    socket.on('framesent', ({ payload }) => { if (measuring) {
      sentBytes += typeof payload === 'string' ? Buffer.byteLength(payload, 'utf8') : payload.length;
      wire.record('sent', payload);
    } });
    socket.on('socketerror', () => errorCount++);
  });
  const rows: any[] = [];
  const ids: string[] = [];
  let probe: any;
  let complete = false;
  let elapsed = 0;
  let browserPid: number | undefined;
  const invalidReasons: string[] = [];
  let firstWs = { counter: NaN, at: NaN, wire: wire.read() }, lastWs = firstWs;
  let firstCpu = { counter: NaN, at: NaN }, lastCpu = firstCpu;
  let firstCpuProbeDuration = NaN;
  let barrierAt = NaN;
  let finalWsTask: Promise<void> | undefined;
  let finalWsTimer: ReturnType<typeof setTimeout> | undefined;
  let frozenWs: ReturnType<typeof captureWs> | undefined;
  const clockBrackets: any[] = [];
  let poster: ReturnType<typeof createReportPoster> | undefined;
  let reportTask: Promise<void> | undefined;
  let reportsClosed = false;
  const browserCdp = await browser.newBrowserCDPSession();
  const pageCdp = await page.context().newCDPSession(page);
  await pageCdp.send('Performance.enable');
  try {
    await gotoLiveGwt(page, process.env.GWT_PLAYWRIGHT_BASE_URL!, { keepPresetModal: true });
    await page.waitForFunction(() => (window as any).__gwtPlaywrightTestBridgeInstalled === true);
    await expect(page.locator('html')).toHaveAttribute('data-theme', manifest.theme);
    for (let index = 0; index < 5; index++) {
      ids.push(await launchWorkload(page, manifest.branch, index));
      // Wizard closure precedes asynchronous materialization. Wait for this
      // PTY before starting the next Work so Git config writes stay sequential.
      await expect.poll(() => {
        try { return JSON.parse(readFileSync(join(manifest.agentDir, `${index}.json`), 'utf8')).pid > 0; } catch { return false; }
      }, { timeout: 90_000 }).toBe(true);
    }
    const pm = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
    await expect(pm).toHaveCount(1);
    await expect(pm.getByRole('log', { name: 'PM reports' })).toBeVisible();
    const fixedRendererPids = rendererPids((await browserCdp.send('SystemInfo.getProcessInfo')).processInfo);
    await page.evaluate(windowIds => (window as any).__gwtPullRenderProbe.start(windowIds), ids);
    const start = Date.now() + 1000;
    barrierAt = performance.now() + (start - Date.now());
    writeFileSync(manifest.barrier, JSON.stringify({ start, seconds: manifest.seconds }));
    measuring = true;
    await page.waitForTimeout(Math.max(0, start - Date.now()));
    const initialCpuProbeStarted = performance.now();
    const [initialProcesses, initialTargets] = await Promise.all([
      browserCdp.send('SystemInfo.getProcessInfo'), targetProcesses([manifest.gui_pid, ...fixedRendererPids]),
    ]);
    if (JSON.stringify(rendererPids(initialProcesses.processInfo)) !== JSON.stringify(fixedRendererPids)) throw new Error('Measured renderer identity changed');
    browserPid = initialProcesses.processInfo.find((entry: any) => entry.type === 'browser')?.id;
    let previousTargets = initialTargets.rows;
    const first = previousTargets.find((row: any) => row.pid === manifest.gui_pid);
    firstCpu = lastCpu = { counter: first.cpu, at: initialTargets.finish };
    let previousHost = captureHostCounter();
    firstCpuProbeDuration = firstCpu.at - initialCpuProbeStarted;
    firstWs = lastWs = captureWs();
    if (firstWs.at < barrierAt || firstCpu.at < barrierAt) {
      invalidReasons.push('barrier clock mismatch'); throw new Error('Measurement started before its barrier');
    }
    if (firstWs.at - barrierAt >= 1000 || firstCpu.at - barrierAt >= 1000) {
      invalidReasons.push('barrier start missed'); throw new Error('Measurement start missed a sample period');
    }
    const measureStart = barrierAt;
    clockBrackets.push(await browserClock(page));
    let previousWs = firstWs, previousCpu = firstCpu;
    const env = { ...process.env, HOME: manifest.home, USERPROFILE: manifest.home };
    for (const key of Object.keys(env)) if (key.startsWith('GWT_')) delete env[key];
    poster = createReportPoster({ binary: join(ROOT, 'target/debug/gwtd'), project: manifest.project, env });
    finalWsTask = new Promise(resolve => {
      finalWsTimer = setTimeout(async () => { await waitUntil(measureStart + manifest.seconds * 1000); frozenWs = captureWs(); measuring = false; resolve(); },
        Math.max(0, measureStart + manifest.seconds * 1000 - performance.now()));
    });
    reportTask = (async () => {
      for (let index = 0; index < Math.ceil(manifest.seconds / 5); index++) {
        const deadline = measureStart + 1000 + index * 5000;
        await waitUntil(deadline);
        if (reportsClosed) return;
        if (performance.now() >= deadline + 5000) { invalidReasons.push('Reports cadence missed'); return; }
        poster!.post(index + 1, deadline);
      }
    })().catch(() => { invalidReasons.push('Reports scheduler failed'); });
    for (let index = 0; index < manifest.seconds; index++) {
      const deadline = measureStart + (index + 1) * 1000;
      await waitUntil(deadline);
      if (performance.now() >= deadline + 1000) { rows.push({ status: 'missing', deadline }); continue; }
      if (index === manifest.seconds - 1) await finalWsTask;
      const sampleWs = index === manifest.seconds - 1 ? frozenWs! : captureWs();
      lastWs = sampleWs;
      const cpuProbeStarted = performance.now();
      const [processes, table] = await Promise.all([
        browserCdp.send('SystemInfo.getProcessInfo'), targetProcesses([manifest.gui_pid, ...fixedRendererPids]),
      ]);
      const renderers = rendererPids(processes.processInfo);
      if (JSON.stringify(renderers) !== JSON.stringify(fixedRendererPids)) throw new Error('Measured renderer identity changed');
      browserPid = processes.processInfo.find((entry: any) => entry.type === 'browser')?.id;
      if (!browserPid) throw new Error('Dedicated Chromium identity unavailable');
      for (const target of table.rows) if (!previousTargets.some(old => old.pid === target.pid && old.birth === target.birth)) throw new Error('Measured target identity changed');
      const cpu = table.rows.find((row: any) => row.pid === manifest.gui_pid);
      lastCpu = { counter: cpu.cpu, at: table.finish };
      previousTargets = table.rows;
      const cpuProbeDuration = lastCpu.at - cpuProbeStarted;
      const metrics = await pageCdp.send('Performance.getMetrics');
      const heap = metrics.metrics.find((entry: any) => entry.name === 'JSHeapUsedSize')?.value;
      if (!Number.isFinite(heap)) throw new Error('Renderer heap unavailable');
      const retention = await page.evaluate(windowIds => ({
        report_count: document.querySelectorAll('.pm-chat article').length,
        report_text_bytes: Array.from(document.querySelectorAll('.pm-chat__text')).reduce((sum, node) =>
          sum + new TextEncoder().encode(node.textContent ?? '').byteLength, 0),
        terminal_buffer_lines: windowIds.map(id => (window as any).__gwtTerminalTestApi.metrics(id).bufferLength),
      }), ids);
      const rendererRss = renderers.reduce((sum: number, pid: number) => {
        const row = table.rows.find((entry: any) => entry.pid === pid);
        if (!row) throw new Error('Measured renderer disappeared');
        return sum + row.rss;
      }, 0);
      const currentHost = captureHostCounter();
      const host = { method: 'whole_host_cpu_covariates_v1', start: previousHost, end: currentHost,
        whole_host_cpu_ratio: wholeHostCpuRatio(previousHost, currentHost), loadavg: loadavg() };
      previousHost = currentHost;
      const collectedAt = performance.now();
      rows.push({ status: collectedAt >= deadline + 1000 ? 'overrun' : 'observed', elapsed: (sampleWs.at - measureStart) / 1000,
        deadline, ws_at: sampleWs.at, ws_counter: sampleWs.counter, ws_wire: sampleWs.wire, cpu_at: lastCpu.at, cpu_counter: lastCpu.counter,
        cpu_probe_duration_ms: cpuProbeDuration, collected_at: collectedAt, host,
        backend_cpu_percent: (lastCpu.counter - previousCpu.counter) / ((lastCpu.at - previousCpu.at) / 1000) * 100,
        renderer_rss_bytes: rendererRss,
        renderer_heap_bytes: heap, ws_payload_bytes_per_second: (sampleWs.counter - previousWs.counter) / ((sampleWs.at - previousWs.at) / 1000), renderer_pids: renderers, ...retention });
      previousWs = sampleWs; previousCpu = lastCpu;
    }
    await finalWsTask;
    lastWs = frozenWs!;
    elapsed = (lastWs.at - barrierAt) / 1000;
    clockBrackets.push(await browserClock(page));
    await reportTask;
    await poster.finish();
    await expect.poll(() => Array.from({ length: 5 }, (_, index) =>
      JSON.parse(readFileSync(join(manifest.agentDir, `${index}.json`), 'utf8')).complete).every(Boolean), { timeout: 30_000 }).toBe(true);
    await expect.poll(() => page.evaluate(() => (window as any).__gwtPullRenderProbe ? true : false)).toBe(true);
    await page.waitForTimeout(1000);
    complete = true;
  } finally {
    if (finalWsTimer) clearTimeout(finalWsTimer);
    measuring = false;
    reportsClosed = true;
    await reportTask;
    await poster?.stop();
    invalidReasons.push(...(poster?.failures ?? []));
    probe = await page.evaluate(() => (window as any).__gwtPullRenderProbe.stop()).catch(() => null);
    const workload = Array.from({ length: 5 }, (_, index) => {
      let record: any = {}; try { record = JSON.parse(readFileSync(join(manifest.agentDir, `${index}.json`), 'utf8')); } catch { /* incomplete evidence */ }
      return { index, pid: record.pid ?? null, sequence: record.sequence ?? 0, bytes: record.bytes ?? 0,
        received_sequence: probe?.received[index] ?? 0,
        dom_sequence: probe?.dom[index] ?? 0, render_sequence: probe?.rendered[index] ?? 0 };
    });
    const visualPass = complete && invalidReasons.length === 0 && errorCount === 0 && probe?.lost === 0 && probe?.dropped === 0
      && probe?.domStats?.count === manifest.seconds * 5 && probe?.frameStats?.count === manifest.seconds * 5
      && rows.length === manifest.seconds && workload.every(record => record.sequence === manifest.seconds
        && record.dom_sequence === manifest.seconds && record.render_sequence === manifest.seconds);
    const counts = { observed: 0, overrun: 0, missing: 0 };
    for (const row of rows) counts[row.status as keyof typeof counts]++;
    const sampleStatus = { ...counts, missing_fraction: counts.missing / manifest.seconds, overrun_fraction: counts.overrun / manifest.seconds };
    const measuredRows = rows.filter(row => row.status !== 'missing');
    const report = { sample_status: sampleStatus, schema_version: 1, mode: manifest.mode, comparison_rule: 'observed_slot_coverage_host_covariates_v1', complete, seconds: manifest.seconds, elapsed_seconds: elapsed,
      agents: 5, source_sha: manifest.source_sha, binary_sha256: manifest.binary_sha256, gui_pid: manifest.gui_pid,
      source_diff_sha256: manifest.source_diff_sha256, source_dirty: manifest.source_dirty,
      browser_pid: browserPid ?? null, recorded_at: new Date().toISOString(),
      environment: { host: manifest.host, platform: manifest.platform, architecture: manifest.architecture,
        host_cpu_method: 'whole_host_cpu_covariates_v1', browser: browser.version(), theme: manifest.theme, viewport: '1440x1000', profile: manifest.profile,
        workload: manifest.workload_hash, harness: manifest.harness_hash, fixture: manifest.fixture_hash, contains_5335: manifest.contains_5335 },
      workload, reports_posted: poster?.records.filter(record => record.exit_code === 0 && !record.timed_out).length ?? 0,
      report_timings: poster?.records ?? [], ws_payload_bytes: lastWs.counter - firstWs.counter, samples: rows.length,
      timing: { barrier_at: barrierAt, browser_clock: clockBrackets,
        ws: { start: firstWs.at, end: lastWs.at, counter_start: firstWs.counter, counter_end: lastWs.counter,
          start_lateness_ms: firstWs.at - barrierAt, wire_start: firstWs.wire, wire_end: lastWs.wire },
        cpu: { start: firstCpu.at, end: lastCpu.at, counter_start: firstCpu.counter, counter_end: lastCpu.counter,
          initial_probe_duration_ms: firstCpuProbeDuration, start_lateness_ms: firstCpu.at - barrierAt } },
      host_cpu_semantics: 'Whole-host busy/total CPU ratio and loadavg are numeric covariates only. Missing and overrun slots remain explicit; common observed coverage is required; host conditions never exclude data or change strict metric acceptance.',
      window_metadata: probe?.window_metadata ?? [],
      metrics: { ...Object.fromEntries(['renderer_rss_bytes', 'renderer_heap_bytes']
        .map(name => [name, numericStats(measuredRows.map(row => (row as any)[name]))])),
        backend_cpu_percent: rateStatistics(measuredRows.map(row => row.backend_cpu_percent), firstCpu, lastCpu, 100),
        ws_payload_bytes_per_second: rateStatistics(measuredRows.map(row => row.ws_payload_bytes_per_second), firstWs, lastWs),
        dom_latency_ms: probe?.domStats ?? null, render_frame_latency_ms: probe?.frameStats ?? null },
      long_tasks: probe?.long_tasks ?? null, errors: errorCount,
      lost_markers: probe?.lost ?? 1, dropped_pending: probe?.dropped ?? 1, raw: { seconds: rows, latency: probe?.raw ?? [], long_tasks: probe?.long_raw ?? [] },
      coalesced_updates: probe?.coalesced ?? 0, invalid_reasons: [...new Set(invalidReasons)], evidence_valid: false, cleanup_success: false,
      latency_semantics: 'native WS message -> latest actual xterm DOM commit of ordered pending updates (including coalesced updates) -> next rAF callback; performance.now clock; no compositor/presentation claim',
      UserVerificationResult: 'n/a (autonomous)', AgentVisualCheck: visualPass ? 'pass' : 'fail', ac17_pass: false };
    writeFileSync(manifest.out, JSON.stringify(report));
    await browserCdp.detach(); await pageCdp.detach();
  }
  expect(errorCount).toBe(0);
  expect(probe.lost).toBe(0); expect(probe.dropped).toBe(0);
  expect(probe.dom).toEqual(Array(5).fill(manifest.seconds));
  expect(probe.rendered).toEqual(Array(5).fill(manifest.seconds));
  expect(rows).toHaveLength(manifest.seconds);
});
