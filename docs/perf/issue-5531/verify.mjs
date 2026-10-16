import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const read = name => JSON.parse(readFileSync(new URL(name, import.meta.url), "utf8"));
const checkout = new URL("../../../", import.meta.url);
const baselineFrontendRef = "6864fa6f497a03368431b4b35e1a4fa019267091";
const harnessPath = "crates/gwt/playwright/tests/socket-receive-backpressure-live.spec.ts";
const baselineSource = path => execFileSync("git", ["show", `${baselineFrontendRef}:${path}`],
  { cwd: checkout, encoding: "utf8" });
const measuredHarness = new URL("measured-harness.spec.ts.txt", import.meta.url);
const normalize = source => source.replace(/\r\n/g, "\n");
const digest = source => createHash("sha256").update(normalize(source)).digest("hex");
assert.equal(digest(readFileSync(measuredHarness, "utf8")),
  read("after-dark-multikey-launch.json").test_sha256[harnessPath]);
assert.equal(normalize(readFileSync(measuredHarness, "utf8"))
  .replace('const input = "GWT_INPUT_5531";', 'const input = "X";')
  .replace("26 controller", "per-event controller"), normalize(baselineSource(harnessPath)),
"the archived multi-key harness differs from the pinned baseline only in its input probe and comment");
for (const label of ["before", "after"]) {
  for (const theme of ["dark", "light"]) {
    const launch = read(label === "before" ? "before-launch.json" : `after-${theme}-launch.json`);
    assert.equal(launch.label, label);
    assert.equal(launch.acceptance_duration, true);
    for (const [path, hash] of Object.entries({
      ...Object.fromEntries(Object.entries(launch.source_sha256).map(([name, hash]) => [`crates/gwt/web/${name}`, hash])),
      ...launch.test_sha256,
    })) {
      const source = label === "before"
        ? baselineSource(path)
        : readFileSync(new URL(path, checkout), "utf8");
      assert.equal(digest(source), hash,
        `${label}: measured source changed: ${path}`);
    }
    const run = read(`${label}-${theme}.json`);
    assert.equal(run.label, label);
    assert.equal(run.theme, theme);
    assert.equal(run.baseline_sha, launch.baseline_sha);
    assert.equal(String(run.frontend.owned_pid), String(launch.owned_pid));
    assert.equal(run.frontend.isolated_home, launch.isolated_home);
    assert.equal(run.acceptance_duration, true);
    assert.ok(run.background_elapsed_ms >= 1800000);
    for (const key of ["controller_restore_started_at_ms", "browser_performance_time_origin_ms", "controller_restore_start_performance_ms"]) {
      assert.ok(Number.isFinite(run[key]), `${label}/${theme}: missing physical foreground clock: ${key}`);
    }
    assert.ok(Math.abs(run.controller_restore_started_at_ms - run.browser_performance_time_origin_ms
      - run.controller_restore_start_performance_ms) < 1, "foreground clock must start at the controller restore request");
    assert.ok(run.metrics.messages > 4000);
    assert.ok(Number.isFinite(run.metrics.max_long_task_ms), "retain completed long tasks even on recovery timeout");
    assert.ok(run.samples.length >= 2, "continuous traffic needs interval samples");
    for (const sample of run.samples) assert.equal(sample.visibility, "hidden");
    const trafficSamples = label === "after" ? run.traffic_samples : run.samples;
    assert.ok(Array.isArray(trafficSamples) && trafficSamples.length >= 2,
      "after traffic must be observed independently of the background renderer");
    let previous = { at_ms: 0, ticks: 0, frames: 0 };
    let largestGap = 0;
    for (const sample of trafficSamples) {
      assert.ok(sample.at_ms > previous.at_ms);
      largestGap = Math.max(largestGap, sample.at_ms - previous.at_ms);
      if (label === "after") {
        assert.ok(sample.at_ms - previous.at_ms <= 60000, "after traffic observations must cover every minute");
      }
      assert.ok(sample.ticks > previous.ticks && sample.frames > previous.frames,
        "each observed interval must continue sending events");
      previous = sample;
    }
    if (label === "after") assert.ok(run.background_elapsed_ms - previous.at_ms <= 60000);
    assert.ok(run.traffic.ticks >= previous.ticks && run.traffic.frames >= previous.frames);
    assert.deepEqual(run.errors, []);
    assert.deepEqual(run.fixture_errors, []);
    if (label === "after") {
      assert.equal(run.outcome, "complete");
      assert.equal(run.recovery_error, null);
      assert.equal(run.foreground_metrics_complete, true);
      assert.ok(Number.isFinite(run.input_ms) && run.input_ms <= 2000);
      assert.ok(Number.isFinite(run.snapshot_ms) && run.snapshot_ms <= 2000);
      assert.ok(Number.isFinite(run.metrics.maxQueue) && run.metrics.maxQueue <= 2048);
      assert.ok(Number.isFinite(run.metrics.resume_max_long_task_ms));
      assert.ok(Number.isFinite(run.metrics.settled_max_long_task_ms) && run.metrics.settled_max_long_task_ms < 200);
      assert.ok(run.ready_requests_after_resume > 0);
      assert.ok(run.metrics.resumes > 0);
    } else if (run.outcome === "timeout") {
      assert.ok(run.recovery_timeout_ms > 2000, "negative reference must exceed the acceptance deadline");
      assert.ok(run.recovery_stage && run.recovery_error);
      assert.equal(run.input_ms, null);
      assert.equal(run.snapshot_ms, null);
      if (run.foreground_metrics_complete) {
        assert.ok(Number.isFinite(run.metrics.resume_max_long_task_ms));
        assert.ok(Number.isFinite(run.metrics.settled_max_long_task_ms));
      } else {
        assert.equal(run.metrics.resume_max_long_task_ms, null);
        assert.equal(run.metrics.settled_max_long_task_ms, null);
      }
    } else {
      assert.equal(run.outcome, "complete");
    }
    console.log(`${label}/${theme}: ${run.outcome}, ${run.background_elapsed_ms}ms background, queue ${run.metrics.maxQueue}, `
      + `largest traffic observation gap ${largestGap}ms, `
      + `completed long task ${run.metrics.max_long_task_ms}ms, `
      + `resume/settled long task ${run.metrics.resume_max_long_task_ms}/${run.metrics.settled_max_long_task_ms}ms, `
      + `input ${run.input_ms}ms, snapshot ${run.snapshot_ms}ms`);
  }
}
