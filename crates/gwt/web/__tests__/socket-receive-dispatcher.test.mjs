import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  DEFAULT_COALESCE_KINDS,
  DEFAULT_MAX_STREAMED_BEFORE_STATE,
  coalesceEvents,
  createSocketReceiveDispatcher,
} from "../socket-receive-dispatcher.js";

const here = dirname(fileURLToPath(import.meta.url));
const dispatcherSource = readFileSync(resolve(here, "../socket-receive-dispatcher.js"), "utf8");

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

test("project snapshots stay after the workspace state that activates their project", () => {
  const workspace = {
    kind: "workspace_state",
    workspace: { active_tab_id: "project-b" },
  };
  const candidates = {
    kind: "improvement_candidates",
    project_root: "/projects/b",
    candidates: [{ id: "impr-b" }],
  };
  const otherProjectCandidates = {
    kind: "improvement_candidates",
    project_root: "/projects/a",
    candidates: [{ id: "impr-a" }],
  };

  assert.deepEqual(
    coalesceEvents(
      [workspace, otherProjectCandidates, candidates],
      DEFAULT_COALESCE_KINDS,
    ),
    [workspace, otherProjectCandidates, candidates],
    "project-scoped snapshots must preserve multiplicity without overtaking activation state",
  );
});

test("project action outcomes stay behind the workspace activation fence", () => {
  const workspace = {
    kind: "workspace_state",
    workspace: { active_tab_id: "project-b" },
  };
  const staleError = {
    kind: "improvement_action_error",
    project_root: "/projects/a",
    message: "stale project error",
  };
  const candidates = {
    kind: "improvement_candidates",
    project_root: "/projects/b",
    candidates: [{ id: "impr-b" }],
  };

  assert.deepEqual(
    coalesceEvents([workspace, staleError, candidates], DEFAULT_COALESCE_KINDS),
    [workspace, staleError, candidates],
    "the active-project fence must run before scoped action outcomes",
  );
});

test("latest workspace activation stays ahead of outcomes across rapid project switches", () => {
  const workspaceB = {
    kind: "workspace_state",
    workspace: { active_tab_id: "project-b" },
  };
  const staleError = {
    kind: "improvement_action_error",
    project_root: "/projects/a",
    message: "stale project error",
  };
  const workspaceC = {
    kind: "workspace_state",
    workspace: { active_tab_id: "project-c" },
  };
  const candidatesC = {
    kind: "improvement_candidates",
    project_root: "/projects/c",
    candidates: [{ id: "impr-c" }],
  };

  assert.deepEqual(
    coalesceEvents(
      [workspaceB, staleError, workspaceC, candidatesC],
      DEFAULT_COALESCE_KINDS,
    ),
    [workspaceC, staleError, candidatesC],
    "the surviving activation must update the frontend fence before stale outcomes",
  );
});

test("default coalescing policy mirrors backend latest-wins event policy", () => {
  for (const kind of [
    "workspace_state",
    "active_work_projection",
    "window_list",
    "runtime_health",
    "project_index_status",
    "launch_wizard_state",
    "update_state",
  ]) {
    assert.equal(
      DEFAULT_COALESCE_KINDS.has(kind),
      true,
      `${kind} should collapse to the latest event`,
    );
  }

  for (const kind of [
    "terminal_output",
    "terminal_snapshot",
    "runtime_hook_event",
    "improvement_candidates",
  ]) {
    assert.equal(
      DEFAULT_COALESCE_KINDS.has(kind),
      false,
      `${kind} must preserve delivery semantics`,
    );
  }
});

test("runtime_health collapses to the latest snapshot", () => {
  const queue = [
    { kind: "runtime_health", snapshot: { cpu_percent: 10 } },
    { kind: "terminal_output", id: "shell", data: "echo" },
    { kind: "runtime_health", snapshot: { cpu_percent: 42 } },
  ];

  const coalesced = coalesceEvents(queue, DEFAULT_COALESCE_KINDS);
  assert.deepEqual(coalesced, [
    { kind: "terminal_output", id: "shell", data: "echo" },
    { kind: "runtime_health", snapshot: { cpu_percent: 42 } },
  ]);
});

