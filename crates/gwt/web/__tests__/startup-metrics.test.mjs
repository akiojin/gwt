import assert from "node:assert/strict";
import test from "node:test";
import { createStartupMetrics } from "../startup-metrics.js";

test("first frame is reported once after two animation frames", () => {
  const frames = [];
  const events = [];
  let navigationMs = 10;
  const metrics = createStartupMetrics({
    send: (event) => events.push(event),
    requestAnimationFrame: (callback) => frames.push(callback),
    now: () => navigationMs,
  });
  metrics.onWorkspaceRendered();
  metrics.onWorkspaceRendered();
  assert.equal(frames.length, 1);
  frames.shift()();
  assert.deepEqual(events, []);
  navigationMs = 42;
  frames.shift()();
  assert.deepEqual(events, [{ kind: "startup_first_frame", navigation_ms: 42 }]);
  metrics.onWorkspaceRendered();
  assert.equal(frames.length, 0);
});

test("each terminal handshake is reported even when a window id is reused", () => {
  const events = [];
  const metrics = createStartupMetrics({ send: (event) => events.push(event) });
  metrics.onTerminalReady("preview", { readOnly: true });
  metrics.onTerminalStatus("preview", "running", { isReady: true, readOnly: true });
  assert.deepEqual(events, []);
  metrics.onTerminalReady("tab:window");
  metrics.onTerminalReady("tab:window");
  assert.deepEqual(events, [
    { kind: "startup_terminal_ready", id: "tab:window" },
    { kind: "startup_terminal_ready", id: "tab:window" },
  ]);
});

test("running re-reports readiness once per runtime without consuming unfinished fits", () => {
  const events = [];
  const metrics = createStartupMetrics({ send: (event) => events.push(event) });
  const runtime = { isReady: false };
  metrics.onTerminalStatus("restored", "running", runtime);
  runtime.isReady = true;
  metrics.onTerminalStatus("restored", "stopped", runtime);
  metrics.onTerminalStatus("restored", "idle", runtime);
  assert.deepEqual(events, []);
  metrics.onTerminalStatus("restored", "running", runtime);
  metrics.onTerminalStatus("restored", "running", runtime);
  assert.equal(events.length, 1);
  metrics.onTerminalStatus("restored", "running", { isReady: true });
  assert.deepEqual(events, [
    { kind: "startup_terminal_ready", id: "restored" },
    { kind: "startup_terminal_ready", id: "restored" },
  ]);
});
