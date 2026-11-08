import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';

const here = new URL('.', import.meta.url);
const read = path => readFileSync(new URL(path, here));
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const evidence = JSON.parse(read('fleet-baseline.json'));
assert.equal(evidence.evidence_kind, 'historical_real_fleet_baseline');
assert.equal(evidence.post_change_measurement, false);
assert.equal(evidence.ac3_literal_proven, false);
assert.equal(evidence.window.duration_seconds, 1800);
const start = Date.parse(evidence.window.started_at);
const end = Date.parse(evidence.window.ended_at_exclusive);
assert.equal(end - start, 1_800_000);

const sampleBytes = read(evidence.source.samples);
const contextBytes = read(evidence.source.context);
assert.equal(sha256(sampleBytes), evidence.source.samples_sha256);
assert.equal(sha256(contextBytes), evidence.source.context_sha256);
const sourceSamples = sampleBytes.toString('utf8').trim().split(/\r?\n/).map(JSON.parse);
const context = JSON.parse(contextBytes);
const parsePane = line => {
  const [pane_id, status, provider, , ...attributes] = line.split('\t');
  const fields = Object.fromEntries(attributes.filter(value => value.includes('='))
    .map(value => [value.slice(0, value.indexOf('=')), value.slice(value.indexOf('=') + 1)]));
  return { pane_id, status, provider, agent_name: fields.agent_name ?? null,
    source_session: fields.source_session === 'none' ? null : fields.source_session ?? null,
    created_at: fields.session_created_at ?? null, role: fields.role ?? null };
};
const implementation = panes => panes.filter(pane => pane.status === 'running'
  && pane.role === 'implementation_agent');
assert.deepEqual(evidence.fleet_roster, implementation(sourceSamples[0].panes.map(parsePane)));
assert.equal(evidence.fleet_roster.length, 8);
assert(evidence.fleet_roster.every(pane => pane.provider === 'codex'));
assert(evidence.fleet_roster.every(pane => pane.source_session === null));
const ids = evidence.fleet_roster.map(pane => pane.pane_id).sort();
assert.equal(evidence.fleet_samples.length, 32);
for (const [index, sample] of evidence.fleet_samples.entries()) {
  const original = sourceSamples[index];
  for (const key of ['minute', 'observed_at', 'elapsed_seconds', 'gui_pid', 'gui_started_at']) {
    assert.deepEqual(sample[key], original[key]);
  }
  assert.equal(sample.gui_pid, evidence.gui.gui_pid);
  assert.equal(sample.gui_started_at, evidence.gui.gui_started_at);
  const panes = original.panes.map(parsePane);
  assert.deepEqual(sample.running_implementation_pane_ids,
    implementation(panes).map(pane => pane.pane_id));
  assert.deepEqual([...sample.running_implementation_pane_ids].sort(), ids);
  assert.equal(original.implementation_panes, 8);
  assert.equal(sample.running_pm_count,
    panes.filter(pane => pane.status === 'running' && pane.role === 'pm').length);
  if (index) {
    const gap = sample.elapsed_seconds - evidence.fleet_samples[index - 1].elapsed_seconds;
    assert(gap >= 55 && gap <= 65, `sample interval ${index}: ${gap}`);
  }
}
assert(Date.parse(evidence.fleet_samples[0].observed_at) <= start);
assert(Date.parse(evidence.fleet_samples.at(-1).observed_at) >= end);
const census = evidence.provider_process_census;
assert.deepEqual(census.codex_processes,
  context.agent_processes.filter(process => process.Name === 'codex.exe'));
assert.equal(census.codex_processes.length, 8);
assert.equal(census.pane_to_process_mapping_recorded, false);

const groups = new Map();
for (const record of evidence.slow_dispatch_records) {
  assert.deepEqual(Object.keys(record).sort(), ['elapsed_ms', 'event', 'timestamp']);
  const timestamp = Date.parse(record.timestamp);
  assert(timestamp >= start && timestamp < end);
  assert(Number.isInteger(record.elapsed_ms) && record.elapsed_ms > 50);
  assert.equal(typeof record.event, 'string');
  if (!groups.has(record.event)) groups.set(record.event, []);
  groups.get(record.event).push(record.elapsed_ms);
}
const top = [...groups].map(([event, durations]) => {
  durations.sort((a, b) => a - b);
  return { event, count: durations.length, total_ms: durations.reduce((a, b) => a + b, 0),
    p95_slow_ms: durations[Math.ceil(durations.length * 0.95) - 1], max_ms: durations.at(-1) };
}).sort((a, b) => b.total_ms - a.total_ms || a.event.localeCompare(b.event)).slice(0, 10);
assert.deepEqual(evidence.summary, {
  slow_dispatch_count: evidence.slow_dispatch_records.length,
  observed_kind_count: groups.size,
  total_ms: evidence.slow_dispatch_records.reduce((sum, record) => sum + record.elapsed_ms, 0),
  top_limit: 10,
  top,
});
for (const detail of evidence.frontend_latency_details) {
  assert(Date.parse(detail.timestamp) >= start && Date.parse(detail.timestamp) < end);
  for (const key of ['handler_ms', 'queue_wait_ms', 'receive_to_complete_ms', 'window_count']) {
    assert(Number.isInteger(detail[key]) && detail[key] >= 0);
  }
}
console.log(`PASS historical baseline: ${evidence.slow_dispatch_records.length} slow dispatches, `
  + `${groups.size} observed kinds, ${evidence.summary.total_ms}ms; 32 fleet samples bracket 1800s.`);
console.log('This historical baseline alone does not prove AC-3: project-scoped fleet and legacy uncorrelated timer.');