test("launch_wizard_state null tombstone wins during coalescing", () => {
  const coalesced = coalesceEvents(
    [
      { kind: "launch_wizard_state", wizard: { id: "wizard-1" } },
      { kind: "terminal_output", id: "agent", data: "ready" },
      { kind: "launch_wizard_state", wizard: null },
    ],
    DEFAULT_COALESCE_KINDS,
  );

  assert.deepEqual(coalesced, [
    { kind: "terminal_output", id: "agent", data: "ready" },
    { kind: "launch_wizard_state", wizard: null },
  ]);
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

test("handle() defers string WebSocket JSON.parse until scheduled flush", () => {
  const received = [];
  const scheduler = manualScheduler();
  const originalParse = JSON.parse;
  let parseCalls = 0;
  const dispatcher = createSocketReceiveDispatcher({
    receive: (event) => received.push(event),
    schedule: scheduler.schedule,
    now: () => 0,
  });

  JSON.parse = (source, reviver) => {
    parseCalls += 1;
    return originalParse.call(JSON, source, reviver);
  };
  try {
    dispatcher.handle({
      data: JSON.stringify({ kind: "terminal_output", id: "shell", data: "0" }),
    });

    assert.equal(parseCalls, 0, "string handle() must not parse synchronously");
    assert.equal(received.length, 0, "receive remains deferred until flush");
    assert.equal(scheduler.pendingCount(), 1, "one frame is scheduled");

    scheduler.runOnce();

    assert.equal(parseCalls, 1, "flush parses the raw payload before receive()");
    assert.deepEqual(received, [
      { kind: "terminal_output", id: "shell", data: "0" },
    ]);
  } finally {
    JSON.parse = originalParse;
  }
});

test("string idempotent events coalesce before full JSON.parse", () => {
  const received = [];
  const scheduler = manualScheduler();
  const originalParse = JSON.parse;
  let parseCalls = 0;
  const dispatcher = createSocketReceiveDispatcher({
    receive: (event) => received.push(event),
    schedule: scheduler.schedule,
    now: () => 0,
  });

  JSON.parse = (source, reviver) => {
    parseCalls += 1;
    return originalParse.call(JSON, source, reviver);
  };
  try {
    for (let i = 0; i < 25; i += 1) {
      dispatcher.handle({
        data: JSON.stringify({ kind: "workspace_state", revision: i }),
      });
    }

    assert.equal(parseCalls, 0, "queued raw strings must not parse during handle()");
    scheduler.runOnce();

    assert.equal(parseCalls, 1, "only the latest coalesced state is parsed");
    assert.deepEqual(received, [{ kind: "workspace_state", revision: 24 }]);
  } finally {
    JSON.parse = originalParse;
  }
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

test("idempotent state is delivered after a bounded streamed chunk during heavy backlog", () => {
  const queue = [];
  for (let i = 0; i < 500; i += 1) {
    queue.push({ kind: "terminal_output", id: "shell", data: String(i) });
  }
  queue.push({ kind: "workspace_state", revision: 7 });

  const coalesced = coalesceEvents(queue, DEFAULT_COALESCE_KINDS, {
    maxStreamedBeforeState: DEFAULT_MAX_STREAMED_BEFORE_STATE,
  });
  const stateIndex = coalesced.findIndex(
    (event) => event.kind === "workspace_state",
  );
  const streamed = coalesced.filter((event) => event.kind === "terminal_output");

  assert.equal(stateIndex, DEFAULT_MAX_STREAMED_BEFORE_STATE);
  assert.ok(500 / stateIndex >= 10);
  assert.equal(streamed.length, 500);
  assert.equal(streamed[0].data, "0");
  assert.equal(streamed[499].data, "499");
});

test("small streamed bursts still flush before idempotent state", () => {
  const queue = [];
  for (let i = 0; i < 4; i += 1) {
    queue.push({ kind: "terminal_output", id: "shell", data: String(i) });
  }
  queue.push({ kind: "workspace_state", revision: 1 });

  const coalesced = coalesceEvents(queue, DEFAULT_COALESCE_KINDS, {
    maxStreamedBeforeState: DEFAULT_MAX_STREAMED_BEFORE_STATE,
  });

  assert.deepEqual(
    coalesced.map((event) => event.kind),
    [
      "terminal_output",
      "terminal_output",
      "terminal_output",
      "terminal_output",
      "workspace_state",
    ],
  );
});

test("dispatcher threads streamed chunk budget into receive order", () => {
  const received = [];
  const scheduler = manualScheduler();
  const dispatcher = createSocketReceiveDispatcher({
    receive: (event) => received.push(event),
    schedule: scheduler.schedule,
    now: () => 0,
    maxStreamedBeforeState: 2,
  });

  for (let i = 0; i < 5; i += 1) {
    dispatcher.enqueue({
      kind: "terminal_output",
      id: "shell",
      data: String(i),
    });
  }
  dispatcher.enqueue({ kind: "workspace_state", revision: 1 });
  scheduler.runOnce();

  assert.deepEqual(
    received.map((event) => `${event.kind}:${event.data ?? event.revision}`),
    [
      "terminal_output:0",
      "terminal_output:1",
      "workspace_state:1",
      "terminal_output:2",
      "terminal_output:3",
      "terminal_output:4",
    ],
  );
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

test("dispatcher emits sanitized trace metadata for parse and receive timing", () => {
  const traces = [];
  const scheduler = manualScheduler();
  let virtualClock = 0;
  const dispatcher = createSocketReceiveDispatcher({
    receive: (event) => {
      assert.equal(event.data, "must-not-leak");
      virtualClock += 4;
    },
    schedule: scheduler.schedule,
    now: () => virtualClock,
    onTrace: (kind, fields) => traces.push({ kind, ...fields }),
  });

  dispatcher.handle({
    data: JSON.stringify({ kind: "terminal_output", data: "must-not-leak" }),
  });
  scheduler.runOnce();

  assert.deepEqual(
    traces.map((trace) => trace.kind),
    [
      "ws_message",
      "ws_flush_start",
      "terminal_output_ws_receive",
      "ws_receive",
      "ws_flush_end",
    ],
  );
  assert.equal(traces[0].event_kind, "terminal_output");
  const receiveTrace = traces.find((trace) => trace.kind === "ws_receive");
  assert.equal(receiveTrace.event_kind, "terminal_output");
  assert.equal(receiveTrace.duration_ms, 4);
  assert.equal(JSON.stringify(traces).includes("must-not-leak"), false);
});

test("dispatcher builds trace metadata lazily", () => {
  assert.match(dispatcherSource, /function\s+trace\(\s*kind,\s*fieldsFactory/);
  assert.doesNotMatch(
    dispatcherSource,
    /trace\(\s*["']ws_[^"']+["']\s*,\s*\{/,
    "trace call sites must pass factories so inactive tracing skips field allocation",
  );
});

test("dispatcher skips trace callbacks while shouldTrace is false", () => {
  const traces = [];
  const received = [];
  const scheduler = manualScheduler();
  let shouldTrace = false;
  const dispatcher = createSocketReceiveDispatcher({
    receive: (event) => received.push(event),
    schedule: scheduler.schedule,
    now: () => 0,
    onTrace: (kind, fields) => traces.push({ kind, ...fields }),
    shouldTrace: () => shouldTrace,
  });

  for (let i = 0; i < 25; i += 1) {
    dispatcher.handle({
      data: JSON.stringify({ kind: "terminal_output", id: "shell", data: String(i) }),
    });
  }
  scheduler.runOnce();

  assert.equal(received.length, 25);
  assert.deepEqual(traces, [], "inactive tracing must not call onTrace");

  shouldTrace = true;
  dispatcher.handle({
    data: JSON.stringify({ kind: "terminal_output", id: "shell", data: "active" }),
  });
  scheduler.runOnce();

  assert.deepEqual(
    traces.map((trace) => trace.kind),
    [
      "ws_message",
      "ws_flush_start",
      "terminal_output_ws_receive",
      "ws_receive",
      "ws_flush_end",
    ],
    "trace events must resume once shouldTrace returns true",
  );
});

test("terminal output receives a browser-local sequence through internal metadata", () => {
  const deliveries = [];
  const traces = [];
  const scheduler = manualScheduler();
  const dispatcher = createSocketReceiveDispatcher({
    receive: (...args) => deliveries.push(args),
    schedule: scheduler.schedule,
    now: () => 0,
    onTrace: (kind, fields) => traces.push({ kind, fields }),
  });
  const parsed = {
    kind: "terminal_output",
    id: "agent-a",
    data_base64: "c2VjcmV0LWE=",
  };
  const raw = {
    kind: "terminal_output",
    id: "agent-b",
    data_base64: "c2VjcmV0LWI=",
  };

  dispatcher.enqueue(parsed);
  dispatcher.handle({ data: JSON.stringify(raw) });
  scheduler.runOnce();

  assert.deepEqual(
    deliveries.map(([event]) => event),
    [parsed, raw],
    "terminal output wire payloads must reach receive unchanged",
  );
  assert.equal(Object.hasOwn(parsed, "sequence"), false);
  assert.equal(Object.hasOwn(raw, "sequence"), false);
  assert.deepEqual(
    deliveries.map(([, metadata]) => metadata),
    [
      { sequence: 1, window_id: "agent-a" },
      { sequence: 2, window_id: "agent-b" },
    ],
  );
  assert.ok(
    deliveries.every(([, metadata]) => Object.isFrozen(metadata)),
    "internal trace metadata must be immutable",
  );
  assert.deepEqual(
    traces
      .filter(({ kind }) => kind === "terminal_output_ws_receive")
      .map(({ fields }) => fields),
    [
      { sequence: 1, window_id: "agent-a" },
      { sequence: 2, window_id: "agent-b" },
    ],
  );
});

test("terminal output keeps the opaque trace epoch only in internal metadata", () => {
  const deliveries = [];
  const traces = [];
  const scheduler = manualScheduler();
  const epoch = Symbol("trace-a");
  const dispatcher = createSocketReceiveDispatcher({
    receive: (...args) => deliveries.push(args),
    schedule: scheduler.schedule,
    now: () => 0,
    onTrace: (kind, fields) => traces.push({ kind, fields }),
    readTraceEpoch: () => epoch,
  });

  dispatcher.enqueue({
    kind: "terminal_output",
    id: "agent-epoch",
    data_base64: "c2VjcmV0",
  });
  scheduler.runOnce();

  assert.deepEqual(deliveries[0][1], {
    sequence: 1,
    window_id: "agent-epoch",
    epoch,
  });
  const marker = traces.find(
    ({ kind }) => kind === "terminal_output_ws_receive",
  );
  assert.deepEqual(marker.fields, {
    sequence: 1,
    window_id: "agent-epoch",
  });
  assert.notEqual(
    marker.fields,
    deliveries[0][1],
    "onTrace must receive a fresh exact-field projection, not internal metadata",
  );
  assert.equal(
    Object.hasOwn(marker.fields, "epoch"),
    false,
    "opaque epoch must never cross the onTrace privacy boundary",
  );
});

test("terminal output receive markers use an exact privacy allowlist", () => {
  const traces = [];
  const scheduler = manualScheduler();
  const dispatcher = createSocketReceiveDispatcher({
    receive: () => {},
    schedule: scheduler.schedule,
    now: () => 0,
    onTrace: (kind, fields) => traces.push({ kind, fields }),
  });

  dispatcher.enqueue({
    kind: "terminal_output",
    id: "agent-private",
    data_base64: "sentinel-base64",
    typed_text: "sentinel-typed-text",
    output_bytes: "sentinel-output-bytes",
    credential: "sentinel-credential",
    env_secret: "sentinel-env-secret",
  });
  scheduler.runOnce();

  const marker = traces.find(
    ({ kind }) => kind === "terminal_output_ws_receive",
  );
  assert.ok(marker, "terminal output must emit the receive stage marker");
  assert.deepEqual(Object.keys(marker.fields).sort(), ["sequence", "window_id"]);
  const serialized = JSON.stringify(marker);
  for (const sentinel of [
    "sentinel-base64",
    "sentinel-typed-text",
    "sentinel-output-bytes",
    "sentinel-credential",
    "sentinel-env-secret",
  ]) {
    assert.equal(serialized.includes(sentinel), false, `${sentinel} must not leak`);
  }
});

test("disabled tracing avoids terminal metadata and sequence allocation", () => {
  const deliveries = [];
  const traces = [];
  const scheduler = manualScheduler();
  let tracing = false;
  let allocatorCalls = 0;
  let epochReads = 0;
  const epoch = Symbol("active-trace");
  const dispatcher = createSocketReceiveDispatcher({
    receive: (...args) => deliveries.push(args),
    schedule: scheduler.schedule,
    now: () => 0,
    onTrace: (kind, fields) => traces.push({ kind, fields }),
    shouldTrace: () => tracing,
    readTraceEpoch: () => {
      epochReads += 1;
      return epoch;
    },
    nextTerminalOutputSequence: () => {
      allocatorCalls += 1;
      return allocatorCalls;
    },
  });

  dispatcher.enqueue({
    kind: "terminal_output",
    id: "inactive",
    data_base64: "aW5hY3RpdmU=",
  });
  scheduler.runOnce();

  assert.equal(deliveries[0].length, 1, "inactive receive keeps the legacy one-argument API");
  assert.deepEqual(traces, []);
  assert.equal(allocatorCalls, 0, "inactive tracing must not call the sequence allocator");
  assert.equal(epochReads, 0, "inactive tracing must not read the trace epoch");

  tracing = true;
  dispatcher.enqueue({
    kind: "terminal_output",
    id: "active",
    data_base64: "YWN0aXZl",
  });
  scheduler.runOnce();

  assert.deepEqual(deliveries[1][1], {
    sequence: 1,
    window_id: "active",
    epoch,
  });
  assert.equal(allocatorCalls, 1);
  assert.equal(epochReads, 1);
});

test("recreated dispatchers preserve output sequence with a shared allocator", () => {
  const deliveries = [];
  let browserSequence = 0;
  const nextTerminalOutputSequence = () => {
    browserSequence += 1;
    return browserSequence;
  };

  for (let connection = 0; connection < 2; connection += 1) {
    const scheduler = manualScheduler();
    const dispatcher = createSocketReceiveDispatcher({
      receive: (_event, metadata) => deliveries.push(metadata),
      schedule: scheduler.schedule,
      now: () => 0,
      onTrace: () => {},
      nextTerminalOutputSequence,
    });
    dispatcher.enqueue({
      kind: "terminal_output",
      id: "same-window",
      data_base64: `connection-${connection}`,
    });
    scheduler.runOnce();
  }

  assert.deepEqual(deliveries, [
    { sequence: 1, window_id: "same-window" },
    { sequence: 2, window_id: "same-window" },
  ]);
});

test("generation advance drops stale dispatcher work before tracing or allocation", () => {
  const oldScheduler = manualScheduler();
  const newScheduler = manualScheduler();
  const deliveries = [];
  const traces = [];
  const epoch = Symbol("current-trace");
  let generation = 1;
  let sequence = 0;
  let allocatorCalls = 0;
  let epochReads = 0;

  function dispatcherFor(ownGeneration, scheduler) {
    return createSocketReceiveDispatcher({
      receive: (event, metadata) => {
        if (ownGeneration !== generation) {
          return;
        }
        deliveries.push({ event, metadata });
      },
      schedule: scheduler.schedule,
      now: () => 0,
      onTrace: (kind, fields) => traces.push({ kind, fields }),
      shouldTrace: () => ownGeneration === generation,
      readTraceEpoch: () => {
        epochReads += 1;
        return epoch;
      },
      nextTerminalOutputSequence: () => {
        allocatorCalls += 1;
        sequence += 1;
        return sequence;
      },
    });
  }

  const oldDispatcher = dispatcherFor(generation, oldScheduler);
  oldDispatcher.enqueue({
    kind: "terminal_output",
    id: "stale-window",
    data_base64: "b2xk",
  });

  generation += 1;
  const newDispatcher = dispatcherFor(generation, newScheduler);
  oldScheduler.runOnce();

  assert.deepEqual(deliveries, []);
  assert.deepEqual(traces, []);
  assert.equal(allocatorCalls, 0, "stale work must not consume a sequence");
  assert.equal(epochReads, 0, "stale work must not inspect the active epoch");

  newDispatcher.enqueue({
    kind: "terminal_output",
    id: "current-window",
    data_base64: "bmV3",
  });
  newScheduler.runOnce();

  assert.equal(allocatorCalls, 1);
  assert.equal(epochReads, 1);
  assert.equal(deliveries.length, 1);
  assert.deepEqual(deliveries[0].metadata, {
    sequence: 1,
    window_id: "current-window",
    epoch,
  });
  assert.deepEqual(
    traces
      .filter(({ kind }) => kind === "terminal_output_ws_receive")
      .map(({ fields }) => fields),
    [{ sequence: 1, window_id: "current-window" }],
  );
});

for (const [failure, nextTerminalOutputSequence] of [
  [
    "throws",
    () => {
      throw new Error("allocator unavailable");
    },
  ],
  ["returns undefined", () => undefined],
  ["returns null", () => null],
  ["returns NaN", () => Number.NaN],
  ["returns Infinity", () => Number.POSITIVE_INFINITY],
  ["returns an empty string", () => ""],
]) {
  test(`sequence allocator that ${failure} leaves terminal delivery operational`, () => {
    const deliveries = [];
    const traces = [];
    const scheduler = manualScheduler();
    const dispatcher = createSocketReceiveDispatcher({
      receive: (...args) => deliveries.push(args),
      schedule: scheduler.schedule,
      now: () => 0,
      onTrace: (kind, fields) => traces.push({ kind, fields }),
      nextTerminalOutputSequence,
    });

    dispatcher.enqueue({
      kind: "terminal_output",
      id: "agent-fail-open",
      data_base64: "b2s=",
    });

    assert.doesNotThrow(() => scheduler.runOnce());
    assert.equal(deliveries.length, 1);
    assert.equal(
      deliveries[0].length,
      1,
      "failed diagnostics keep the legacy receive call",
    );
    assert.equal(
      traces.some(({ kind }) => kind === "terminal_output_ws_receive"),
      false,
      "a failed allocator must not emit an ambiguous marker",
    );
  });
}

test("terminal receive trace failures never interrupt delivery", () => {
  const received = [];
  const scheduler = manualScheduler();
  const dispatcher = createSocketReceiveDispatcher({
    receive: (event, metadata) => received.push({ event, metadata }),
    schedule: scheduler.schedule,
    now: () => 0,
    onTrace: () => {
      throw new Error("diagnostics failed");
    },
  });

  dispatcher.enqueue({
    kind: "terminal_output",
    id: "agent-safe",
    data_base64: "b2s=",
  });

  assert.doesNotThrow(() => scheduler.runOnce());
  assert.equal(received.length, 1);
  assert.deepEqual(received[0].metadata, {
    sequence: 1,
    window_id: "agent-safe",
  });
});
