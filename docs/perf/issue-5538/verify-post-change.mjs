// Recompute AC-3 from sanitized real-provider evidence, without launching agents.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const directory = new URL('./', import.meta.url);
const evidence = JSON.parse(readFileSync(new URL('fleet-post-change.json', directory), 'utf8'));
const { dark, light, shutdown, owned_processes: receipts } = evidence;
assert.match(evidence.source_commit, /^[a-f0-9]{40}$/);
assert.equal(dark.passed, true);
assert.equal(light.passed, true);
assert.deepEqual(dark.browser_errors, []);
assert.deepEqual(light.browser_errors, []);
assert.equal(shutdown.test_exit_code, 0);
assert.equal(shutdown.owned_gui_stopped, true);
assert.deepEqual(shutdown.cleanup_errors, []);
assert.equal(evidence.private_auth_removed, true);
assert.equal(dark.user_verification_result, 'n/a (autonomous)');
assert.equal(dark.agent_visual_check, 'pass');
assert.equal(light.agent_visual_check, 'pass');
assert(dark.elapsed_ms >= 1_800_000);
const start = Date.parse(dark.observation_started_at);
const end = Date.parse(dark.observation_ended_at);
assert(end - start >= 1_800_000);

// The source commit precedes the evidence commit. Future source changes must
// not relabel this historical measurement as a measurement of newer code.
const changedSinceMeasurement = [];
for (const source of dark.fixture.source_snapshot) {
  let actual = execFileSync('git', ['hash-object', '--', source.path], {
    cwd: fileURLToPath(new URL('../../../', directory)), encoding: 'utf8',
  }).trim();
  if (actual !== source.blob_sha) {
    changedSinceMeasurement.push(source.path);
    try {
      actual = execFileSync('git', ['rev-parse', `${evidence.source_commit}:${source.path}`], {
        cwd: fileURLToPath(new URL('../../../', directory)), encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
      }).trim();
    } catch {
      throw new Error(`Measured source unavailable; git fetch origin ${evidence.source_commit}, then retry`);
    }
  }
  assert.equal(actual, source.blob_sha, source.path);
}
assert.match(dark.fixture.binary_sha256, /^[A-Fa-f0-9]{64}$/);
const samples = readFileSync(new URL('fleet-samples.jsonl', directory), 'utf8')
  .trim().split('\n').map(line => JSON.parse(line));
assert(samples.length > 2);
assert(samples[0].elapsed_ms < 60_000);
assert(samples.at(-1).elapsed_ms >= 1_800_000);
const first = samples[0].agents;
assert.equal(first.length, 8);
assert.equal(new Set(first.map(agent => agent.session_id)).size, 8);
assert.equal(new Set(first.map(agent => agent.child.pid)).size, 8);
assert.equal(evidence.tool_result_sources.length, 8);
for (const agent of first) {
  const source = evidence.tool_result_sources.find(source => source.session_id === agent.session_id);
  assert(source);
  assert.equal(source.heartbeat_shell_pid, agent.heartbeat_shell_pid);
  assert.match(source.sha256, /^[a-f0-9]{64}$/);
}
const lastTick = new Map();
const lastElapsed = new Map();
for (let index = 0; index < samples.length; index++) {
  const sample = samples[index];
  assert.equal(sample.agent_count, 8);
  assert.equal(sample.agents.length, 8);
  if (index) assert(sample.elapsed_ms - samples[index - 1].elapsed_ms < 60_000);
  for (const [position, agent] of sample.agents.entries()) {
    assert.equal(agent.session_id, first[position].session_id);
    assert.equal(agent.window_id, first[position].window_id);
    assert.deepEqual(agent.child, first[position].child);
    assert.equal(agent.child.name.toLowerCase(), 'codex.exe');
    assert(agent.heartbeat_tick > 0);
    assert(agent.heartbeat_elapsed_ms >= (lastElapsed.get(agent.session_id) || 0));
    lastElapsed.set(agent.session_id, agent.heartbeat_elapsed_ms);
    const heartbeatAt = Date.parse(agent.heartbeat_observed_at);
    const observedAt = Date.parse(sample.observed_at);
    assert(heartbeatAt <= observedAt && observedAt - heartbeatAt < 60_000);
    const previous = lastTick.get(agent.session_id);
    if (!previous || agent.heartbeat_tick > previous.tick) {
      lastTick.set(agent.session_id, { tick: agent.heartbeat_tick, at: sample.elapsed_ms });
    } else {
      assert.equal(agent.heartbeat_tick, previous.tick);
      assert(sample.elapsed_ms - previous.at < 60_000);
    }
    // The retained census contains only exact descendants of the owned GUI.
    // Both the real provider and heartbeat must lead back to that same PID.
    for (const leaf of [agent.child.pid, agent.heartbeat_shell_pid]) {
      const seen = new Set();
      let current = leaf;
      while (current !== dark.fixture.gui_pid) {
        assert(!seen.has(current), 'acyclic process lineage'); seen.add(current);
        const process = agent.process_lineage.find(receipt => receipt.pid === current);
        assert(process, `owned process receipt ${current}`);
        assert(receipts.some(receipt => receipt.pid === process.pid
          && receipt.started_at === process.started_at && receipt.parent_pid === process.parent_pid));
        current = process.parent_pid;
      }
    }
  }
}

const windows = new Map();
const groups = new Map();
for (const row of dark.dispatch_summary_rows) {
  const summary = row.dispatch;
  const ended = Date.parse(summary.window_ended_at);
  assert.equal(row.type, 'gui_dispatch_summary');
  assert.equal(summary.pid, dark.fixture.gui_pid);
  assert.equal(summary.dropped_windows, 0);
  assert(summary.window_ms >= 60_000);
  assert(ended <= end && ended - summary.window_ms >= start);
  windows.set(summary.window_ended_at, summary.window_ms);
  if (summary.count === 0) { assert.equal(row.target, 'gui:event_loop'); continue; }
  assert(summary.total_ms > summary.count * 50);
  assert(summary.p95_ms > 50 && summary.p95_ms <= summary.max_ms);
  assert(summary.pending_max === null || Number.isInteger(summary.pending_max));
  assert(summary.pending_unknown_count >= 0 && summary.pending_unknown_count <= summary.count);
  const group = groups.get(row.target) || { event: row.target, count: 0, total_ms: 0, max_ms: 0 };
  group.count += summary.count; group.total_ms += summary.total_ms;
  group.max_ms = Math.max(group.max_ms, summary.max_ms); groups.set(row.target, group);
}
assert(windows.size >= 30);
assert.equal(windows.size, dark.complete_window_count);
assert.equal([...windows.values()].reduce((sum, value) => sum + value, 0), dark.complete_window_ms);
const ranked = [...groups.values()].sort((a, b) => b.total_ms - a.total_ms).slice(0, 10);
assert.deepEqual(ranked, dark.top10);
console.log(JSON.stringify({ issue: 5538, real_agents: 8, samples: samples.length,
  elapsed_ms: dark.elapsed_ms, complete_windows: windows.size, top10: ranked,
  source_commit: evidence.source_commit, changed_since_measurement: changedSinceMeasurement }, null, 2));
console.log('PASS: pinned-source real eight-agent observation and both headed themes');
