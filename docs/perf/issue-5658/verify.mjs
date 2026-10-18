import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { basename, dirname, isAbsolute, relative, resolve } from "node:path";

const directory = new URL("./", import.meta.url);
const json = async name => JSON.parse(await readFile(new URL(name, directory), "utf8"));
const sha256 = bytes => createHash("sha256").update(bytes).digest("hex");
async function assertSourceHash(path, expected) {
  const bytes = await readFile(path);
  // Preserve measured raw hashes; permit only Git checkout newline conversion.
  // Executable and generated-provider identities are compared exactly below.
  const lf = bytes.toString("utf8").replace(/\r\n/g, "\n");
  assert.ok([sha256(bytes), sha256(lf), sha256(lf.replace(/\n/g, "\r\n"))].includes(expected), path);
}
const launch = await json("after-launch.json");
const cleanup = await json("after-cleanup.json");
const baselineLaunch = await json("before-launch.json");
const extraSources = await json("after-extra-sources.json");
assert.equal(extraSources.binary_sha256, launch.sha256["target/release/gwt.exe"]);
assert.ok(Date.parse(extraSources.recorded_at) <= Date.parse(launch.gui_started_at));
assert.deepEqual(Object.keys(extraSources.sha256), [
  "crates/gwt-agent/src/session_ledger.rs", "crates/gwt-core/src/index/path_policy.rs",
]);
for (const [path, hash] of Object.entries(extraSources.sha256)) {
  await assertSourceHash(path, hash);
}
for (const record of [baselineLaunch, launch]) {
  assert.equal(record.measurement_scope, "steady idle");
  assert.equal(record.build_profile, "release");
  assert.match(record.sha256["target/release/gwt.exe"], /^[a-f0-9]{64}$/);
  const home = dirname(resolve(record.project));
  const fixturePath = relative(resolve("target"), home);
  assert.ok(!isAbsolute(fixturePath) && !/^\.\.(?:[\\/]|$)/.test(fixturePath));
  assert.match(basename(home), /^issue-5658-home-[a-f0-9]{32}$/);
  assert.equal(resolve(record.project), resolve(home, "project"));
  assert.equal(resolve(record.fixture_origin), resolve(home, "origin.git"));
  assert.equal(resolve(record.fixture_push_url), resolve(record.fixture_origin));
  assert.equal(record.fixture_origin_branch_refs, 1);
  assert.equal(record.fixture_project_origin_branch_refs, 1);
  assert.equal(record.fixture_clone_sha, record.git_head);
}
assert.equal(launch.fixture_clone_sha, baselineLaunch.fixture_clone_sha);
assert.equal(launch.provider_sha256, baselineLaunch.provider_sha256);
for (const path of ["scripts/diagnostics/issue-5658-browser-check.ps1", "scripts/diagnostics/issue-5658-cpu.ps1"]) {
  assert.ok(launch.sha256[path], `missing comparison harness: ${path}`);
  assert.equal(launch.sha256[path], baselineLaunch.sha256[path], `comparison harness differs: ${path}`);
}
assert.equal(launch.agent_count, 12);
assert.equal(launch.ready_requests_per_minute, 66);
assert.equal(launch.sessions_fixture.files, 14841);
assert.equal(launch.sessions_fixture.bytes, 36009778);
assert.deepEqual(launch.sessions_fixture, baselineLaunch.sessions_fixture);
assert.equal(launch.pm_auto_start, false);
assert.equal(launch.issue_monitor_enabled, false);
assert.equal(launch.background_index, "normal");
assert.equal(cleanup.gui_pid, launch.gui_pid);
assert.equal(cleanup.gui_exited, true);
assert.equal(cleanup.daemon_exited, true);
for (const path of ["crates/gwt-terminal/src/pane.rs", "crates/gwt/src/session_inventory.rs",
  "crates/gwt/src/index_worker.rs", "crates/gwt/src/runtime_health_poller.rs", "crates/gwt/src/agent_capacity.rs"]) {
  assert.ok(launch.sha256[path], `missing measured source: ${path}`);
}
for (const [path, hash] of Object.entries(launch.sha256)) {
  if (path.startsWith("target/")) continue;
  await assertSourceHash(path, hash);
}
for (const theme of ["dark", "light"]) {
  const result = await json(`after-${theme}.json`);
  assert.equal(result.gui_pid, launch.gui_pid);
  assert.equal(result.label, "after");
  assert.equal(result.theme, theme);
  assert.equal(result.measurement_scope, "steady idle");
  assert.equal(result.build_profile, "release");
  assert.equal(result.workload.panes, 12);
  assert.equal(result.idle_readiness.requested, true);
  assert.equal(result.idle_readiness.ready, true);
  assert.equal(result.idle_readiness.format, 2);
  assert.equal(result.idle_readiness.source_list_present, true);
  assert.equal(result.idle_readiness.expected_worktree_groups, 12);
  assert.equal(result.idle_readiness.matched_worktree_groups, 12);
  assert.deepEqual(result.idle_readiness.missing_worktree_roots, []);
  assert.equal(result.workload.history_lines_per_pane, 600);
  assert.equal(result.workload.kind, "regularly launched custom command Agent PTYs");
  assert.equal(result.workload.fixture_clone_sha, launch.fixture_clone_sha);
  assert.equal(result.workload.provider_sha256, launch.provider_sha256);
  assert.equal(result.workload.ready_requests_per_minute, 66);
  assert.equal(result.workload.agents.length, 12);
  assert.equal(new Set(result.workload.agents.map(row => row.session_id)).size, 12);
  let successes = 0;
  let attempts = 0;
  for (const agent of result.workload.agents) {
    assert.deepEqual(agent.failures, []);
    assert.ok(agent.success_count >= Math.floor(result.elapsed_seconds * 1000
      / result.workload.ready_period_per_agent_ms) - 1);
    assert.equal(agent.history_lines, 600);
    assert.equal(agent.latencies_ms.length, agent.success_count);
    assert.ok(agent.latencies_ms.every(value => Number.isFinite(value) && value >= 0));
    successes += agent.success_count;
    attempts += agent.attempt_count;
  }
  assert.equal(result.workload.ipc_success_count, successes);
  assert.equal(result.workload.ipc_attempt_count, attempts);
  assert.ok(successes >= Math.floor(66 * result.elapsed_seconds / 60) - 12);
  if (theme === "light") {
    assert.equal(result.workload.reused_across_themes, true);
    const dark = await json("after-dark.json");
    assert.deepEqual(result.workload.agents.map(row => row.session_id).sort(),
      dark.workload.agents.map(row => row.session_id).sort());
  }
  assert.ok(result.elapsed_seconds >= (theme === "dark" ? 1800 : 30));
  assert.ok(Number.isFinite(result.average_cores) && result.average_cores >= 0);
  assert.ok(result.average_cores <= 0.2, `${theme}: ${result.average_cores} cores`);
  assert.equal(result.budget.asserted, true);
  assert.equal(result.raw_deltas.length, 12);
  assert.ok(result.raw_deltas.every(row => row.raw_bytes === 0 && row.raw_chunks === 0));
  assert.equal(result.input_latency.length, 24);
  assert.ok(result.input_latency.every(row => Number.isFinite(row.raw_latency_ms)
    && Number.isFinite(row.visible_latency_ms) && row.visible_latency_ms < 15000));
  const p95 = rows => rows.map(row => row.visible_latency_ms).sort((a, b) => a - b)[Math.floor(rows.length * 0.95)];
  const baselineTheme = await json(`before-${theme}.json`);
  assert.equal(baselineTheme.label, "before");
  assert.equal(baselineTheme.theme, theme);
  assert.equal(baselineTheme.build_profile, "release");
  assert.equal(baselineTheme.measurement_scope, "steady idle");
  assert.equal(baselineTheme.gui_pid, baselineLaunch.gui_pid);
  assert.equal(baselineTheme.native_snapshots.length, 2);
  assert.ok(baselineTheme.native_snapshots.every(row => row.process_id === baselineLaunch.gui_pid
    && row.process_started_at === baselineLaunch.gui_started_at
    && row.binary_sha256 === baselineLaunch.sha256["target/release/gwt.exe"]));
  assert.ok(baselineTheme.elapsed_seconds >= (theme === "dark" ? 1800 : 30));
  assert.equal(baselineTheme.idle_readiness.ready, true);
  assert.equal(baselineTheme.idle_readiness.matched_worktree_groups, 12);
  assert.equal(baselineTheme.workload.panes, 12);
  assert.equal(baselineTheme.workload.history_lines_per_pane, 600);
  assert.equal(baselineTheme.workload.ready_requests_per_minute, 66);
  assert.equal(baselineTheme.workload.provider_sha256, baselineLaunch.provider_sha256);
  assert.deepEqual(await json(`before-${theme}-errors.json`), []);
  const baselineP95 = p95(baselineTheme.input_latency);
  const actualP95 = p95(result.input_latency);
  assert.ok(actualP95 <= baselineP95 + 500,
    `${theme}: visible input p95 ${actualP95}ms exceeds baseline ${baselineP95}ms + 500ms scheduling tolerance`);
  assert.equal(result.native_snapshots.length, 2);
  const [start, end] = result.native_snapshots;
  assert.equal(start.process_started_at, end.process_started_at);
  assert.ok(Math.abs(end.elapsed_seconds - start.elapsed_seconds - result.elapsed_seconds) < 1e-9);
  assert.ok(Math.abs(end.process_cpu_seconds - start.process_cpu_seconds - result.process_cpu_seconds) < 1e-9);
  assert.ok(Math.abs(result.process_cpu_seconds / result.elapsed_seconds - result.average_cores) < 1e-9);
  assert.ok(result.native_snapshots.every(row => row.process_id === launch.gui_pid
    && row.binary_sha256 === launch.sha256["target/release/gwt.exe"]));
  assert.equal(result.user_verification_result, "n/a (autonomous)");
  assert.equal(result.agent_visual_check, "pass");
  assert.deepEqual(await json(`after-${theme}-errors.json`), []);
  console.log(`PASS ${theme}: ${result.process_cpu_seconds} CPU seconds / ${result.elapsed_seconds} seconds = ${result.average_cores} cores; 12 quiet Agent PTYs, ${successes} authenticated reads, 24 input probes`);
}
