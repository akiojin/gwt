// Validate saved #5379 evidence; this does not rerun the measurement.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile, readdir, writeFile } from "node:fs/promises";
import { createReadStream } from "node:fs";
import { createInterface } from "node:readline";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gunzipSync, gzipSync } from "node:zlib";

const directory = dirname(fileURLToPath(import.meta.url));
const checkout = resolve(directory, "../../..");
const json = async name => JSON.parse(await readFile(resolve(directory, name), "utf8"));
const provenance = await json("provenance.json");
// PM decision 33031353: validate capped-failure records for the delivery slice;
// this never certifies the unchanged 100ms latency acceptance criterion.
const pendingLatency = provenance.latency_acceptance;
const deferred = pendingLatency !== undefined;
let environment;
let deferredLatencyFailure = false;
if (deferred) {
  assert.equal(pendingLatency.state, "deferred-environment");
  assert.equal(pendingLatency.owner_issue, 5816);
  assert.equal(pendingLatency.decision_id, "33031353-c4f9-4cad-963b-afd327090da2");
  assert.equal(pendingLatency.budget_ms, 100);
  assert.equal(createHash("sha256").update(await readFile(resolve(directory, "retained-failures.json")))
    .digest("hex"), provenance.retained_failure_evidence_sha256, "retained failure evidence differs");
  const bytes = await readFile(resolve(directory, "after-environment.json"));
  assert.equal(createHash("sha256").update(bytes).digest("hex"), pendingLatency.environment_sha256);
  environment = JSON.parse(bytes.toString("utf8"));
  assert.equal(environment.measured_source.worktree_clean, true);
  assert.deepEqual(environment.measured_source.source_sha256, provenance.treatment_source_sha256);
  assert.equal(Object.keys(provenance.treatment_source_sha256).length, 11);
  assert.ok(Object.hasOwn(provenance.treatment_source_sha256, "crates/gwt-core/src/usage/limit_notice.rs"));
  assert.ok(environment.job_observations.length >= 2, "multiple cap observations required");
  assert.equal(new Set(environment.job_observations.map(proof => proof.observed_at_utc)).size,
    environment.job_observations.length, "cap observations must be distinct");
}
function validateResult(arm, run) {
  if (!deferred || arm !== "after") { assert.equal(run.passed, true, arm + " did not pass"); return; }
  assert.equal(environment.gui_pid, run.fixture.gui_pid);
  assert.equal(environment.gui_started_at, run.fixture.gui_started_at);
  assert.equal(environment.measured_source.head, run.fixture.checkout_head);
  assert.equal(environment.measured_source.binary_sha256, run.fixture.binary_sha256.toLowerCase());
  for (const proof of environment.job_observations) {
    assert.equal(proof.read_only, true);
    assert.equal(proof.status, "observed");
    assert.equal(proof.target_pid, run.fixture.gui_pid, "cap proof PID differs");
    assert.equal(proof.target_started_at_utc, run.fixture.gui_started_at, "cap proof start differs");
    const at = Date.parse(proof.observed_at_utc);
    assert.ok(Number.isFinite(at) && at >= Date.parse(run.fixture.gui_started_at)
      && at <= Date.parse(environment.gui_stopped_at), "cap proof is outside GUI lifetime");
    assert.ok(proof.matching_jobs.some(job => job.target_membership_confirmed === true
      && job.cpu_rate.status === "observed" && job.cpu_rate.enabled === true
      && job.cpu_rate.hard_cap_enabled === true && job.cpu_rate.cpu_rate_raw === 800),
    "the approved capped environment was not observed");
  }
  assert.ok(typeof run.passed === "boolean");
  const failures = environment.measurement_failures.filter(failure => failure.theme === run.theme);
  if (run.passed) assert.equal(failures.length, 0, "passing theme has a recorded test error");
  if (!run.passed) {
    const output = run.native_dispatch?.events?.find(event => event.event === "RuntimeOutput");
    assert.ok(output?.max_ms >= 100 || run.native_dispatch?.output_batches?.max_ms >= 100,
      "only a recorded latency failure can be deferred");
    assert.ok(failures.length > 0, "original runner latency error is required");
    for (const failure of failures) assert.ok(
      failure.message.includes("RuntimeOutput dispatch stays below 100ms")
      || failure.message.includes("each GUI output batch stays below 100ms"),
      "a non-latency runner error cannot be deferred");
    deferredLatencyFailure = true;
  }
}
function validateCleanup(shutdown, arm) {
  assert.equal(shutdown.owned_gui_stopped, true);
  assert.deepEqual(shutdown.cleanup_errors, []);
  assert.equal(shutdown.test_exit_code, deferred && arm === "after" ? 1 : 0);
  if (deferred && arm === "after") assert.equal(shutdown.stopped_at, environment.gui_stopped_at);
}

