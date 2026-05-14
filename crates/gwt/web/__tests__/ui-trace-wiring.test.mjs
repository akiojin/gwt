import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  UI_TRACE_EVENT,
  createUiTraceSavePayload,
  createUiTraceWiring,
} from "../ui-trace-wiring.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

function fakeProfiler() {
  const calls = [];
  return {
    calls,
    measure(kind, fields, callback) {
      calls.push({ kind: "measure", event: kind, fields });
      return callback();
    },
    record(kind, fields) {
      calls.push({ kind: "record", event: kind, fields });
    },
    recordPointer(kind, event, fields) {
      calls.push({ kind: "recordPointer", event: kind, pointerId: event.pointerId, fields });
    },
    start() {
      calls.push({ kind: "start" });
      return { session_id: "trace-1", started_at: 1 };
    },
    stop() {
      calls.push({ kind: "stop" });
      return { session_id: "trace-1", entries: [] };
    },
  };
}

test("UI trace wiring registers command palette actions and saves stopped traces", () => {
  const profiler = fakeProfiler();
  const sent = [];
  const registered = [];
  const logs = [];
  const wiring = createUiTraceWiring({
    profiler,
    send: (message) => sent.push(message),
    alert: () => {
      throw new Error("alert should not be called for an active trace");
    },
    log: (message) => logs.push(message),
  });

  assert.equal(
    wiring.registerPalette({ register: (entry) => registered.push(entry) }),
    true,
  );
  assert.deepEqual(
    registered.map((entry) => [entry.id, entry.label, entry.group]),
    [
      ["diagnostics-ui-trace-start", "Start UI Trace", "Diagnostics"],
      ["diagnostics-ui-trace-stop", "Stop UI Trace", "Diagnostics"],
    ],
  );

  registered[0].handler();
  registered[1].handler();

  assert.deepEqual(sent, [
    {
      kind: "save_ui_trace",
      trace: { session_id: "trace-1", entries: [] },
    },
  ]);
  assert.deepEqual(logs, ["[ui-trace] started trace-1"]);
});

test("UI trace wiring reports stop without an active trace", () => {
  const alerts = [];
  const wiring = createUiTraceWiring({
    profiler: {
      record() {},
      recordPointer() {},
      measure(_kind, _fields, callback) {
        return callback();
      },
      start() {
        return { session_id: "trace-1" };
      },
      stop() {
        return null;
      },
    },
    send: () => {
      throw new Error("send should not run without a trace payload");
    },
    alert: (message) => alerts.push(message),
    log: () => {},
  });

  assert.equal(wiring.stop(), null);
  assert.deepEqual(alerts, ["UI trace is not running."]);
});

test("UI trace wiring exposes recorder helpers and stable event constants", () => {
  const profiler = fakeProfiler();
  const wiring = createUiTraceWiring({
    profiler,
    send: () => {},
    alert: () => {},
    log: () => {},
  });

  wiring.traceUi(UI_TRACE_EVENT.applyViewport, { zoom: 1 });
  wiring.tracePointer(UI_TRACE_EVENT.pointerPanMove, { pointerId: 7 }, { gesture: "pan" });
  const measured = wiring.traceMeasure(UI_TRACE_EVENT.renderWorkspace, { windows: 2 }, () => "ok");

  assert.equal(measured, "ok");
  assert.deepEqual(profiler.calls.slice(-3), [
    { kind: "record", event: "apply_viewport", fields: { zoom: 1 } },
    {
      kind: "recordPointer",
      event: "pointer_pan_move",
      pointerId: 7,
      fields: { gesture: "pan" },
    },
    {
      kind: "measure",
      event: "render_workspace",
      fields: { windows: 2 },
    },
  ]);
});

test("UI trace save payload keeps the wire kind in one place", () => {
  const trace = { session_id: "trace-1", entries: [] };
  assert.deepEqual(createUiTraceSavePayload(trace), {
    kind: "save_ui_trace",
    trace,
  });
});

test("app.js imports and instantiates the UI trace wiring module", () => {
  assert.match(
    appSource,
    /import\s*\{\s*UI_TRACE_EVENT,\s*createUiTraceWiring\s*\}\s*from\s*["']\/ui-trace-wiring\.js["']/,
  );
  assert.match(appSource, /uiTraceWiring\s*=\s*createUiTraceWiring\(/);
});

test("WebSocket dispatcher forwards timing trace events through the wiring facade", () => {
  assert.match(appSource, /onTrace:\s*\(kind,\s*fields\)\s*=>/);
  assert.match(appSource, /traceUi\(kind,\s*fields\)/);
});

test("pointer diagnostics use centralized event constants", () => {
  assert.match(appSource, /UI_TRACE_EVENT\.pointerPanMove/);
  assert.match(appSource, /UI_TRACE_EVENT\.pointerMoveIgnored/);
  assert.match(appSource, /UI_TRACE_EVENT\.resizePointermoveApply/);
});

test("backend UI trace save result is surfaced to the user", () => {
  assert.match(appSource, /case\s+"ui_trace_saved"/);
  assert.match(appSource, /case\s+"ui_trace_error"/);
});
