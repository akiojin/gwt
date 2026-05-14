import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

test("app.js imports and instantiates the UI trace profiler", () => {
  assert.match(
    appSource,
    /import\s*\{\s*createUiTraceProfiler\s*\}\s*from\s*["']\/ui-trace-profiler\.js["']/,
  );
  assert.match(appSource, /uiTraceProfiler\s*=\s*createUiTraceProfiler\(\)/);
});

test("command palette can start and stop UI trace capture", () => {
  assert.match(appSource, /label:\s*"Start UI Trace"/);
  assert.match(appSource, /label:\s*"Stop UI Trace"/);
  assert.match(appSource, /kind:\s*"save_ui_trace"/);
});

test("WebSocket dispatcher forwards timing trace events", () => {
  assert.match(appSource, /onTrace:\s*\(kind,\s*fields\)\s*=>/);
  assert.match(appSource, /uiTraceProfiler\.record\(kind,\s*fields\)/);
});

test("pointer diagnostics distinguish accepted and ignored movement", () => {
  assert.match(appSource, /pointer_pan_move/);
  assert.match(appSource, /pointer_move_ignored/);
  assert.match(appSource, /resize_pointermove_apply/);
});

test("backend UI trace save result is surfaced to the user", () => {
  assert.match(appSource, /case\s+"ui_trace_saved"/);
  assert.match(appSource, /case\s+"ui_trace_error"/);
});