if (process.argv[2] === "--collect") {
  const home = resolve(process.argv[3]);
  const arm = process.argv[4];
  assert.ok(["before", "after"].includes(arm));
  const shutdown = JSON.parse(await readFile(resolve(home, "shutdown.json"), "utf8"));
  validateCleanup(shutdown, arm);
  for (const theme of ["dark", "light"]) {
    const label = arm + "-" + theme;
    const run = JSON.parse(await readFile(resolve(home, label + ".json"), "utf8"));
    validateResult(arm, run);
    assert.equal(run.duration_seconds, theme === "dark" ? 1800 : 10, label);
    const roots = [resolve(home, ".gwt/logs"), ...run.fixture.projects.map(project =>
      resolve(home, ".gwt/projects", project.project_key, "logs"))];
    const dispatch = [], batches = [];
    for (const root of roots) for (const name of (await readdir(root).catch(() => [])).filter(name => name.startsWith("gwt.log."))) {
      const lines = createInterface({ input: createReadStream(resolve(root, name)), crlfDelay: Infinity });
      for await (const line of lines) {
        let row;
        try { row = JSON.parse(line); } catch { continue; }
        const timestamp = Date.parse(row.timestamp), fields = row.fields;
        if (row.target !== "gwt.frontend.timing" || timestamp < Date.parse(run.started_at) ||
          timestamp > Date.parse(run.ended_at) || !Number.isFinite(fields?.elapsed_ms)) continue;
        if (fields.marker === "issue_5379_runtime_output_batch") batches.push(line);
        else if (fields.message === "GUI event loop dispatch handled" ||
          /^[A-Za-z]+ blocked the GUI event loop for \d+ms$/.test(fields.message || "")) dispatch.push(line);
      }
    }
    await writeFile(resolve(directory, label + ".json"), JSON.stringify(run, null, 2) + "\n");
    await writeFile(resolve(directory, label + "-native.jsonl.gz"), gzipSync(dispatch.join("\n") + "\n"));
    if (arm === "after") await writeFile(resolve(directory, label + "-batches.jsonl.gz"), gzipSync(batches.join("\n") + "\n"));
  }
  await writeFile(resolve(directory, arm + "-cleanup.json"), JSON.stringify(shutdown, null, 2) + "\n");
  if (deferred && arm === "after") assert.ok(deferredLatencyFailure, "no deferred latency failure recorded");
  console.log("Collected " + arm + " evidence and original timing rows; latency acceptance: "
    + (deferred && arm === "after" ? "pending Issue #5816" : "pass") + ".");
  process.exit(0);
}
assert.match(provenance.baseline_source_sha, /^[a-f0-9]{40}$/i);
for (const hash of [provenance.baseline_binary_sha256, provenance.treatment_binary_sha256]) {
  assert.match(hash, /^[a-f0-9]{64}$/i);
}
const sources = Object.entries(provenance.treatment_source_sha256);
assert.ok(sources.length > 0, "source fingerprint map is empty");
for (const [path, expected] of sources) {
  let source = await readFile(resolve(checkout, path));
  // Retain the measured fingerprint while proving the only later test edits
  // are the explicitly recorded hygiene comments (not executable changes).
  for (const comment of provenance.post_measurement_test_comments?.[path] || []) {
    assert.ok(comment.trim().startsWith("// test-hygiene: allow-wall-clock-deadline "));
    let text = source.toString("utf8");
    const line = comment + (text.includes(comment + "\r\n") ? "\r\n" : "\n");
    assert.ok(text.includes(line), "missing recorded test comment: " + path);
    source = Buffer.from(text.replace(line, ""));
  }
  // Git may check out the measured source with LF or CRLF line endings.
  const lf = source.toString("utf8").replaceAll("\r\n", "\n");
  const variants = [source, Buffer.from(lf), Buffer.from(lf.replaceAll("\n", "\r\n"))];
  assert.ok(variants.some(value => createHash("sha256").update(value).digest("hex") === expected.toLowerCase()),
    "source changed after measurement: " + path);
}

