// Validate the primary observation, including an honestly reported budget miss.
// Success means the evidence is complete and agrees with result.json.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const read = name => readFileSync(new URL(name, import.meta.url), 'utf8');
const context = JSON.parse(read('context.json'));
const samples = read('samples.jsonl').trim().split('\n').map(line => JSON.parse(line));
assert.equal(samples.length, 61, '61 real samples are required; incomplete observations cannot pass');
assert.equal(context.issue, 5385);
assert.equal(context.duration_seconds, 3600);
assert.equal(context.interval_seconds, 60);
assert.equal(context.rss_budget_bytes, 2 ** 31);
assert.equal(context.cpu_budget_cores, 1);
assert.equal(context.pane_listener_pid, context.gui_pid, 'pane source must be the measured GUI');
assert.match(context.gui_sha256, /^[a-f0-9]{64}$/);
assert.equal(context.gui_version, '9.121.1');
assert.equal(context.agent_processes.filter(process => process.Name === 'codex.exe').length, 8);
assert.deepEqual(context.dependency_commits.map(row => row.issue), [5381, 5383, 5384]);
const git = args => execFileSync('git', args, { encoding: 'utf8' }).trim();
assert.equal(git(['rev-parse', `${context.release_tag}^{commit}`]), context.release_commit);
for (const dependency of context.dependency_commits) {
  git(['merge-base', '--is-ancestor', dependency.commit, context.release_commit]);
  git(['merge-base', '--is-ancestor', dependency.commit, context.baseline_commit]);
}

const first = samples[0];
const last = samples.at(-1);
for (const [index, sample] of samples.entries()) {
  assert.equal(sample.minute, index);
  assert.equal(sample.gui_pid, context.gui_pid);
  assert.equal(sample.gui_started_at, context.gui_started_at);
  assert.ok(Number.isFinite(sample.elapsed_seconds) && sample.elapsed_seconds >= 0);
  assert.ok(Number.isFinite(sample.cpu_seconds) && sample.cpu_seconds >= 0);
  assert.ok(Number.isSafeInteger(sample.working_set_bytes) && sample.working_set_bytes > 0);
  const panes = sample.panes.filter(line => /\trunning\t/.test(line) && /\trole=implementation_agent(?:\t|$)/.test(line));
  assert.equal(panes.length, sample.implementation_panes);
  assert.equal(new Set(panes.map(line => line.split('\t')[0])).size, panes.length);
  if (index > 0) {
    const previous = samples[index - 1];
    assert.ok(sample.cpu_seconds >= previous.cpu_seconds, 'CPU counter must not reset');
    // OS wakeup jitter is measured, not treated as an exact wall-clock assertion.
    const interval = sample.elapsed_seconds - previous.elapsed_seconds;
    assert.ok(interval >= 55 && interval <= 65, `one-minute cadence at minute ${index}: ${interval}`);
    const wallInterval = (Date.parse(sample.observed_at) - Date.parse(previous.observed_at)) / 1000;
    assert.ok(Math.abs(interval - wallInterval) < 1, 'monotonic and UTC intervals must agree');
  }
}
const duration = last.elapsed_seconds - first.elapsed_seconds;
assert.ok(Math.abs(duration - 3600) < 1, `one hour of actual elapsed time: ${duration}`);
const peakRss = Math.max(...samples.map(row => row.working_set_bytes));
const averageCores = (last.cpu_seconds - first.cpu_seconds) / duration;
const actual = {
  sample_count: samples.length,
  duration_seconds: duration,
  peak_rss_bytes: peakRss,
  average_cpu_cores: averageCores,
  min_implementation_panes: Math.min(...samples.map(row => row.implementation_panes)),
  max_implementation_panes: Math.max(...samples.map(row => row.implementation_panes)),
  fleet_within_requested_range: samples.every(row => row.implementation_panes >= 7 && row.implementation_panes <= 8),
  rss_within_budget: peakRss <= context.rss_budget_bytes,
  cpu_within_budget: averageCores < context.cpu_budget_cores,
};
const result = JSON.parse(read('result.json'));
for (const [key, value] of Object.entries(actual)) assert.equal(result[key], value, key);
if (!actual.rss_within_budget || !actual.cpu_within_budget) {
  assert.ok(Number.isSafeInteger(result.follow_up_issue) && result.follow_up_issue > 0,
    'a budget miss requires the registered follow-up Issue from AC-3');
  assert.match(read('report.md'), new RegExp(`Issue #${result.follow_up_issue}`));
}
if (!actual.fleet_within_requested_range || !actual.rss_within_budget || !actual.cpu_within_budget) {
  assert.equal(result.issue_remains_open, true, 'incomplete acceptance must not be reported as delivered');
}
if (!actual.fleet_within_requested_range) assert.match(read('report.md'), /AC-1.*未証明/);
if (!actual.cpu_within_budget) assert.match(read('report.md'), /AC-2.*未達/);
console.log(JSON.stringify({ evidence_valid: true, ...actual, follow_up_issue: result.follow_up_issue ?? null }, null, 2));
