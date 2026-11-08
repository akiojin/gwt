// Validate the retained investigation packet, not a provider performance SLA.
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { createHash } = require('node:crypto');
const { gunzipSync } = require('node:zlib');
const directory = join(__dirname, '../../docs/perf/issue-6422');
const summary = JSON.parse(readFileSync(join(directory, 'summary.json')));
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
assert.equal(summary.issue, 6422);
for (const artifact of summary.artifacts) {
  const bytes = readFileSync(join(directory, artifact.file));
  assert.equal(sha(bytes), artifact.sha256, artifact.file);
  const raw = gunzipSync(bytes);
  assert.equal(sha(raw), artifact.uncompressed_sha256, artifact.file);
  assert.equal(raw.length, artifact.uncompressed_bytes, artifact.file);
}
const paired = summary.runs.filter(run => run.name.startsWith('paired-'));
assert.deepEqual(paired.map(run => run.theme).sort(), ['dark', 'light']);
assert.equal(new Set(summary.runs.map(run => run.binary_sha256)).size, 1);
assert.equal(new Set(summary.runs.map(run => run.daemon_sha256)).size, 1);
for (const run of paired) {
  assert.equal(run.measurement_test_passed, true);
  assert.deepEqual(run.browser_errors, []);
  assert.deepEqual(run.shutdown.cleanup_errors, []);
  assert.equal(run.shutdown.test_exit_code, 0);
  assert.equal(run.observer.priority, 'Normal');
  assert.equal(run.registry_error_samples, 0);
  assert.equal(run.measurements.find(row => row.load === 'low').outcome, 'first-hook-complete');
  const high = run.measurements.find(row => row.load === 'high');
  assert.ok(high.claim_id && high.delivery_id && high.session_id && high.window_id);
  assert.ok(Date.parse(high.pty_release_bracket.lower_bound) <= Date.parse(high.pty_release_bracket.upper_bound));
  assert.ok(Date.parse(high.pty_release_bracket.upper_bound) <= Date.parse(high.ack_observed_at));
  assert.ok(high.post_ack_observation_ms >= 120000);
  assert.ok(high.os_samples > 1);
  assert.equal(high.load_before.workers, 30);
  assert.equal(high.load_at_end.workers, 30);
  assert.equal(high.load_before.pid, high.load_at_end.pid);
  assert.ok(high.load_at_end.work_units > high.load_before.work_units);
  const build = high.processes.find(process => process.pid === high.load_before.pid);
  assert.deepEqual(build.priority, ['Normal']);
  assert.ok(build.cpu_delta_ms > 0);
  assert.ok(high.processes.some(process => process.priority.includes('BelowNormal')
    && process.jobs.some(job => job.member && job.cpu_query_error === 0
      && job.cpu_flags === 5 && job.cpu_rate === 800 && (job.limit_flags & 8192) !== 0)));
  console.log(`${run.name}: exact ACK, ${high.os_samples} OS samples, ${high.post_ack_observation_ms}ms post-ACK; outcome=${high.outcome}; max sampler gap=${run.os_gap_ms.max.toFixed(1)}ms`);
}
const verification = JSON.parse(readFileSync(join(directory, 'canonical-paired.json')));
assert.equal(verification.all_passed, true);
assert.equal(verification.plan_covered, true);
assert.ok(verification.commands.every(command => command.exit_code === 0));
console.log(`Issue #6422 packet: PASS (${summary.artifacts.length} hash-checked archives; ${verification.record_id}). This is investigation evidence, not a performance-improvement claim.`);
