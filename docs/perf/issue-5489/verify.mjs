import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';

const read = (name) => readFileSync(new URL(name, import.meta.url), 'utf8').replace(/^\uFEFF/, '');
const rows = (name) => read(name).trim().split(/\r?\n/).map(JSON.parse);
const median = (values) => values.toSorted((a, b) => a - b)[Math.floor(values.length / 2)];
const source = readFileSync(new URL('../../../crates/gwt/src/session_inventory.rs', import.meta.url));
const hash = createHash('sha256').update(source).digest('hex');
const comparison = rows('comparison.jsonl');
const breakdown = rows('breakdown.jsonl');

for (const file of ['comparison.jsonl', 'breakdown.jsonl']) {
  const context = JSON.parse(read(`${file}.context.json`));
  assert.equal(context.current_sha256, hash, `${file}: measured source must match`);
  assert.equal(context.roots.length, 5);
  assert.equal(context.instrumented, file === 'breakdown.jsonl');
}

for (const data of [comparison, breakdown]) {
  assert.equal(data.length, 6, 'three alternating pairs');
  for (let pair = 1; pair <= 3; pair++) {
    const before = data.find((row) => row.pair === pair && row.mode === 'before');
    const after = data.find((row) => row.pair === pair && row.mode === 'after');
    assert.ok(before && after);
    const observations = (row) => row.roots.map(({ root, sessions, uncertainties }) => ({ root, sessions, uncertainties }));
    assert.equal(before.roots.length, 5);
    assert.deepEqual(observations(after), observations(before), 'project observations are preserved');
    if (data === breakdown) {
      assert.equal(after.breakdown.os_process_snapshot_all.calls, 1);
      assert.equal(before.breakdown.runtime_sidecar_load.calls, after.breakdown.runtime_sidecar_load.calls * 5);
    }
  }
}

const before = median(comparison.filter((row) => row.mode === 'before').map((row) => row.tick_cpu_seconds));
const after = median(comparison.filter((row) => row.mode === 'after').map((row) => row.tick_cpu_seconds));
assert.ok(before > 0 && after > 0 && after <= before / 2, 'census CPU is at most half');
console.log(`PASS: five roots, three pairs, scoped observations, one global snapshot; census ${before} -> ${after} CPU seconds/tick (${((1 - after / before) * 100).toFixed(2)}% lower)`);

const worker = rows('worker-comparison.jsonl');
const workerContext = JSON.parse(read('worker-comparison.jsonl.context.json'));
const monitorSource = readFileSync(new URL('../../../crates/gwt/src/monitor_duplicate_runtime.rs', import.meta.url));
assert.equal(workerContext.current_sha256, hash);
assert.equal(workerContext.monitor_current_sha256, createHash('sha256').update(monitorSource).digest('hex'));
const contexts = worker.filter((row) => row.kind === 'context');
assert.equal(contexts.length, 2);
for (const context of contexts) {
  assert.equal(context.pty_registrations, 0);
  assert.equal(context.stop_or_park_groups, 0);
  assert.equal(context.roots.length, 5);
  assert.ok(context.initial_prefs.every((prefs) => prefs.path.startsWith(context.isolated_home)));
}
assert.deepEqual(contexts[0].initial_prefs.map((prefs) => prefs.sha256), contexts[1].initial_prefs.map((prefs) => prefs.sha256));
const workerMedians = ['before', 'after'].map((mode) => {
  const ticks = worker.filter((row) => row.kind === 'tick' && row.mode === mode);
  assert.equal(ticks.length, 3);
  assert.ok(ticks.every((row) => row.thread_name === 'gwt-monitor-runtime' && row.thread_cpu_seconds > 0));
  return median(ticks.map((row) => row.thread_cpu_seconds));
});
assert.ok(workerMedians[1] < workerMedians[0], 'worker improves');
// Preserve the measured acceptance miss; a passing evidence check does not
// certify the owner's worker-wide 50% target or installed-GUI remeasurement.
console.log(`Worker measurement: ${workerMedians.join(' -> ')} CPU seconds/tick; AC-3 target met: ${workerMedians[1] <= workerMedians[0] / 2} (owner remains open when false)`);
