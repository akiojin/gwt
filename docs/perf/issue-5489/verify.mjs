import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const repo = fileURLToPath(new URL('../../../', import.meta.url));
const historicalRef = 'a96ce624e';
const read = (name) => readFileSync(new URL(name, import.meta.url), 'utf8').replace(/^\uFEFF/, '');
const rows = (name) => read(name).trim().split(/\r?\n/).map(JSON.parse);
const digest = (value) => createHash('sha256').update(value).digest('hex');
const median = (values) => values.toSorted((a, b) => a - b)[Math.floor(values.length / 2)];
const gitSource = (name) => execFileSync('git', ['show', `${historicalRef}:crates/gwt/src/${name}`], { cwd: repo });
const currentSource = (name) => readFileSync(new URL(`../../../crates/gwt/src/${name}`, import.meta.url));

function verify(prefix, sourceHash, monitorHash) {
  const comparison = rows(`${prefix}comparison.jsonl`);
  const breakdown = rows(`${prefix}breakdown.jsonl`);
  const evidenceContexts = [];
  for (const file of ['comparison.jsonl', 'breakdown.jsonl']) {
    const context = JSON.parse(read(`${prefix}${file}.context.json`));
    assert.equal(context.current_sha256, sourceHash, `${prefix}${file}: measured source must match`);
    assert.equal(context.roots.length, 5);
    assert.equal(context.instrumented, file === 'breakdown.jsonl');
    evidenceContexts.push(context);
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
        const beforeLoads = before.breakdown.runtime_sidecar_load?.calls ?? 0;
        const afterLoads = after.breakdown.runtime_sidecar_load?.calls ?? 0;
        assert.ok(beforeLoads > 0 && afterLoads <= beforeLoads / 5, 'shared census/cache reduces sidecar decoding');
      }
    }
  }
  const before = median(comparison.filter((row) => row.mode === 'before').map((row) => row.tick_cpu_seconds));
  const after = median(comparison.filter((row) => row.mode === 'after').map((row) => row.tick_cpu_seconds));
  assert.ok(before > 0 && after > 0 && after <= before / 2, 'census CPU is at most half');
  console.log(`PASS ${prefix || historicalRef}: five roots, three pairs, scoped observations, one global snapshot; census ${before} -> ${after} CPU seconds/tick (${((1 - after / before) * 100).toFixed(2)}% lower)`);

  const worker = rows(`${prefix}worker-comparison.jsonl`);
  const workerContext = JSON.parse(read(`${prefix}worker-comparison.jsonl.context.json`));
  assert.equal(workerContext.current_sha256, sourceHash);
  assert.equal(workerContext.monitor_current_sha256, monitorHash);
  evidenceContexts.push(workerContext);
  const manifestName = workerContext.prefs_manifest ?? `${prefix}worker-prefs-manifest.json`;
  assert.equal(manifestName, `${prefix}worker-prefs-manifest.json`);
  const manifest = JSON.parse(read(manifestName));
  assert.equal(manifest.length, 5);
  const contexts = worker.filter((row) => row.kind === 'context');
  assert.equal(contexts.length, 2);
  for (const context of contexts) {
    assert.equal(context.pty_registrations, 0);
    assert.equal(context.stop_or_park_groups, 0);
    assert.equal(context.roots.length, 5);
    assert.ok(context.initial_prefs.every((prefs) => prefs.path.startsWith(context.isolated_home)));
    assert.deepEqual(context.initial_prefs.map((prefs) => prefs.sha256), manifest.map((prefs) => prefs.sha256));
  }
  const workerMedians = ['before', 'after'].map((mode) => {
    const ticks = worker.filter((row) => row.kind === 'tick' && row.mode === mode);
    assert.equal(ticks.length, 3);
    assert.ok(ticks.every((row) => row.thread_name === 'gwt-monitor-runtime' && row.thread_cpu_seconds > 0));
    return median(ticks.map((row) => row.thread_cpu_seconds));
  });
  assert.ok(workerMedians[1] < workerMedians[0], 'worker improves');
  const workerCores = ['before', 'after'].map((mode) => {
    const ticks = worker.filter((row) => row.kind === 'tick' && row.mode === mode);
    return ticks.reduce((sum, row) => sum + row.thread_cpu_seconds, 0)
      / ticks.reduce((sum, row) => sum + row.elapsed_seconds, 0);
  });
  console.log(`${prefix || historicalRef} worker: ${workerMedians.join(' -> ')} CPU seconds/tick; AC-3 target met: ${workerMedians[1] <= workerMedians[0] / 2}`);

  if (prefix) {
    assert.ok(workerMedians[1] <= workerMedians[0] / 2, 'integrated worker CPU/tick is at most half');
    assert.ok(workerCores[1] <= workerCores[0] / 2, 'integrated worker average cores are at most half');
    console.log(`PASS integrated worker average cores: ${workerCores.join(' -> ')}`);
    const libraryHashes = evidenceContexts.map((context) => context.gwt_library_sha256);
    assert.ok(libraryHashes.every((hash) => /^[a-f0-9]{64}$/.test(hash)), 'library digest provenance');
    assert.equal(new Set(libraryHashes).size, 1, 'all integrated probes use the same library');
    const sharedHash = digest(currentSource('issue_monitor.rs'));
    for (const context of evidenceContexts) {
      assert.equal(context.shared_issue_monitor_sha256, sharedHash);
      assert.ok(/^[a-f0-9]{40}$/.test(context.source_commit));
      assert.ok(context.dependency_base_commit.startsWith('4cb8fe4a0'));
      assert.deepEqual(context.dependencies, ['Issue #5498 / PR #5511', 'Issue #5499 / PR #5508']);
      execFileSync('git', ['merge-base', '--is-ancestor', context.dependency_base_commit, context.source_commit], { cwd: repo });
    }
  }
}

verify('', digest(gitSource('session_inventory.rs')), digest(gitSource('monitor_duplicate_runtime.rs')));
if (!process.argv.includes('--historical-only')) {
  verify('integrated-', digest(currentSource('session_inventory.rs')), digest(currentSource('monitor_duplicate_runtime.rs')));
}
