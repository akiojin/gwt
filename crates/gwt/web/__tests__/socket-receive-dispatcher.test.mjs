import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_COALESCE_KINDS,
  coalesceEvents,
  createSocketReceiveDispatcher,
} from "../socket-receive-dispatcher.js";

function manualScheduler() {
  const pending = [];
  return {
    schedule: (cb) => {
      pending.push(cb);
    },
    runOnce() {
      const cb = pending.shift();
      if (cb) cb();
    },
    runAll() {
      while (pending.length > 0) {
        const cb = pending.shift();
        cb();
      }
    },
    pendingCount() {
      return pending.length;
    },
  };
}

test("idempotent kinds collapse to the most recent occurrence", () => {
  const queue = [
    { kind: "workspace_state", n: 1 },
    { kind: "terminal_output", id: "a", data: "x" },
    { kind: "workspace_state", n: 2 },
    { kind: "active_work_projection", v: 1 },
    { kind: "workspace_state", n: 3 },
    { kind: "active_work_projection", v: 2 },
  ];

  const coalesced = coalesceEvents(queue, DEFAULT_COALESCE_KINDS);
  assert.deepEqual(coalesced, [
    { kind: "terminal_output", id: "a", data: "x" },
    { kind: "workspace_state", n: 3 },
    { kind: "active_work_projection", v: 2 },
  ]);
});

test("non-coalesce kinds preserve order and multiplicity", () => {
  const queue = [
    { kind: "terminal_output", data: "a" },
    { kind: "terminal_output", data: "b" },
    { kind: "notification", id: 1 },
    { kind: "terminal_output", data: "c" },
    { kind: "notification", id: 2 },
  ];

  const coalesced = coalesceEvents(queue, DEFAULT_COALESCE_KINDS);
  assert.deepEqual(coalesced, queue);
});

test("dispatcher flushes once per frame and renders only the latest workspace_state", () => {
  const received = [];
  const scheduler = manualScheduler();
  const dispatcher = createSocketReceiveDispatcher({
    receive: (event) => received.push(event),
    schedule: scheduler.schedule,
    now: () => 0,
  });

  for (let i = 0; i < 100; i += 1) {
    dispatcher.enqueue({ kind: "workspace_state", revision: i });
  }
  assert.equal(received.length, 0, "no synchronous receive() calls");
  assert.equal(scheduler.pendingCount(), 1, "one frame scheduled");

  scheduler.runOnce();
  assert.equal(received.length, 1, "only the latest workspace_state reaches receive()");
  assert.deepEqual(received[0], { kind: "workspace_state", revision: 99 });
});

test("ordered terminal_output stream is delivered in arrival order", () => {
  const received = [];
  const scheduler = manualScheduler();
  const dispatcher = createSocketReceiveDispatcher({
    receive: (event) => received.push(event),
    schedule: scheduler.schedule,
    now: () => 0,
  });

  for (let i = 0; i < 50; i += 1) {
    dispatcher.enqueue({ kind: "terminal_output", id: "shell", data: String(i) });
  }
  scheduler.runOnce();

  assert.equal(received.length, 50, "every terminal_output reaches receive()");
  assert.equal(received[0].data, "0");
  assert.equal(received[49].data, "49");
});

test("budget overflow defers remaining events to the next frame", () => {
  const received = [];
  const scheduler = manualScheduler();
  let virtualClock = 0;
  const dispatcher = createSocketReceiveDispatcher({
    receive: () => {
      virtualClock += 5; // simulate 5ms per receive
    },
    schedule: scheduler.schedule,
    now: () => virtualClock,
    budgetMs: 8,
  });
  // Wrap receive so we can also record what was processed.
  const inner = dispatcher;
  inner.enqueue({ kind: "terminal_output", data: "a" });
  inner.enqueue({ kind: "terminal_output", data: "b" });
  inner.enqueue({ kind: "terminal_output", data: "c" });

  scheduler.runOnce();
  // After 2 events, virtualClock = 10ms > 8ms budget, so 3rd event is deferred.
  assert.equal(scheduler.pendingCount(), 1, "remaining events scheduled for next frame");
  scheduler.runOnce();
  assert.equal(scheduler.pendingCount(), 0, "all events drained across two frames");
});

