import { test } from "node:test";
import assert from "node:assert/strict";

import { WINDOW_RUNTIME_STATES } from "../protocol-enums.js";
import {
  LEGACY_WINDOW_RUNTIME_STATE_ALIASES,
  WINDOW_RUNTIME_STATE_LABELS,
  mapAgentTelemetryState,
  normalizeWindowRuntimeState,
  presetSupportsWaitingStatus,
  selectNextAgentFocusWindowId,
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

test("telemetry mapping projects runtime states to status strip states", () => {
  assert.equal(mapAgentTelemetryState("running"), "running");
  assert.equal(mapAgentTelemetryState("starting"), "running");
  assert.equal(mapAgentTelemetryState("ready"), "idle");
  assert.equal(mapAgentTelemetryState("idle"), "idle");
  // FR-039 (安心): waiting is its own LOUD telemetry state, not quiet idle.
  assert.equal(mapAgentTelemetryState("waiting"), "waiting");
  assert.equal(mapAgentTelemetryState("stopped"), "done");
  assert.equal(mapAgentTelemetryState("exited"), "done");
  assert.equal(mapAgentTelemetryState("error"), "error");
  assert.equal(mapAgentTelemetryState("future-state"), "idle");
});

test("agent focus candidates include Canvas presets or non-empty agent ids, including hidden tabs", () => {
  const windows = [
    {
      id: "idle-agent-id",
      preset: "shell",
      agent_id: "custom-agent",
      status: "idle",
    },
    {
      id: "starting-hidden-tab",
      preset: "claude",
      status: "starting",
      placement: { kind: "canvas" },
      tab_group_id: "group-1",
      tab_group_active: false,
    },
    { id: "running-codex", preset: "codex", status: "running" },
    { id: "plain-shell", preset: "shell", status: "running" },
    {
      id: "blank-agent-id",
      preset: "shell",
      agent_id: "",
      status: "running",
    },
    {
      id: "kanban-agent",
      preset: "agent",
      status: "running",
      placement: { kind: "agent_kanban" },
    },
  ];

  assert.equal(
    selectNextAgentFocusWindowId(windows, null, "forward"),
    "starting-hidden-tab",
  );
  assert.equal(
    selectNextAgentFocusWindowId(windows, "starting-hidden-tab", "forward"),
    "running-codex",
  );
  assert.equal(
    selectNextAgentFocusWindowId(windows, "running-codex", "forward"),
    "idle-agent-id",
  );
});

test("agent focus cycling prioritizes runtime buckets while preserving workspace order", () => {
  const windows = [
    { id: "idle-first", preset: "agent", status: "idle" },
    { id: "running-first", preset: "agent", status: "running" },
    { id: "waiting-second", preset: "agent", status: "waiting" },
    { id: "starting-second", preset: "agent", status: "starting" },
    { id: "error-first", preset: "agent", status: "error" },
    { id: "stopped-second", preset: "agent", status: "stopped" },
  ];

  assert.equal(
    selectNextAgentFocusWindowId(windows, "running-first", "forward"),
    "starting-second",
  );
  assert.equal(
    selectNextAgentFocusWindowId(windows, "starting-second", "forward"),
    "idle-first",
  );
  assert.equal(
    selectNextAgentFocusWindowId(windows, "idle-first", "forward"),
    "waiting-second",
  );
  assert.equal(
    selectNextAgentFocusWindowId(windows, "waiting-second", "forward"),
    "error-first",
  );
  assert.equal(
    selectNextAgentFocusWindowId(windows, "error-first", "forward"),
    "stopped-second",
  );
});

test("agent focus cycling can prioritize composed live runtime state", () => {
  const windows = [
    { id: "persisted-idle", preset: "agent", status: "idle" },
    { id: "live-running", preset: "agent", status: "idle" },
  ];
  const liveStates = new Map([
    ["persisted-idle", "idle"],
    ["live-running", "running"],
  ]);

  assert.equal(
    selectNextAgentFocusWindowId(
      windows,
      null,
      "forward",
      (windowData) => liveStates.get(windowData.id),
    ),
    "live-running",
  );
});

test("agent focus cycling places unknown persisted and live states in the final bucket", () => {
  const persistedWindows = [
    { id: "future", preset: "agent", status: "future-state" },
    { id: "waiting", preset: "agent", status: "waiting" },
    { id: "running", preset: "agent", status: "running" },
  ];

  assert.equal(
    selectNextAgentFocusWindowId(persistedWindows, null, "forward"),
    "running",
  );
  assert.equal(
    selectNextAgentFocusWindowId(persistedWindows, "waiting", "forward"),
    "future",
  );

  const liveWindows = [
    { id: "live-future", preset: "agent", status: "running" },
    { id: "live-idle", preset: "agent", status: "running" },
  ];
  const liveStates = new Map([
    ["live-future", "future-state"],
    ["live-idle", "idle"],
  ]);
  assert.equal(
    selectNextAgentFocusWindowId(
      liveWindows,
      null,
      "forward",
      (windowData) => liveStates.get(windowData.id),
    ),
    "live-idle",
  );
});

test("agent focus cycling wraps in both directions", () => {
  const windows = [
    { id: "waiting", preset: "agent", status: "waiting" },
    { id: "running", preset: "agent", status: "running" },
    { id: "stopped", preset: "agent", status: "stopped" },
  ];

  assert.equal(
    selectNextAgentFocusWindowId(windows, "stopped", "forward"),
    "running",
  );
  assert.equal(
    selectNextAgentFocusWindowId(windows, "running", "backward"),
    "stopped",
  );
  assert.equal(
    selectNextAgentFocusWindowId(windows, "waiting", "backward"),
    "running",
  );
});

test("agent focus cycling starts at the directional edge when focus is not a candidate", () => {
  const windows = [
    { id: "idle", preset: "agent", status: "idle" },
    { id: "running", preset: "agent", status: "running" },
    { id: "non-agent", preset: "shell", status: "running" },
    { id: "error", preset: "agent", status: "error" },
  ];

  assert.equal(
    selectNextAgentFocusWindowId(windows, "non-agent", "forward"),
    "running",
  );
  assert.equal(
    selectNextAgentFocusWindowId(windows, "non-agent", "backward"),
    "error",
  );
  assert.equal(
    selectNextAgentFocusWindowId(windows, "missing", "forward"),
    "running",
  );
  assert.equal(
    selectNextAgentFocusWindowId(windows, "missing", "backward"),
    "error",
  );
});

test("agent focus cycling returns null for no candidates and the same id for one candidate", () => {
  assert.equal(
    selectNextAgentFocusWindowId(
      [
        { id: "shell", preset: "shell", status: "running" },
        {
          id: "kanban-agent",
          preset: "agent",
          status: "running",
          placement: { kind: "agent_kanban" },
        },
      ],
      "shell",
      "forward",
    ),
    null,
  );

  const single = [{ id: "only-agent", preset: "agent", status: "idle" }];
  assert.equal(
    selectNextAgentFocusWindowId(single, "only-agent", "forward"),
    "only-agent",
  );
  assert.equal(
    selectNextAgentFocusWindowId(single, "only-agent", "backward"),
    "only-agent",
  );
});