const indices = [0, 1, 2, 3, 4];
const runs = {};
for (const arm of ["before", "after"]) for (const theme of ["dark", "light"]) {
  const label = arm + "-" + theme;
  const run = await json(label + ".json");
  const seconds = theme === "dark" ? 1800 : 10;
  const bytes = seconds * 8192;
  assert.equal(run.issue, 5379, label);
  assert.equal(run.fixture.arm, arm, label);
  assert.equal(run.theme, theme, label);
  validateCleanup(await json(arm + "-cleanup.json"), arm);
  validateResult(arm, run);
  assert.equal(run.full_acceptance_observation, theme === "dark", label);
  assert.equal(run.duration_seconds, seconds, label);
  assert.equal(run.workload, provenance.workload, label);
  const start = Date.parse(run.started_at), end = Date.parse(run.ended_at);
  assert.ok(Number.isFinite(start) && Number.isFinite(end), label + " has invalid timestamps");
  assert.equal(run.actual_elapsed_ms, end - start, label + " elapsed time does not match timestamps");
  assert.ok(run.actual_elapsed_ms >= seconds * 1000, label + " observation is too short");
  const binary = arm === "before" ? provenance.baseline_binary_sha256 : provenance.treatment_binary_sha256;
  assert.equal(run.fixture.binary_sha256.toLowerCase(), binary.toLowerCase(), label + " binary differs");
  assert.deepEqual(run.browser_errors, [], label + " browser errors");
  assert.equal(run.user_verification_result, "n/a (autonomous)", label);
  assert.equal(run.agent_visual_check, run.passed ? "pass" : "fail", label);

  assert.equal(run.producer_receipts.length, 5, label);
  assert.deepEqual(run.producer_receipts.map(row => row.index).sort(), indices, label + " producer indices");
  assert.equal(new Set(run.producer_receipts.map(row => row.pid)).size, 5, label + " producer PIDs");
  assert.equal(new Set(run.producer_receipts.map(row => row.session)).size, 5, label + " producer Sessions");
  for (const row of run.producer_receipts) {
    assert.ok(Number.isInteger(row.pid) && row.pid > 0, label);
    assert.ok(typeof row.session === "string" && row.session.length > 0, label);
    assert.equal(row.run_id, label, label);
    assert.equal(row.complete, true, label);
    assert.equal(row.sequence, seconds, label);
    assert.equal(row.bytes, bytes, label + " producer payload");
  }
  assert.equal(run.wire.length, 5, label);
  assert.deepEqual(run.wire.map(row => row.index).sort(), indices, label + " wire indices");
  for (const row of run.wire) {
    assert.ok(Number.isInteger(row.output_frames) && row.output_frames > 0, label);
    assert.ok(Number.isInteger(row.output_bytes) && row.output_bytes >= bytes, label + " wire payload");
    assert.deepEqual(row.sequences, Array.from({ length: seconds }, (_, index) => index + 1), label + " missing sequence");
  }

  const native = run.native_dispatch;
  assert.ok(Number.isInteger(native.samples) && native.samples > 0, label + " native instrumentation inactive");
  const rawGroups = new Map();
  const rawLines = gunzipSync(await readFile(resolve(directory, label + "-native.jsonl.gz")))
    .toString("utf8").split("\n").filter(line => line.trim());
  assert.ok(rawLines.length > 0, label + " native raw evidence is empty");
  for (const line of rawLines) {
    const row = JSON.parse(line), timestamp = Date.parse(row.timestamp), fields = row.fields;
    assert.ok(Number.isFinite(timestamp) && timestamp >= start && timestamp <= end, label + " raw dispatch outside observation");
    assert.equal(row.target, "gwt.frontend.timing", label);
    assert.ok(fields && Number.isInteger(fields.elapsed_ms) && fields.elapsed_ms >= 0, label);
    assert.ok(fields.message === "GUI event loop dispatch handled" ||
      /^[A-Za-z]+ blocked the GUI event loop for \d+ms$/.test(fields.message || ""), label + " raw row is not an outer dispatch");
    assert.ok(typeof fields.event === "string" && fields.event.length > 0, label);
    const group = rawGroups.get(fields.event) || { event: fields.event, count: 0, stalls: 0, max_ms: 0 };
    group.count++;
    if (fields.elapsed_ms >= 100) group.stalls++;
    group.max_ms = Math.max(group.max_ms, fields.elapsed_ms);
    rawGroups.set(group.event, group);
  }
  assert.equal(rawLines.length, native.samples, label + " raw dispatch count differs");
  const byEvent = (a, b) => a.event.localeCompare(b.event);
  assert.deepEqual([...rawGroups.values()].sort(byEvent), [...native.events].sort(byEvent),
    label + " native aggregate differs from original dispatch rows");
  assert.ok(native.events.length > 0, label);
  assert.equal(new Set(native.events.map(row => row.event)).size, native.events.length, label);
  for (const event of native.events) {
    assert.ok(Number.isInteger(event.count) && event.count > 0, label);
    assert.ok(Number.isInteger(event.stalls) && event.stalls >= 0 && event.stalls <= event.count, label);
    assert.ok(Number.isInteger(event.max_ms) && event.max_ms >= 0, label);
    const slow = native.slow_rows.filter(row => row.fields.event === event.event);
    assert.equal(slow.length, event.stalls, label + " per-event stall count differs");
    assert.equal(event.max_ms >= 100, event.stalls > 0, label + " stall threshold differs");
    if (slow.length) assert.equal(event.max_ms, Math.max(...slow.map(row => row.fields.elapsed_ms)), label);
  }
  for (const row of native.slow_rows) {
    assert.ok(row.fields.elapsed_ms >= 100, label + " non-stall included in slow rows");
    assert.ok(Date.parse(row.timestamp) >= start && Date.parse(row.timestamp) <= end, label + " slow row outside observation");
  }
  assert.equal(native.samples, native.events.reduce((sum, row) => sum + row.count, 0), label + " sample count differs");
  assert.equal(native.stalls, native.events.reduce((sum, row) => sum + row.stalls, 0), label + " stall sum differs");
  assert.equal(native.stalls, native.slow_rows.length, label + " slow rows differ");
  assert.equal(native.max_ms, Math.max(...native.events.map(row => row.max_ms)), label + " maximum differs");
  const output = native.events.find(row => row.event === "RuntimeOutput");
  assert.ok(output?.count > 0, label + " RuntimeOutput inactive");
  if (arm === "after") {
    if (!deferred || run.passed) assert.ok(output.max_ms < 100, label + " RuntimeOutput exceeded 100ms");
    const batchRows = gunzipSync(await readFile(resolve(directory, label + "-batches.jsonl.gz")))
      .toString("utf8").split("\n").filter(line => line.trim()).map(line => JSON.parse(line));
    assert.ok(batchRows.length > 0, label + " no actual GUI batch measurements");
    for (const row of batchRows) {
      assert.equal(row.fields.marker, "issue_5379_runtime_output_batch", label);
      assert.equal(row.target, "gwt.frontend.timing", label);
      assert.ok(Date.parse(row.timestamp) >= start && Date.parse(row.timestamp) <= end, label);
      assert.ok(Number.isInteger(row.fields.elapsed_ms) && row.fields.elapsed_ms >= 0, label);
    }
    assert.deepEqual(native.output_batches, { count: batchRows.length,
      stalls: batchRows.filter(row => row.fields.elapsed_ms >= 100).length,
      max_ms: Math.max(...batchRows.map(row => row.fields.elapsed_ms)) }, label + " batch aggregate differs");
    if (!deferred || run.passed) assert.ok(native.output_batches.max_ms < 100, label + " GUI batch exceeded 100ms");
  }
  if (theme === "dark") assert.ok(native.summaries.length > 0, label + " minute summaries missing");
  for (const row of native.summaries) {
    assert.equal(row.type, "gui_dispatch_summary", label);
    assert.equal(row.dispatch.pid, run.fixture.gui_pid, label);
    assert.equal(row.dispatch.dropped_windows, 0, label + " dispatch summary windows dropped");
  }
  runs[label] = { elapsed_ms: run.actual_elapsed_ms, native_samples: native.samples, native_raw_rows: rawLines.length,
    stalls_ge_100ms: native.stalls, max_ms: native.max_ms, runtime_output: output,
    output_batches: native.output_batches, producer_bytes: bytes * 5,
    wire_bytes: run.wire.reduce((sum, row) => sum + row.output_bytes, 0) };
}
const comparisons = {};
for (const theme of ["dark", "light"]) {
  const before = runs["before-" + theme], after = runs["after-" + theme];
  comparisons[theme] = { before, after, stall_count_delta: after.stalls_ge_100ms - before.stalls_ge_100ms,
    max_ms_delta: after.max_ms - before.max_ms };
}
if (deferred) assert.ok(deferredLatencyFailure, "no deferred latency failure recorded");
console.log(JSON.stringify({ record_integrity: "pass",
  latency_acceptance: deferred ? { state: "pending", owner_issue: 5816, budget_ms: 100 }
    : { state: "pass", budget_ms: 100 }, source_files_verified: sources.length,
  baseline_source_sha: provenance.baseline_source_sha, comparisons }, null, 2));
