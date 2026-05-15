import assert from "node:assert/strict";
import test from "node:test";

import { createUiTraceProfiler } from "../ui-trace-profiler.js";

function fakePointerEvent(overrides = {}) {
  return {
    type: "pointermove",
    pointerId: 7,
    button: 0,
    buttons: 1,
    clientX: 120,
    clientY: 48,
    target: {
      id: "canvas-stage",
      className: "canvas-stage",
      tagName: "DIV",
      dataset: { preset: "agent" },
    },
    ...overrides,
  };
}

test("UI trace profiler records sanitized pointer diagnostics", () => {
  let now = 100;
  const profiler = createUiTraceProfiler({
    now: () => now,
    sessionId: () => "trace-test",
  });

  profiler.start();
  profiler.recordPointer("pointer_move_ignored", fakePointerEvent(), {
    gesture: "resize",
    accepted: false,
    reason: "pointer_id_mismatch",
    expected_pointer_id: 3,
    data_base64: "must-not-leak",
  });
  now = 132;
  const payload = profiler.stop();

  assert.equal(payload.session_id, "trace-test");
  assert.equal(payload.entries.length, 2);
  assert.deepEqual(payload.entries[1], {
    ts: 100,
    kind: "pointer_move_ignored",
    gesture: "resize",
    accepted: false,
    reason: "pointer_id_mismatch",
    expected_pointer_id: 3,
    pointer_id: 7,
    button: 0,
    buttons: 1,
    client_x: 120,
    client_y: 48,
    target: "div#canvas-stage.canvas-stage[preset=agent]",
  });
  assert.equal(JSON.stringify(payload).includes("must-not-leak"), false);
});

test("UI trace profiler keeps a bounded ring buffer", () => {
  let now = 0;
  const profiler = createUiTraceProfiler({
    maxEntries: 3,
    now: () => now++,
    sessionId: () => "bounded",
  });

  profiler.start();
  profiler.record("one", { value: 1 });
  profiler.record("two", { value: 2 });
  profiler.record("three", { value: 3 });
  profiler.record("four", { value: 4 });
  const payload = profiler.stop();

  assert.deepEqual(
    payload.entries.map((entry) => entry.kind),
    ["two", "three", "four"],
  );
  assert.equal(payload.dropped_entries, 2);
});

test("UI trace profiler measures callback duration", () => {
  let now = 20;
  const profiler = createUiTraceProfiler({
    now: () => now,
    sessionId: () => "measure",
  });

  profiler.start();
  const result = profiler.measure("render_workspace", { windows: 2 }, () => {
    now = 28;
    return "ok";
  });
  const payload = profiler.stop();

  assert.equal(result, "ok");
  assert.equal(payload.entries[1].kind, "render_workspace");
  assert.equal(payload.entries[1].duration_ms, 8);
  assert.equal(payload.entries[1].windows, 2);
});

test("UI trace profiler restarts capture with a fresh buffer", () => {
  let now = 1;
  const profiler = createUiTraceProfiler({
    now: () => now,
    sessionId: () => `trace-${now}`,
  });

  profiler.start();
  profiler.record("old_event");
  now = 2;
  profiler.start();
  profiler.record("new_event");

  const payload = profiler.stop();
  assert.equal(payload.session_id, "trace-2");
  assert.deepEqual(
    payload.entries.map((entry) => entry.kind),
    ["trace_start", "new_event"],
  );
});

test("UI trace profiler does not inspect pointer events while inactive", () => {
  const profiler = createUiTraceProfiler();
  const event = {};
  for (const key of ["pointerId", "button", "buttons", "clientX", "clientY", "target"]) {
    Object.defineProperty(event, key, {
      get() {
        throw new Error(`${key} should not be read while tracing is inactive`);
      },
    });
  }

  assert.doesNotThrow(() => {
    profiler.recordPointer("pointer_move_ignored", event, {
      gesture: "resize",
    });
  });
});
