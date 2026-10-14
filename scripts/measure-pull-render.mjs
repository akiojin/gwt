#!/usr/bin/env node
// Issue #5016 SAFE-T4. Build this checkout first. Full runs are always 1800s.
// node scripts/measure-pull-render.mjs --mode before --out /tmp/before.json --contains-5335 yes
// node scripts/measure-pull-render.mjs --mode after --baseline /tmp/before.json --out /tmp/after.json --contains-5335 yes
// --mode smoke --seconds 10 checks the harness only; it can never pass AC-17.
import { spawn, spawnSync } from 'node:child_process';
import { constants } from 'node:fs';
import { access, mkdir, mkdtemp, readFile, realpath, symlink, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { hostname, homedir, tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { setTimeout as delay } from 'node:timers/promises';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export function parseOptions(args) {
  const { values } = parseArgs({ args, options: {
    mode: { type: 'string', default: 'before' }, seconds: { type: 'string' },
    theme: { type: 'string', default: 'dark' }, out: { type: 'string' },
    baseline: { type: 'string' }, 'contains-5335': { type: 'string', default: 'unknown' },
    'validate-only': { type: 'boolean', default: false },
  } });
  const seconds = Number(values.seconds ?? (values.mode === 'smoke' ? '10' : '1800'));
  if (!['before', 'after', 'smoke'].includes(values.mode)) throw new Error('mode must be before, after or smoke');
  if (!Number.isInteger(seconds) || seconds < 1 || seconds > 1800
    || (values.mode !== 'smoke' && seconds !== 1800)) throw new Error('Full runs require exactly 1800 seconds; use --mode smoke for shorter runs');
  if (!['dark', 'light'].includes(values.theme)) throw new Error('theme must be dark or light');
  if (!['yes', 'no', 'unknown'].includes(values['contains-5335'])) throw new Error('contains-5335 must be yes or no');
  return { mode: values.mode, seconds, agents: 5, theme: values.theme, out: values.out,
    baseline: values.baseline, contains_5335: values['contains-5335'],
    validate_only: values['validate-only'], ac17_pass: false };
}

function statistics(values) {
  const sorted = [...values].sort((a, b) => a - b);
  return { count: values.length, mean: values.reduce((sum, value) => sum + value, 0) / values.length,
    p50: sorted[Math.ceil(values.length * 0.5) - 1], p95: sorted[Math.ceil(values.length * 0.95) - 1], max: sorted.at(-1) };
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

export function pairedComparison(before, after) {
  const runs = [before, after];
  for (const run of runs) {
    if (run?.comparison_rule !== 'observed_slot_coverage_host_covariates_v1' || run?.seconds !== 1800 || run.raw?.seconds?.length !== 1800) throw new Error('Shared host metadata missing');
    let previousHostEnd;
    const counts = { observed: 0, overrun: 0, missing: 0 };
    for (const row of run.raw.seconds) {
      if (!Object.hasOwn(counts, row.status)) throw new Error('Sample status metadata invalid');
      counts[row.status]++;
      if (row.status === 'missing') {
        if (Object.keys(row).some(key => !['status', 'deadline'].includes(key))) throw new Error('Missing sample contains measured values');
        continue;
      }
      const host = row.host;
      if (!['renderer_rss_bytes', 'renderer_heap_bytes'].every(key => Number.isFinite(row[key]))) throw new Error('Selected metric metadata missing');
      if (!host || host.method !== 'whole_host_cpu_covariates_v1' || !Array.isArray(host.loadavg) || host.loadavg.length !== 3
        || !host.loadavg.every(value => Number.isFinite(value) && value >= 0)) throw new Error('Shared host metadata missing');
      let ratio;
      try { ratio = wholeHostCpuRatio(host.start, host.end); }
      catch (error) { throw new Error(`Shared host metadata invalid: ${error.message}`); }
      if (!Number.isFinite(host.whole_host_cpu_ratio) || Math.abs(host.whole_host_cpu_ratio - ratio) > 1e-12
        || (previousHostEnd && JSON.stringify(previousHostEnd) !== JSON.stringify(host.start))) throw new Error('Shared host metadata inconsistent');
      previousHostEnd = host.end;
    }
    const summary = { ...counts, missing_fraction: counts.missing / run.seconds, overrun_fraction: counts.overrun / run.seconds };
    if (!run.sample_status || Object.entries(summary).some(([key, value]) => run.sample_status[key] !== value)) throw new Error('Sample status counts inconsistent');
  }
  const mask = before.raw.seconds.map((row, k) => row.status === 'observed' && after.raw.seconds[k].status === 'observed');
  const evidence = runs.map(run => {
    const barrier = run.timing.barrier_at;
    const brackets = run.timing.browser_clock;
    if (!Array.isArray(brackets) || brackets.length !== 2 || brackets.some(b => ![b.before, b.after, b.browser].every(Number.isFinite) || b.after < b.before)) throw new Error('Browser clock metadata invalid');
    const low = Math.min(...brackets.map(b => b.before - b.browser));
    const high = Math.max(...brackets.map(b => b.after - b.browser));
    if (!Number.isFinite(barrier) || high - low >= 1000) throw new Error('Browser clock uncertainty invalid');
    const metrics = {}, coverage = {};
    for (const [kind, metric, scale] of [['ws', 'ws_payload_bytes_per_second', 1], ['cpu', 'backend_cpu_percent', 100]]) {
      let previous = run.timing[kind].start, counter = run.timing[kind].counter_start ?? 0;
      const bands = Array(6).fill(0), values = [];
      let duration = 0, delta = 0;
      run.raw.seconds.forEach((row, k) => {
        if (row.status === 'missing') return;
        const at = row[`${kind}_at`], next = row[`${kind}_counter`];
        if (![previous, counter, at, next].every(Number.isFinite) || at <= previous || next < counter) throw new Error('Selected counter metadata invalid');
        const overlapsMissing = mask.some((keep, slot) => !keep
          && Math.min(at, barrier + (slot + 1) * 1000) > Math.max(previous, barrier + slot * 1000));
        if (mask[k] && (k === 0 || mask[k - 1]) && !overlapsMissing) {
          const span = at - previous;
          duration += span; delta += next - counter; values.push(scale * (next - counter) / (span / 1000));
          for (let band = 0; band < 6; band++) bands[band] += Math.max(0, Math.min(at, barrier + (band + 1) * 300000) - Math.max(previous, barrier + band * 300000));
        }
        previous = at; counter = next;
      });
      if (bands.reduce((sum, value) => sum + value, 0) < 1620000 || bands.some(value => value < 270000)) throw new Error('Retained time coverage insufficient');
      coverage[kind] = { retained_ms: bands.reduce((sum, value) => sum + value, 0), actual_interval_ms: duration, bands_ms: bands };
      metrics[metric] = { ...statistics(values), mean: scale * delta / (duration / 1000) };
    }
    for (const name of ['renderer_rss_bytes', 'renderer_heap_bytes']) {
      const values = run.raw.seconds.filter((_, k) => mask[k]).map(row => row[name]);
      if (!values.every(Number.isFinite)) throw new Error('Selected metric metadata missing');
      metrics[name] = statistics(values);
    }
    const latency = run.raw.latency;
    if (!Array.isArray(latency) || latency.length !== 9000) throw new Error('Latency tuples incomplete');
    const seen = new Set(), retained = Array(5).fill(0), updateBands = Array.from({ length: 5 }, () => Array(6).fill(0)), selected = [];
    for (const row of latency) {
      const key = `${row.index}:${row.sequence}`;
      if (!Number.isInteger(row.index) || row.index < 0 || row.index > 4 || !Number.isInteger(row.sequence) || row.sequence < 1 || row.sequence > 1800
        || seen.has(key) || ![row.receive, row.dom, row.frame].every(Number.isFinite) || row.dom < row.receive || row.frame < row.dom) throw new Error('Latency tuples incomplete');
      seen.add(key);
      if (mask[row.sequence - 1]) { selected.push(row); retained[row.index]++; updateBands[row.index][Math.floor((row.sequence - 1) / 300)]++; }
    }
    if (retained.some(value => value < 1620) || updateBands.some(bands => bands.some(value => value < 270))) throw new Error('Retained update coverage insufficient');
    coverage.pane_updates = retained;
    coverage.pane_update_bands = updateBands;
    coverage.sample_status = run.sample_status;
    for (const [name, key] of [['dom_latency_ms', 'dom'], ['render_frame_latency_ms', 'frame']]) {
      const values = selected.map(row => row[key] - row.receive);
      metrics[name] = { ...statistics(values), p50: Math.ceil(statistics(values).p50), p95: Math.ceil(statistics(values).p95) };
    }
    const tasks = run.raw.long_tasks;
    if (!Array.isArray(tasks) || tasks.length > 40960) throw new Error('Long task metadata invalid');
    if (tasks.length !== run.long_tasks?.count || tasks.reduce((sum, pair) => sum + (pair?.[1] ?? NaN), 0) !== run.long_tasks?.duration_ms) throw new Error('Long task aggregate mismatch');
    const longTasks = { count: 0, duration_ms: 0 }, windowTasks = { count: 0, duration_ms: 0 };
    for (const pair of tasks) {
      if (!Array.isArray(pair) || pair.length !== 2 || !pair.every(Number.isFinite) || pair[1] < 0) throw new Error('Long task metadata invalid');
      const first = Math.floor((pair[0] + low - barrier) / 1000), last = Math.floor((pair[0] + high - barrier) / 1000);
      if (last >= 0 && first < 1800) { windowTasks.count++; windowTasks.duration_ms += pair[1]; }
      if (last >= 0 && first < 1800) { longTasks.count++; longTasks.duration_ms += pair[1]; }
    }
    return { metrics, long_tasks: longTasks, window_long_tasks: windowTasks, coverage };
  });
  return { rule: 'observed_slot_coverage_host_covariates_v1; common observed slots; missing-crossing intervals excluded whole; host CPU/load are covariates only', mask, runs: evidence };
}

export function validateMeasurementTiming(run) {
  const reasons = new Set();
  const ws = run?.timing?.ws, cpu = run?.timing?.cpu;
  const rows = run?.raw?.seconds, reports = run?.report_timings;
  if (!ws || !cpu || !Array.isArray(rows) || !Array.isArray(reports)) return ['timing metadata missing'];
  const finite = (...values) => values.every(Number.isFinite);
  const barrier = run.timing.barrier_at;
  if (!finite(barrier, ws.start_lateness_ms, cpu.start_lateness_ms)) reasons.add('timing metadata missing');
  if (ws.start < barrier || cpu.start < barrier) reasons.add('barrier clock mismatch');
  if (ws.start - barrier >= 1000 || cpu.start - barrier >= 1000) reasons.add('barrier start missed');
  if (Math.abs(ws.start_lateness_ms - (ws.start - barrier)) > 1e-6
    || Math.abs(cpu.start_lateness_ms - (cpu.start - barrier)) > 1e-6) reasons.add('barrier clock mismatch');
  if (!Number.isFinite(cpu.initial_probe_duration_ms) || cpu.initial_probe_duration_ms < 0) reasons.add('timing metadata missing');
  for (const window of [ws, cpu]) if (!finite(window.start, window.end, window.counter_start, window.counter_end)
    || window.end <= window.start || window.counter_end < window.counter_start) reasons.add('clock/counter mismatch');
  if (rows.length !== run.samples || rows.length !== run.seconds) reasons.add('sample cadence missed');
  let previousWs = ws.start, previousCpu = cpu.start, wsCounter = ws.counter_start, cpuCounter = cpu.counter_start;
  rows.forEach((row, index) => {
    const deadline = barrier + (index + 1) * 1000;
    if (!['observed', 'overrun', 'missing'].includes(row.status) || row.deadline !== deadline) reasons.add('sample status invalid');
    if (row.status === 'missing') return;
    if (!finite(row.deadline, row.ws_at, row.ws_counter, row.cpu_at, row.cpu_counter, row.cpu_probe_duration_ms, row.collected_at)
      || row.ws_at <= previousWs || row.cpu_at <= previousCpu || row.cpu_at < row.ws_at
      || row.collected_at < row.cpu_at || row.cpu_probe_duration_ms < 0
      || row.ws_counter < wsCounter || row.cpu_counter < cpuCounter) reasons.add('clock/counter mismatch');
    if (row.ws_at < deadline || row.ws_at >= deadline + 1000
      || (row.status === 'observed' && row.collected_at >= deadline + 1000)
      || (row.status === 'overrun' && row.collected_at < deadline + 1000)) reasons.add('sample cadence missed');
    previousWs = row.ws_at; previousCpu = row.cpu_at; wsCounter = row.ws_counter; cpuCounter = row.cpu_counter;
  });
  const equal = (left, right) => Number.isFinite(left) && Number.isFinite(right)
    && Math.abs(left - right) <= 1e-9 * Math.max(1, Math.abs(left), Math.abs(right));
  if (ws.end < barrier + run.seconds * 1000 || ws.end >= barrier + (run.seconds + 1) * 1000
    || wsCounter > ws.counter_end || previousWs > ws.end
    || !equal(run.ws_payload_bytes, ws.counter_end - ws.counter_start)
    || !equal(run.metrics?.ws_payload_bytes_per_second?.mean, (ws.counter_end - ws.counter_start) / ((ws.end - ws.start) / 1000))) {
    reasons.add('WS counter/window mismatch');
  }
  if (!equal(previousCpu, cpu.end) || !equal(cpuCounter, cpu.counter_end)
    || !equal(run.metrics?.backend_cpu_percent?.mean, 100 * (cpu.counter_end - cpu.counter_start) / ((cpu.end - cpu.start) / 1000))) reasons.add('CPU counter/window mismatch');
  if (reports.length !== Math.ceil(run.seconds / 5) || reports.length !== run.reports_posted) reasons.add('Reports metadata incomplete');
  reports.forEach((report, index) => {
    const deadline = barrier + 1000 + index * 5000;
    if (!finite(report.deadline, report.started, report.finished, report.pid) || report.sequence !== index + 1
      || report.deadline !== deadline || report.started < deadline || report.started >= deadline + 5000) reasons.add('Reports cadence missed');
    if (report.finished < report.started || report.finished > ws.end || report.exit_code !== 0 || report.timed_out !== false) reasons.add('Reports completion/window mismatch');
    if (report.finished - report.started > 30_000) reasons.add('Reports timeout');
  });
  return [...reasons];
}

export function compareMeasurements(before, after) {
  const failures = [];
  for (const [label, run, mode] of [['baseline', before, 'before'], ['after', after, 'after']]) {
    if (run?.evidence_valid !== true || run?.cleanup_success !== true || run?.invalid_reasons?.length !== 0) failures.push(`${label} evidence invalid`);
    failures.push(...validateMeasurementTiming(run).map(reason => `${label} ${reason}`));
    if (!run || run.schema_version !== 1 || run.mode !== mode || run.complete !== true || run.seconds !== 1800
      || !Number.isFinite(run.elapsed_seconds) || run.elapsed_seconds < 1800 || run.agents !== 5
      || !Number.isInteger(run.samples) || run.samples < 1800 || run.errors !== 0
      || run.lost_markers !== 0 || run.dropped_pending !== 0) failures.push(`${label} is not a complete 1800-second ${mode} run`);
    if (run?.workload?.length !== 5 || run.workload.some((entry, index) => entry.index !== index
      || !Number.isInteger(entry.pid) || entry.pid <= 0 || entry.sequence !== 1800
      || entry.bytes !== 1800 * 8192 || entry.dom_sequence !== 1800 || entry.render_sequence !== 1800)
      || new Set(run?.workload?.map(entry => entry.pid)).size !== 5) {
      failures.push(`${label} workload is incomplete`);
    }
    if (!/^[a-f0-9]{40}$/.test(run?.source_sha ?? '') || !/^[a-f0-9]{64}$/.test(run?.source_diff_sha256 ?? '')
      || !/^[a-f0-9]{64}$/.test(run?.binary_sha256 ?? '')) {
      failures.push(`${label} source/build identity is missing`);
    }
    if (!/^[a-f0-9]{64}$/.test(run?.environment?.workload ?? '') || !/^[a-f0-9]{64}$/.test(run?.environment?.harness ?? '')) {
      failures.push(`${label} workload/harness identity is missing`);
    }
    if (!/^[a-f0-9]{64}$/.test(run?.environment?.fixture ?? '')) failures.push(`${label} fixture identity is missing`);
    if (!(run?.metrics?.dom_latency_ms?.count >= 9000) || !(run?.metrics?.render_frame_latency_ms?.count >= 9000)) {
      failures.push(`${label} latency observations are incomplete`);
    }
    if (run?.reports_posted !== 360) failures.push(`${label} Reports workload is incomplete`);
  }
  if (JSON.stringify(before?.environment) !== JSON.stringify(after?.environment)) failures.push('environment/workload mismatch');
  if (before?.reports_posted !== after?.reports_posted) failures.push('Reports workload mismatch');
  let paired;
  try { paired = pairedComparison(before, after); }
  catch (error) { failures.push(error.message); return failures; }
  const [selectedBefore, selectedAfter] = paired.runs;
  after.filtered_comparison = paired;
  for (const name of ['backend_cpu_percent', 'renderer_rss_bytes', 'renderer_heap_bytes',
    'ws_payload_bytes_per_second', 'dom_latency_ms', 'render_frame_latency_ms']) {
    for (const statistic of ['mean', 'p50', 'p95', 'max']) {
      const previous = selectedBefore.metrics[name][statistic];
      const current = selectedAfter.metrics[name][statistic];
      if (!Number.isFinite(previous) || !Number.isFinite(current)
        || !(before.metrics[name].count > 0) || !(after.metrics[name].count > 0)) failures.push(`${name}.${statistic} missing`);
      else if (current > previous) failures.push(`${name}.${statistic} regressed`);
    }
  }
  if (!Number.isFinite(before?.ws_payload_bytes) || !Number.isFinite(after?.ws_payload_bytes)) failures.push('WS payload bytes missing');
  for (const key of ['count', 'duration_ms']) if (!Number.isFinite(before?.long_tasks?.[key]) || !Number.isFinite(after?.long_tasks?.[key])
    || selectedAfter.long_tasks[key] > selectedBefore.long_tasks[key]) failures.push(`long_tasks.${key} regressed or missing`);
  return failures;
}

function operation(binary, name, params, env, cwd = ROOT) {
  const result = spawnSync(binary, [], { cwd, env, encoding: 'utf8', timeout: 30_000,
    input: JSON.stringify({ schema_version: 1, operation: name, params }) });
  if (result.status !== 0) throw new Error(`${name} failed (exit ${result.status})`);
  const envelope = JSON.parse(result.stdout);
  if (!envelope.ok) throw new Error(`${name} refused`);
  return { envelope, value: typeof envelope.output === 'string' && name !== 'hook.register_codex_managed_hook_trust'
    ? JSON.parse(envelope.output) : envelope.output };
}

export const WORKLOAD_SOURCE = String.raw`const fs = require('node:fs');
const path = require('node:path');
const index = Number(process.argv[2]);
const directory = process.env.GWT_PULL_AGENT_DIR;
const record = { index, pid: process.pid, sequence: 0, bytes: 0, complete: false };
function save() { const file = path.join(directory, index + '.json'); fs.writeFileSync(file + '.tmp', JSON.stringify(record)); fs.renameSync(file + '.tmp', file); }
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
(async () => {
  save();
  while (!fs.existsSync(process.env.GWT_PULL_BARRIER)) await sleep(100);
  const { start, seconds } = JSON.parse(fs.readFileSync(process.env.GWT_PULL_BARRIER, 'utf8'));
  for (let sequence = 1; sequence <= seconds; sequence++) {
    await sleep(Math.max(0, start + (sequence - 1) * 1000 - Date.now()));
    const marker = 'GWT_PULL_5016_' + index + '_' + String(sequence).padStart(4, '0');
    const line = prefix => prefix + 'x'.repeat(510 - prefix.length) + '\r\n';
    const output = line('fixed task ').repeat(15) + line(marker + ' ');
    if (!process.stdout.write(output)) await new Promise(resolve => process.stdout.once('drain', resolve));
    record.sequence = sequence; record.bytes += Buffer.byteLength(output); save();
  }
  await sleep(Math.max(0, start + seconds * 1000 - Date.now()));
  record.complete = true; save();
})().catch(() => { process.exitCode = 1; });
`;

// Owns only this measurement's GUI, test process and independently grouped PTYs.
export function createMeasurementCleanup({ guiChild, getTestChild, agentDir, workloadFile }) {
  let cleanupPromise;
  async function stopChild(owned, group = false) {
    if (!owned?.pid) return;
    const reaped = () => owned.exitCode !== null || owned.signalCode !== null;
    // The detached test child's PID is the exact PGID created by this runner.
    // Reaping its leader does not prove its Report CLI/Chromium descendants exited.
    const groupExists = () => {
      try { process.kill(-owned.pid, 0); return true; }
      catch (error) {
        if (error.code === 'ESRCH') return false;
        const diagnostic = { stage: 'group_probe', signal: 0, pgid: owned.pid, reaped: reaped() };
        // Darwin killpg may return EPERM for an empty pgrp during removal.
        // Prove disappearance independently; resident members and probe errors fail closed.
        if (error.code === 'EPERM' && reaped()) {
          const snapshot = spawnSync('ps', ['-axo', 'pgid=,pid='],
            { encoding: 'utf8', timeout: 2000, env: { ...process.env, LC_ALL: 'C' } });
          const rows = snapshot.stdout?.trim().split('\n').map(line => line.trim().match(/^(\d+)\s+(\d+)$/));
          Object.assign(diagnostic, { ps_status: snapshot.status ?? -1,
            member_count: rows ? rows.filter(row => row && Number(row[1]) === owned.pid).length : -1,
            observer_present: Boolean(rows?.some(row => row && Number(row[2]) === process.pid)),
            malformed_count: rows ? rows.filter(row => !row).length : -1 });
          if (!snapshot.error && snapshot.status === 0 && rows?.length && rows.every(Boolean)
            && rows.some(row => Number(row[2]) === process.pid)
            && !rows.some(row => Number(row[1]) === owned.pid)) return false;
        }
        throw new Error(`${error.message}; ${JSON.stringify(diagnostic)}`);
      }
    };
    const finished = () => group ? !groupExists() && reaped() : reaped();
    if (finished()) return;
    const send = signal => {
      try { if (group) process.kill(-owned.pid, signal); else owned.kill(signal); }
      catch (error) { if (error.code !== 'ESRCH') throw new Error(`${error.message}; ${JSON.stringify({
        stage: 'send', signal: signal === 'SIGTERM' ? 15 : 9, pid: owned.pid, group, reaped: reaped(),
      })}`); }
    };
    for (const signal of ['SIGTERM', 'SIGKILL']) {
      send(signal);
      const deadline = Date.now() + 10_000;
      while (!finished() && Date.now() < deadline) await delay(100);
      if (finished()) return;
    }
    throw new Error(group ? 'Owned measurement process group did not disappear and reap' : 'Owned measurement child did not exit and reap');
  }
  function agentIdentity(pid) {
    const row = spawnSync('ps', ['-ww', '-p', String(pid), '-o', 'lstart=,command='],
      { encoding: 'utf8', timeout: 2000, env: { ...process.env, LC_ALL: 'C' } });
    if (row.status === 1 && !row.stdout?.trim() && !row.stderr?.trim()) {
      try { process.kill(pid, 0); } catch (error) { if (error.code === 'ESRCH') return null; throw error; }
    }
    if (row.error || row.status !== 0 || row.stdout.trimEnd().length <= 24) throw new Error(`Owned PTY ${pid} identity probe failed`);
    const line = row.stdout.trimEnd();
    return { started: line.slice(0, 24), command: line.slice(24).trim() };
  }
  async function stopAgent(index) {
    let record;
    try { record = JSON.parse(await readFile(path.join(agentDir, `${index}.json`), 'utf8')); }
    catch (error) { if (error.code === 'ENOENT') return; throw new Error(`Owned PTY ${index} metadata unreadable`); }
    if (record.index !== index || !Number.isInteger(record.pid) || record.pid <= 1) throw new Error(`Owned PTY ${index} metadata invalid`);
    const pid = record.pid;
    const expected = `${process.execPath} ${workloadFile} ${index}`;
    const initial = agentIdentity(pid);
    if (!initial || initial.command !== expected) return;
    for (const signal of ['SIGTERM', 'SIGKILL']) {
      const current = agentIdentity(pid);
      if (!current || current.started !== initial.started) return;
      if (current.command !== expected) throw new Error(`Owned PTY ${pid} changed identity during cleanup`);
      try { process.kill(pid, signal); } catch (error) { if (error.code === 'ESRCH') return; throw error; }
      const deadline = Date.now() + 10_000;
      while (Date.now() < deadline) {
        await delay(100);
        const remaining = agentIdentity(pid);
        if (!remaining || remaining.started !== initial.started) return;
      }
    }
    throw new Error(`Owned PTY ${pid} did not disappear after SIGKILL`);
  }
  return function cleanup() {
    return cleanupPromise ??= (async () => {
      const failures = [];
      for (const [owned, group] of [[getTestChild(), true], [guiChild, false]]) {
        try { await stopChild(owned, group); } catch (error) { failures.push(`Owned child cleanup failed: ${error.message}`); }
      }
      for (let index = 0; index < 5; index++) {
        try { await stopAgent(index); } catch (error) { failures.push(`Owned PTY cleanup failed: ${error.message}`); }
      }
      if (failures.length) throw new Error(failures.join('; '));
    })();
  };
}

async function run(options) {
  if (!options.out) throw new Error('--out is required');
  if (options.mode === 'after' && !options.baseline) throw new Error('--baseline is required for after');
  if (options.mode !== 'smoke' && options.contains_5335 === 'unknown') throw new Error('Record --contains-5335 yes|no before a full run');
  if (process.platform !== 'darwin' && process.platform !== 'linux') throw new Error('OS CPU/RSS collection currently requires macOS or Linux');
  const gwt = path.join(ROOT, 'target/debug/gwt');
  const gwtd = path.join(ROOT, 'target/debug/gwtd');
  await Promise.all([access(gwt, constants.X_OK), access(gwtd, constants.X_OK)]);
  const home = await mkdtemp(path.join(tmpdir(), 'gwt-pull-render-'));
  if (await realpath(home) === await realpath(homedir())) throw new Error('HOME isolation failed');
  const state = path.join(home, '.gwt');
  const agentDir = path.join(home, 'agents');
  await Promise.all([mkdir(state), mkdir(agentDir)]);
  // Initial fixture input only. gwt's Launch materialization owns all five
  // work branches/worktrees; this runner never creates or switches those.
  const project = path.join(home, 'project');
  const fixtureText = 'Fixed pull-render workload input. Sixteen 512-byte lines per second, five agents.\n';
  await mkdir(project);
  await writeFile(path.join(project, 'TASK.txt'), fixtureText);
  const git = args => {
    const result = spawnSync('git', args, { cwd: project, encoding: 'utf8', timeout: 30_000,
      env: { ...process.env, GIT_AUTHOR_DATE: '2000-01-01T00:00:00Z', GIT_COMMITTER_DATE: '2000-01-01T00:00:00Z' } });
    if (result.status !== 0) throw new Error('Measurement Git fixture preparation failed');
    return result.stdout.trim();
  };
  git(['init', '--quiet', '--initial-branch=develop']);
  git(['add', 'TASK.txt']);
  git(['-c', 'user.name=gwt measurement', '-c', 'user.email=measurement@gwt.invalid', '-c', 'core.hooksPath=/dev/null',
    'commit', '--quiet', '--no-gpg-sign', '-m', 'Fixed measurement input']);
  const fixtureCommit = git(['rev-parse', 'HEAD']);
  // Start Work fetches origin/develop. This fixed, local-only bare repository
  // contains the same single commit and never contacts a network remote.
  const fixtureOrigin = path.join(home, 'origin.git');
  git(['clone', '--quiet', '--bare', '.', fixtureOrigin]);
  git(['remote', 'add', 'origin', fixtureOrigin]);
  try { await access(path.join(homedir(), '.gwt/runtime')); await symlink(path.join(homedir(), '.gwt/runtime'), path.join(state, 'runtime')); }
  catch (error) { if (error.code !== 'ENOENT') throw error; }
  const env = { ...process.env, HOME: home, USERPROFILE: home, GIT_TERMINAL_PROMPT: '0', GH_PROMPT_DISABLED: '1' };
  for (const key of Object.keys(env)) if (key.startsWith('GWT_')) delete env[key];
  const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN
    || spawnSync('gh', ['auth', 'token'], { encoding: 'utf8', timeout: 10_000 }).stdout?.trim();
  if (token) { env.GH_TOKEN = token; env.GITHUB_TOKEN = token; }
  const checkoutLocal = value => /[/\\]target[/\\]/i.test(value ?? '');
  const locator = spawnSync('which', ['gwtd'], { encoding: 'utf8' });
  let hookBin = process.env.GWT_HOOK_BIN;
  if (!hookBin || checkoutLocal(hookBin)) hookBin = undefined;
  if (hookBin && hookBin !== 'gwtd') { try { await access(hookBin, constants.X_OK); } catch { hookBin = undefined; } }
  if (!hookBin && locator.status === 0 && !checkoutLocal(locator.stdout.trim())) hookBin = 'gwtd';
  if (!hookBin) { try { await access('/Applications/GWT.app/Contents/MacOS/gwtd', constants.X_OK); hookBin = '/Applications/GWT.app/Contents/MacOS/gwtd'; } catch { hookBin = 'gwtd'; } }
  const auditEnv = { ...process.env, GWT_HOOK_BIN: hookBin };
  delete auditEnv.GWT_BIN_PATH;
  const hookParams = { expected_hook_bin: hookBin, runtime_state_path: path.join(state, 'browser-check-missing-runtime-state.json') };
  const allowMissing = hookBin === 'gwtd' && locator.status !== 0;
  function assertHealth(health) {
    if (!health || health.status === 'inactive' || !Array.isArray(health.issues)
      || health.issues.some(issue => !(allowMissing && issue.startsWith('managed hook binary missing: ') && issue.endsWith(' uses gwtd'))
        && !(issue.startsWith('managed hook failure: ') && / state=fail-open( |$)/.test(issue)))) throw new Error(`Managed hook convergence failed: ${JSON.stringify({ status: health?.status, issues: Array.isArray(health?.issues) ? health.issues.map(issue => issue.replace(/(?:First untrusted command:|expected_command=|actual_command=)[\s\S]*/, '[command diagnostics omitted]')) : [] })}`);
  }
  operation(gwtd, 'hook.doctor', { ...hookParams, repair: true }, auditEnv);
  // Repair regenerates managed identities; registration verifies only generated
  // or already reviewed hooks. Use the real provider config, never fresh HOME.
  operation(gwtd, 'hook.register_codex_managed_hook_trust', {
    project_root: ROOT, codex_hook_discovery: 'both',
    codex_config: path.join(process.env.CODEX_HOME || path.join(homedir(), '.codex'), 'config.toml'),
  }, auditEnv);
  // Unlike hook.health, doctor includes persisted Codex trust in its projection.
  assertHealth(operation(gwtd, 'hook.doctor', { ...hookParams, repair: false }, auditEnv).value.health);
  const lookup = operation(gwtd, 'issue.monitor.status', {}, env, project);
  const hash = lookup.envelope.project_store?.hash;
  if (!/^[a-zA-Z0-9_-]+$/.test(hash ?? '')) throw new Error('Missing canonical project_store.hash');
  const projectState = path.join(state, 'projects', hash, 'project-state');
  await mkdir(projectState, { recursive: true });
  const json = (file, value) => writeFile(file, JSON.stringify(value));
  await json(path.join(projectState, 'pm.json'), { settings: { auto_start: false }, registration: {
    session_id: 'pm-pull-render-fixture', agent_id: 'codex', worktree_path: project, created_at: '2026-10-13T00:00:00Z' } });
  await json(path.join(projectState, 'issue-monitor.json'), { enabled: false, max_active_agents: 5, priority_order: [] });
  await json(path.join(state, 'session.json'), { tabs: [{ id: 'pull-render', title: 'Pull render measurement', project_root: project, kind: 'git' }], active_tab_id: 'pull-render', recent_projects: [] });
  await json(path.join(state, 'projects', hash, 'workspace.json'), { viewport: { x: 0, y: 0, zoom: 1 }, windows: [{
    id: 'pm-reports', title: 'PM', preset: 'agent', geometry: { x: 970, y: 450, width: 440, height: 350 },
    z_index: 1, status: 'stopped', session_id: 'pm-pull-render-fixture', agent_id: 'codex', persist: true }], next_z_index: 2 });
  await mkdir(path.join(state, 'sessions'));
  const branch = 'develop';
  await writeFile(path.join(state, 'sessions/pm-pull-render-fixture.toml'), `id = "pm-pull-render-fixture"\nworktree_path = ${JSON.stringify(project)}\nbranch = ${JSON.stringify(branch)}\nagent_id = { type = "Codex" }\nstatus = "Stopped"\ncreated_at = "2026-10-13T00:00:00Z"\nupdated_at = "2026-10-13T00:00:00Z"\nlast_activity_at = "2026-10-13T00:00:00Z"\n`);
  const workloadFile = path.join(home, 'fixed-task.cjs');
  await writeFile(workloadFile, WORKLOAD_SOURCE);
  const barrier = path.join(home, 'start.json');
  await writeFile(path.join(state, 'config.toml'), Array.from({ length: 5 }, (_, index) =>
    `[tools.customCodingAgents.pull-render-${index}]\ndisplayName = "Pull render ${index}"\nagentType = "path"\ncommand = ${JSON.stringify(process.execPath)}\ndefaultArgs = [${JSON.stringify(workloadFile)}, "${index}"]\n[tools.customCodingAgents.pull-render-${index}.env]\nGWT_PULL_AGENT_DIR = ${JSON.stringify(agentDir)}\nGWT_PULL_BARRIER = ${JSON.stringify(barrier)}\n`).join('\n'));
  const out = path.resolve(options.out);
  await mkdir(path.dirname(out), { recursive: true });
  const urlFile = path.join(home, 'url');
  const child = spawn(gwt, ['--no-tray', '--no-open'], { cwd: project,
    env: { ...env, GWT_HOOK_BIN: hookBin, GWT_PROJECT_ROOT: project, GWT_BROWSER_URL_FILE: urlFile }, stdio: ['ignore', 'ignore', 'ignore'] });
  let launchError;
  child.on('error', error => { launchError = error; });
  let testChild;
  let interrupted = false;
  const cleanup = createMeasurementCleanup({ guiChild: child, getTestChild: () => testChild, agentDir, workloadFile });
  const signals = new Map(['SIGINT', 'SIGTERM', 'SIGHUP'].map(signal => [signal, async () => {
    interrupted = true;
    try { await cleanup(); } catch (error) { console.error(error.message); }
    process.exit(1);
  }]));
  for (const [signal, handler] of signals) process.on(signal, handler);
  try {
    let url;
    const deadline = Date.now() + 60_000;
    while (Date.now() < deadline) {
      if (launchError) throw new Error('Fresh GUI could not start');
      if (child.exitCode !== null || child.signalCode !== null) throw new Error('Fresh GUI exited during readiness');
      try { url = (await readFile(urlFile, 'utf8')).trim(); if (url && (await fetch(url, { method: 'HEAD', signal: AbortSignal.timeout(2000) })).ok) break; url = undefined; } catch { url = undefined; }
      await delay(100);
    }
    if (!url) throw new Error('Fresh checkout GUI readiness failed');
    assertHealth(operation(gwtd, 'hook.doctor', { ...hookParams, repair: false }, auditEnv).value.health);
    const sha = spawnSync('git', ['rev-parse', 'HEAD'], { cwd: ROOT, encoding: 'utf8' }).stdout.trim();
    // HEAD alone cannot identify a build made during the RED/GREEN loop.
    // New harness files have their own hash; preserve the tracked source diff
    // identity without copying its contents into measurement artifacts.
    const sourceDiff = spawnSync('git', ['diff', '--binary', 'HEAD', '--', 'crates', 'scripts', 'Cargo.toml', 'Cargo.lock'],
      { cwd: ROOT, encoding: 'utf8' });
    if (sourceDiff.status !== 0) throw new Error('Measurement source identity unavailable');
    const manifest = { ...options, out, root: ROOT, project, home, gui_pid: child.pid, runner_pid: process.pid, source_sha: sha, branch,
      source_diff_sha256: createHash('sha256').update(sourceDiff.stdout).digest('hex'), source_dirty: sourceDiff.stdout.length > 0,
      host: hostname(), platform: process.platform, architecture: process.arch, profile: 'debug',
      binary_sha256: createHash('sha256').update(await readFile(gwt)).digest('hex'),
      workload_hash: createHash('sha256').update(WORKLOAD_SOURCE).digest('hex'), agentDir, barrier };
    manifest.fixture_hash = createHash('sha256').update(fixtureText).update(fixtureCommit).digest('hex');
    manifest.fixture_commit = fixtureCommit;
    manifest.harness_hash = createHash('sha256').update(await readFile(fileURLToPath(import.meta.url)))
      .update(await readFile(path.join(ROOT, 'crates/gwt/playwright/tests/pull-render-performance.spec.ts'))).digest('hex');
    await json(path.join(home, 'manifest.json'), manifest);
    if (interrupted) throw new Error('Measurement interrupted before Playwright launch');
    console.log(JSON.stringify({ phase: 'measurement_ready', manifest: path.join(home, 'manifest.json'), gui_pid: child.pid, harness_hash: manifest.harness_hash }));
    const exitCode = await new Promise((resolve, reject) => {
      testChild = spawn('bash', [path.join(ROOT, 'scripts/run-visual-tests.sh'),
        'crates/gwt/playwright/tests/pull-render-performance.spec.ts', '--grep', 'real AgentBootstrap measurement',
        '--headed', '--workers=1', '--retries=0', `--project=chromium-${options.theme}`, '--output', path.join(home, 'playwright')],
      { cwd: ROOT, detached: true, env: { ...process.env, GWT_PLAYWRIGHT_BASE_URL: url, GWT_PLAYWRIGHT_CHECK_HOME: home,
        GWT_PLAYWRIGHT_CHECKOUT_ROOT: ROOT, GWT_PLAYWRIGHT_PROJECT_ROOT: project, GWT_PULL_RENDER_MANIFEST: path.join(home, 'manifest.json') }, stdio: 'inherit' });
      testChild.once('error', reject); testChild.once('exit', code => resolve(code));
    });
    const report = JSON.parse(await readFile(out, 'utf8'));
    report.ac17_pass = false;
    report.invalid_reasons = [...new Set([...(report.invalid_reasons ?? []), ...validateMeasurementTiming(report)])];
    if (options.mode === 'before') {
      try { report.self_coverage = pairedComparison(report, report).runs[0].coverage; }
      catch (error) { report.invalid_reasons.push(error.message); }
    }
    report.evidence_valid = report.complete === true && report.invalid_reasons.length === 0;
    if (!report.evidence_valid) report.AgentVisualCheck = 'fail';
    try { await cleanup(); report.cleanup_success = true; }
    catch (error) {
      report.cleanup_success = false; report.evidence_valid = false;
      report.invalid_reasons = [...new Set([...(report.invalid_reasons ?? []), 'owned cleanup failed'])];
      await json(out, report); throw error;
    }
    await json(out, report);
    if (exitCode !== 0) throw new Error('Measurement Playwright run failed');
    if (!report.evidence_valid) throw new Error('Measurement evidence invalid; see metadata report');
    if (options.mode === 'after') {
      const baseline = JSON.parse(await readFile(options.baseline, 'utf8'));
      report.comparison_failures = compareMeasurements(baseline, report);
      report.ws_payload_reduction_bytes = baseline.ws_payload_bytes - report.ws_payload_bytes;
      report.ac17_pass = report.comparison_failures.length === 0;
    }
    await json(out, report);
    if (options.mode === 'after' && !report.ac17_pass) throw new Error('AC-17 comparison failed; see metadata report');
    console.log(JSON.stringify({ report: out, mode: options.mode, complete: report.complete, ac17_pass: report.ac17_pass }));
  } finally {
    await cleanup();
    for (const [signal, handler] of signals) process.removeListener(signal, handler);
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { const options = parseOptions(process.argv.slice(2));
    if (options.validate_only) console.log(JSON.stringify(options)); else await run(options);
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
