import { test } from "node:test";
import assert from "node:assert/strict";

import { WINDOW_RUNTIME_STATES } from "../protocol-enums.js";
import {
  LEGACY_WINDOW_RUNTIME_STATE_ALIASES,
  WINDOW_RUNTIME_STATE_LABELS,
  mapAgentTelemetryState,
  normalizeWindowRuntimeState,
  presetSupportsWaitingStatus,
  windowRuntimeLabel,
} from "../window-runtime-state.js";

test("label table covers exactly the generated wire states", () => {
  assert.deepEqual(
    Object.keys(WINDOW_RUNTIME_STATE_LABELS).sort(),
    [...WINDOW_RUNTIME_STATES].sort(),
  );
});

test("label mapping equals the expected English labels", () => {
  assert.deepEqual(WINDOW_RUNTIME_STATE_LABELS, {
    running: "Running",
    starting: "Starting",
    idle: "Idle",
    waiting: "Waiting",
    stopped: "Stopped",
    error: "Error",
  });
});

test("legacy aliases normalize to current wire states", () => {
  assert.equal(normalizeWindowRuntimeState("not_started", "agent"), "starting");
  assert.equal(normalizeWindowRuntimeState("notstarted", "agent"), "starting");
  assert.equal(normalizeWindowRuntimeState("not-started", "agent"), "starting");
  assert.equal(normalizeWindowRuntimeState("ready", "agent"), "idle");
  assert.equal(normalizeWindowRuntimeState("exited", "agent"), "stopped");
});

test("every legacy alias targets a generated wire state", () => {
  for (const target of Object.values(LEGACY_WINDOW_RUNTIME_STATE_ALIASES)) {
    assert.ok(
      WINDOW_RUNTIME_STATES.includes(target),
      `legacy alias target ${target} is not a generated wire state`,
    );
  }
});

test("waiting is demoted to running for non-agent presets", () => {
  assert.equal(normalizeWindowRuntimeState("waiting", "shell"), "running");
  assert.equal(normalizeWindowRuntimeState("waiting", undefined), "running");
  assert.equal(normalizeWindowRuntimeState("waiting", "agent"), "waiting");
  assert.equal(normalizeWindowRuntimeState("waiting", "claude"), "waiting");
  assert.equal(normalizeWindowRuntimeState("waiting", "codex"), "waiting");
});

test("presetSupportsWaitingStatus only accepts agent presets", () => {
  assert.equal(presetSupportsWaitingStatus("agent"), true);
  assert.equal(presetSupportsWaitingStatus("claude"), true);
  assert.equal(presetSupportsWaitingStatus("codex"), true);
  assert.equal(presetSupportsWaitingStatus("shell"), false);
  assert.equal(presetSupportsWaitingStatus(""), false);
  assert.equal(presetSupportsWaitingStatus(undefined), false);
});

test("unknown or missing states fall back to running", () => {
  assert.equal(normalizeWindowRuntimeState("bogus", "agent"), "running");
  assert.equal(normalizeWindowRuntimeState("", "agent"), "running");
  assert.equal(normalizeWindowRuntimeState(undefined, "agent"), "running");
  assert.equal(normalizeWindowRuntimeState(null, "shell"), "running");
});

test("normalization lowercases incoming states", () => {
  assert.equal(normalizeWindowRuntimeState("Running", "shell"), "running");
  assert.equal(normalizeWindowRuntimeState("NotStarted", "agent"), "starting");
});

test("windowRuntimeLabel maps states and falls back to Running", () => {
  assert.equal(windowRuntimeLabel("idle"), "Idle");
  assert.equal(windowRuntimeLabel("error"), "Error");
  assert.equal(windowRuntimeLabel("nonsense"), "Running");
  assert.equal(windowRuntimeLabel(undefined), "Running");
});

test("telemetry mapping projects runtime states to semantic states", () => {
  assert.equal(mapAgentTelemetryState("running"), "active");
  assert.equal(mapAgentTelemetryState("starting"), "not_started");
  assert.equal(mapAgentTelemetryState("ready"), "idle");
  assert.equal(mapAgentTelemetryState("idle"), "idle");
  assert.equal(mapAgentTelemetryState("waiting"), "idle");
  assert.equal(mapAgentTelemetryState("stopped"), "done");
  assert.equal(mapAgentTelemetryState("exited"), "done");
  assert.equal(mapAgentTelemetryState("error"), "blocked");
  assert.equal(mapAgentTelemetryState("future-state"), "idle");
});