test("handle() accepts both WebSocket message events and pre-parsed payloads", () => {
  const received = [];
  const scheduler = manualScheduler();
  const dispatcher = createSocketReceiveDispatcher({
    receive: (event) => received.push(event),
    schedule: scheduler.schedule,
    now: () => 0,
  });

  dispatcher.handle({ data: JSON.stringify({ kind: "notification", id: 1 }) });
  dispatcher.handle({ kind: "notification", id: 2 });
  scheduler.runOnce();

  assert.deepEqual(received, [
    { kind: "notification", id: 1 },
    { kind: "notification", id: 2 },
  ]);
});

test("flushNow synchronously drains pending events without waiting for the scheduler", () => {
  const received = [];
  const scheduler = manualScheduler();
  const dispatcher = createSocketReceiveDispatcher({
    receive: (event) => received.push(event),
    schedule: scheduler.schedule,
    now: () => 0,
  });

  dispatcher.enqueue({ kind: "workspace_state", revision: 1 });
  dispatcher.enqueue({ kind: "workspace_state", revision: 2 });
  dispatcher.flushNow();

  assert.equal(received.length, 1);
  assert.equal(received[0].revision, 2);
});

// Issue #2698 PR 3 — terminal_output (streamed) flushes ahead of
// idempotent kinds within the same rAF tick. Backend state
// broadcasts (workspace_state, etc.) can pile up while the user is
// typing; without prioritization, terminal echo waits behind a
// heavy renderWorkspaceState call and the user feels keystroke lag.

test("streamed events flush before idempotent kinds even when idempotent arrived first", () => {
  const queue = [
    { kind: "workspace_state", n: 1 },
    { kind: "workspace_state", n: 2 },
    { kind: "terminal_output", id: "shell", data: "echo" },
    { kind: "active_work_projection", v: 1 },
  ];

  const coalesced = coalesceEvents(queue, DEFAULT_COALESCE_KINDS);
  assert.deepEqual(coalesced, [
    { kind: "terminal_output", id: "shell", data: "echo" },
    { kind: "workspace_state", n: 2 },
    { kind: "active_work_projection", v: 1 },
  ]);
});

test("multiple streamed events maintain relative order, then idempotent follows", () => {
  const queue = [
    { kind: "workspace_state", n: 1 },
    { kind: "terminal_output", id: "a", data: "x" },
    { kind: "notification", id: 1 },
    { kind: "workspace_state", n: 2 },
    { kind: "terminal_output", id: "a", data: "y" },
    { kind: "active_work_projection", v: 5 },
  ];

  const coalesced = coalesceEvents(queue, DEFAULT_COALESCE_KINDS);
  assert.deepEqual(coalesced, [
    { kind: "terminal_output", id: "a", data: "x" },
    { kind: "notification", id: 1 },
    { kind: "terminal_output", id: "a", data: "y" },
    { kind: "workspace_state", n: 2 },
    { kind: "active_work_projection", v: 5 },
  ]);
});

test("dispatcher delivers terminal_output ahead of pending workspace_state in the same flush", () => {
  const received = [];
  const scheduler = manualScheduler();
  const dispatcher = createSocketReceiveDispatcher({
    receive: (event) => received.push(event),
    schedule: scheduler.schedule,
    now: () => 0,
  });

  // 20 idempotent state updates piled up before a single terminal echo.
  for (let i = 0; i < 20; i += 1) {
    dispatcher.enqueue({ kind: "workspace_state", revision: i });
  }
  dispatcher.enqueue({ kind: "terminal_output", id: "shell", data: "a" });
  scheduler.runOnce();

  assert.equal(received.length, 2, "coalesced workspace_state + 1 terminal_output");
  assert.equal(received[0].kind, "terminal_output", "echo lands first");
  assert.equal(received[1].kind, "workspace_state", "state update follows");
});
