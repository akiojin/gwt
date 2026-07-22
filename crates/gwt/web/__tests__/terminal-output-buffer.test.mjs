import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_MAX_CHARS_PER_FLUSH,
  DEFAULT_MAX_TOTAL_CHARS_PER_FLUSH,
  DEFAULT_MAX_WINDOWS_PER_FLUSH,
  createTerminalOutputBatcher,
} from "../terminal-output-buffer.js";

function manualScheduler() {
  const pending = [];
  return {
    schedule: (cb) => {
      pending.push(cb);
      return pending.length;
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

function cursorScheduler() {
  const pending = [];
  let head = 0;
  function compact() {
    if (head > 32 && head * 2 >= pending.length) {
      pending.splice(0, head);
      head = 0;
    }
  }
  return {
    schedule: (cb) => {
      pending.push(cb);
      return pending.length - head;
    },
    runOnce() {
      const cb = pending[head];
      if (cb) {
        head += 1;
        compact();
        cb();
      }
    },
    runAll() {
      while (head < pending.length) {
        const cb = pending[head];
        head += 1;
        compact();
        cb();
      }
    },
    pendingCount() {
      return pending.length - head;
    },
  };
}

function continuouslyPendingFixture({
  windowIds,
  maxCharsPerFlush,
  maxWindowsPerFlush,
  maxTotalCharsPerFlush,
}) {
  const scheduler = manualScheduler();
  const writes = [];
  const firstServicedFrame = new Map();
  let frame = 0;
  let batcher;
  batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxCharsPerFlush,
    maxWindowsPerFlush,
    maxTotalCharsPerFlush,
    write: (windowId, text, done) => {
      writes.push({ frame, windowId, text });
      if (!firstServicedFrame.has(windowId)) {
        firstServicedFrame.set(windowId, frame);
      }
      batcher.enqueue(windowId, ".");
      done();
    },
  });
  for (const windowId of windowIds) {
    batcher.enqueue(windowId, "a");
    batcher.enqueue(windowId, "b");
  }
  return {
    batcher,
    firstServicedFrame,
    runFrame(priorityWindowId) {
      frame += 1;
      if (priorityWindowId !== undefined) {
        batcher.prioritize(priorityWindowId);
      }
      scheduler.runOnce();
    },
    writes,
  };
}

test("same-window terminal chunks batch into one ordered xterm write", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const flushed = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
    onFlush: (windowId) => flushed.push(windowId),
  });

  for (let i = 0; i < 500; i += 1) {
    batcher.enqueue("agent-1", `${i},`);
  }

  assert.equal(writes.length, 0, "enqueue must not call xterm synchronously");
  assert.equal(scheduler.pendingCount(), 1, "one frame handles the burst");

  scheduler.runOnce();

  assert.equal(writes.length, 1, "500 chunks should collapse to one write");
  assert.equal(writes[0].windowId, "agent-1");
  assert.equal(
    writes[0].text,
    Array.from({ length: 500 }, (_, i) => `${i},`).join(""),
  );
  assert.deepEqual(flushed, ["agent-1"]);
});

test("terminal output batching stays isolated per window", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-a", "a1");
  batcher.enqueue("agent-b", "b1");
  batcher.enqueue("agent-a", "a2");
  batcher.enqueue("agent-b", "b2");

  assert.equal(scheduler.pendingCount(), 1, "one shared frame drains all windows");
  scheduler.runAll();

  assert.deepEqual(writes, [
    { windowId: "agent-a", text: "a1a2" },
    { windowId: "agent-b", text: "b1b2" },
  ]);
});

test("multi-window terminal bursts schedule one shared frame", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  for (let i = 0; i < 50; i += 1) {
    const windowId = `agent-${i}`;
    batcher.enqueue(windowId, `${windowId}:a`);
    batcher.enqueue(windowId, `${windowId}:b`);
  }

  assert.equal(
    scheduler.pendingCount(),
    1,
    "one shared frame should drain all pending terminal windows",
  );

  scheduler.runAll();

  assert.equal(writes.length, 50);
  assert.deepEqual(writes.slice(0, 3), [
    { windowId: "agent-0", text: "agent-0:aagent-0:b" },
    { windowId: "agent-1", text: "agent-1:aagent-1:b" },
    { windowId: "agent-2", text: "agent-2:aagent-2:b" },
  ]);
  assert.equal(scheduler.pendingCount(), 0);
});

test("multi-window terminal bursts respect the per-frame window budget", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxWindowsPerFlush: 8,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  for (let i = 0; i < 50; i += 1) {
    batcher.enqueue(`agent-${i}`, `chunk-${i}`);
  }

  assert.equal(scheduler.pendingCount(), 1, "one shared frame is scheduled");

  scheduler.runOnce();

  assert.equal(writes.length, 8, "first frame writes only the configured window budget");
  assert.equal(
    batcher.pendingWindowCount(),
    42,
    "remaining windows stay pending for follow-up frames",
  );
  assert.equal(scheduler.pendingCount(), 1, "remaining windows schedule one follow-up frame");

  scheduler.runAll();

  assert.equal(writes.length, 50);
  assert.deepEqual(writes.slice(0, 3), [
    { windowId: "agent-0", text: "chunk-0" },
    { windowId: "agent-1", text: "chunk-1" },
    { windowId: "agent-2", text: "chunk-2" },
  ]);
  assert.deepEqual(writes.slice(-2), [
    { windowId: "agent-48", text: "chunk-48" },
    { windowId: "agent-49", text: "chunk-49" },
  ]);
  assert.equal(scheduler.pendingCount(), 0);
  assert.equal(batcher.pendingWindowCount(), 0);
});

test("continuously busy windows do not starve a ninth active echo", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const busyWindowIds = Array.from({ length: 8 }, (_, index) => `busy-${index}`);
  let frame = 0;
  let activeEchoFrame = null;
  let batcher;
  batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxCharsPerFlush: 1,
    maxWindowsPerFlush: 8,
    write: (windowId, text, done) => {
      writes.push({ frame, windowId, text });
      if (busyWindowIds.includes(windowId)) {
        batcher.enqueue(windowId, ".");
      } else if (windowId === "active" && activeEchoFrame === null) {
        activeEchoFrame = frame;
      }
      done();
    },
  });

  for (const windowId of busyWindowIds) {
    // Keep each Map entry alive while write() replenishes its tail. This
    // reproduces the production failure where the first eight keys never
    // leave the scheduler and a later active window cannot advance.
    batcher.enqueue(windowId, "a");
    batcher.enqueue(windowId, "b");
  }
  batcher.enqueue("active", "e");

  while (frame < 2) {
    frame += 1;
    scheduler.runOnce();
  }

  assert.ok(
    activeEchoFrame !== null && activeEchoFrame <= 2,
    "the ninth active echo must reach xterm within two frames",
  );
  assert.equal(batcher.pendingCount("active"), 0);
  assert.ok(
    writes.some((entry) => entry.windowId === "busy-7"),
    "the fairness fix must retain service for the existing busy windows",
  );
});

test("continuously pending windows are serviced within the round-robin frame bound", () => {
  const scheduler = manualScheduler();
  const windowIds = Array.from({ length: 17 }, (_, index) => `agent-${index}`);
  const servicedFrame = new Map();
  let frame = 0;
  let batcher;
  batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxCharsPerFlush: 1,
    maxWindowsPerFlush: 4,
    write: (windowId, _text, done) => {
      if (!servicedFrame.has(windowId)) {
        servicedFrame.set(windowId, frame);
      }
      batcher.enqueue(windowId, ".");
      done();
    },
  });

  for (const windowId of windowIds) {
    batcher.enqueue(windowId, "a");
    batcher.enqueue(windowId, "b");
  }

  const frameBound = Math.ceil(windowIds.length / 4);
  while (frame < frameBound) {
    frame += 1;
    scheduler.runOnce();
  }

  assert.equal(
    servicedFrame.size,
    windowIds.length,
    "every continuously pending window must receive service within ceil(N / budget) frames",
  );
  for (const windowId of windowIds) {
    assert.ok(
      servicedFrame.get(windowId) <= frameBound,
      `${windowId} exceeded the round-robin frame bound`,
    );
  }
});

test("input priority is one-shot without resetting the round-robin cursor", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const windowIds = Array.from({ length: 5 }, (_, index) => `agent-${index}`);
  let batcher;
  batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxCharsPerFlush: 1,
    maxWindowsPerFlush: 2,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      batcher.enqueue(windowId, ".");
      done();
    },
  });

  for (const windowId of windowIds.slice(0, 2)) {
    batcher.enqueue(windowId, "a");
    batcher.enqueue(windowId, "b");
  }

  assert.equal(
    typeof batcher.prioritize,
    "function",
    "the batcher must expose the input-priority hint used by app.js",
  );
  batcher.prioritize("agent-2");
  // The hint is armed before the echo arrives, matching terminal input send
  // followed by asynchronous PTY output.
  batcher.enqueue("agent-2", "e");
  batcher.enqueue("agent-2", "f");
  for (const windowId of windowIds.slice(3)) {
    batcher.enqueue(windowId, "a");
    batcher.enqueue(windowId, "b");
  }

  scheduler.runOnce();
  assert.deepEqual(
    writes.slice(0, 2).map(({ windowId }) => windowId),
    ["agent-2", "agent-0"],
    "priority service must preserve the independent normal fairness cursor",
  );

  scheduler.runOnce();
  assert.deepEqual(
    writes.slice(2, 4).map(({ windowId }) => windowId),
    ["agent-1", "agent-2"],
    "normal service must advance the round-robin cursor across frames",
  );
});

test("one-shot priority yields the next single-window frame to a peer", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxCharsPerFlush: 1,
    maxWindowsPerFlush: 1,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("active", "a");
  batcher.enqueue("active", "b");
  batcher.enqueue("peer", "p");
  batcher.enqueue("peer", "q");
  batcher.prioritize("active");

  scheduler.runOnce();
  scheduler.runOnce();

  assert.deepEqual(
    writes.slice(0, 2).map(({ windowId }) => windowId),
    ["active", "peer"],
    "the serviced priority window must remain excluded for one fairness frame without re-arm",
  );
});

test("one-shot priority yields after saturating the aggregate character budget", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxCharsPerFlush: 1,
    maxWindowsPerFlush: 2,
    maxTotalCharsPerFlush: 1,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("active", "a");
  batcher.enqueue("active", "b");
  batcher.enqueue("peer", "p");
  batcher.enqueue("peer", "q");
  batcher.prioritize("active");

  scheduler.runOnce();
  scheduler.runOnce();

  assert.deepEqual(
    writes.slice(0, 2).map(({ windowId }) => windowId),
    ["active", "peer"],
    "aggregate saturation must preserve the serviced priority window across the yield",
  );
});

test("hidden-only peers do not defer a later active echo", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const eligibility = new Map([
    ["active", true],
    ["hidden", false],
  ]);
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxCharsPerFlush: 1,
    maxWindowsPerFlush: 1,
    canWrite: (windowId) => eligibility.get(windowId) !== false,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("active", "a");
  batcher.enqueue("hidden", "h");
  batcher.prioritize("active");
  scheduler.runOnce();
  assert.equal(
    scheduler.pendingCount(),
    0,
    "hidden-only pending output must not schedule a fairness frame",
  );

  batcher.enqueue("active", "b");
  batcher.prioritize("active");
  scheduler.runOnce();

  assert.deepEqual(
    writes.map(({ windowId }) => windowId),
    ["active", "active"],
    "a hidden-only peer must not leave stale defer state for a later echo",
  );
});

test("a different priority target replaces input priority without inheriting defer", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxCharsPerFlush: 1,
    maxWindowsPerFlush: 1,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-a", "a");
  batcher.enqueue("agent-a", "b");
  batcher.enqueue("agent-b", "c");
  batcher.enqueue("agent-b", "d");
  batcher.prioritize("agent-a");
  scheduler.runOnce();
  batcher.prioritize("agent-b");
  scheduler.runOnce();

  assert.deepEqual(
    writes.slice(0, 2).map(({ windowId }) => windowId),
    ["agent-a", "agent-b"],
    "a new target must receive priority while the prior target pays its fairness yield",
  );
});

test("clear removes stale priority defer state", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxCharsPerFlush: 1,
    maxWindowsPerFlush: 1,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-a", "a");
  batcher.enqueue("agent-a", "b");
  batcher.enqueue("agent-b", "c");
  batcher.prioritize("agent-a");
  scheduler.runOnce();
  batcher.clear("agent-a");
  batcher.enqueue("agent-a", "fresh");
  batcher.prioritize("agent-a");
  scheduler.runOnce();

  assert.deepEqual(
    writes.map(({ windowId }) => windowId),
    ["agent-a", "agent-a"],
    "a fresh target generation must not inherit defer state cleared with the old queue",
  );
});

test("flushNow removes stale priority defer state", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxCharsPerFlush: 1,
    maxWindowsPerFlush: 1,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-a", "a");
  batcher.enqueue("agent-a", "b");
  batcher.enqueue("agent-a", "c");
  batcher.enqueue("agent-b", "d");
  batcher.prioritize("agent-a");
  scheduler.runOnce();
  batcher.flushNow("agent-a");
  batcher.enqueue("agent-a", "fresh");
  batcher.prioritize("agent-a");
  scheduler.runOnce();

  assert.deepEqual(
    writes.map(({ windowId }) => windowId),
    ["agent-a", "agent-a", "agent-a", "agent-a"],
    "a fresh echo must not inherit defer state from a synchronously drained queue",
  );
});

test("repeated input priority preserves bounded round-robin service", () => {
  const windowIds = Array.from({ length: 5 }, (_, index) => `agent-${index}`);
  const activeWindowId = "agent-2";
  const maxWindowsPerFlush = 2;
  const fixture = continuouslyPendingFixture({
    windowIds,
    maxCharsPerFlush: 1,
    maxWindowsPerFlush,
  });
  const fairnessFrameBound = Math.ceil(
    (windowIds.length - 1) / (maxWindowsPerFlush - 1),
  );

  for (let frame = 0; frame < fairnessFrameBound; frame += 1) {
    fixture.runFrame(activeWindowId);
  }

  assert.equal(
    fixture.firstServicedFrame.size,
    windowIds.length,
    "re-arming the same active window must not reset the normal fairness cursor",
  );
  const activeFrames = fixture.writes
    .filter(({ windowId }) => windowId === activeWindowId)
    .map(({ frame }) => frame);
  assert.deepEqual(
    activeFrames,
    Array.from({ length: fairnessFrameBound }, (_, index) => index + 1),
    "with a spare frame slot, each re-armed active echo remains prioritized",
  );
});

test("repeated input priority stays fair with default flush budgets", () => {
  const windowIds = Array.from({ length: 10 }, (_, index) => `agent-${index}`);
  const activeWindowId = "agent-5";
  const fixture = continuouslyPendingFixture({ windowIds });
  const fairnessFrameBound = Math.ceil(
    (windowIds.length - 1) / (DEFAULT_MAX_WINDOWS_PER_FLUSH - 1),
  );

  for (let frame = 0; frame < fairnessFrameBound; frame += 1) {
    fixture.runFrame(activeWindowId);
  }

  assert.equal(
    fixture.firstServicedFrame.size,
    windowIds.length,
    "default budgets must service every continuously pending window while priority repeats",
  );
});

test("single-window frame budget alternates repeated priority with fairness service", () => {
  const windowIds = Array.from({ length: 4 }, (_, index) => `agent-${index}`);
  const activeWindowId = "agent-1";
  const fixture = continuouslyPendingFixture({
    windowIds,
    maxCharsPerFlush: 1,
    maxWindowsPerFlush: 1,
  });
  const fairnessFrameBound = 2 * (windowIds.length - 1);

  for (let frame = 0; frame < fairnessFrameBound; frame += 1) {
    fixture.runFrame(activeWindowId);
  }

  assert.equal(
    fixture.firstServicedFrame.size,
    windowIds.length,
    "a one-window frame budget must not let repeated priority starve peers",
  );
  const activeFrames = fixture.writes
    .filter(({ windowId }) => windowId === activeWindowId)
    .map(({ frame }) => frame);
  assert.ok(
    activeFrames[0] <= 2,
    "the first active echo must stay within two frames",
  );
  for (let index = 1; index < activeFrames.length; index += 1) {
    assert.ok(
      activeFrames[index] - activeFrames[index - 1] <= 2,
      "repeated active echoes must stay within the two-frame latency bound",
    );
  }
});

test("aggregate-budget saturation alternates repeated priority with fairness service", () => {
  const windowIds = Array.from({ length: 3 }, (_, index) => `agent-${index}`);
  const activeWindowId = "agent-1";
  const fixture = continuouslyPendingFixture({
    windowIds,
    maxCharsPerFlush: 1,
    maxWindowsPerFlush: 2,
    maxTotalCharsPerFlush: 1,
  });
  const fairnessFrameBound = 2 * (windowIds.length - 1);

  for (let frame = 0; frame < fairnessFrameBound; frame += 1) {
    fixture.runFrame(activeWindowId);
  }

  assert.equal(
    fixture.firstServicedFrame.size,
    windowIds.length,
    "priority must yield a frame when it consumes the aggregate budget alone",
  );
});

for (const scenario of [
  {
    name: "single-window frame budget",
    options: {
      maxCharsPerFlush: 16,
      maxWindowsPerFlush: 1,
    },
  },
  {
    name: "aggregate character budget",
    options: {
      maxCharsPerFlush: 1,
      maxWindowsPerFlush: 2,
      maxTotalCharsPerFlush: 1,
    },
  },
]) {
  test(`fresh one-chunk priority cannot starve a peer under ${scenario.name}`, () => {
    const scheduler = manualScheduler();
    const writes = [];
    let frame = 0;
    let batcher;
    batcher = createTerminalOutputBatcher({
      schedule: scheduler.schedule,
      ...scenario.options,
      write: (windowId, text, done) => {
        writes.push({ frame, windowId, text });
        if (windowId === "peer") {
          batcher.enqueue("peer", "p");
        }
        done();
      },
    });

    batcher.enqueue("peer", "p");
    batcher.enqueue("peer", "q");
    for (frame = 1; frame <= 6; frame += 1) {
      batcher.enqueue("active", "a");
      batcher.prioritize("active");
      scheduler.runOnce();
    }

    const peerFrames = writes
      .filter(({ windowId }) => windowId === "peer")
      .map((entry) => entry.frame);
    assert.ok(
      peerFrames.length > 0 && peerFrames[0] <= 2,
      "a continuously pending peer must receive service within two frames",
    );
    const activeFrames = writes
      .filter(({ windowId }) => windowId === "active")
      .map((entry) => entry.frame);
    assert.equal(activeFrames[0], 1);
    for (let index = 1; index < activeFrames.length; index += 1) {
      assert.ok(
        activeFrames[index] - activeFrames[index - 1] <= 2,
        "fresh active echoes must stay within the two-frame latency bound",
      );
    }
  });
}

test("large terminal output queues drain without per-chunk Array.shift", () => {
  const scheduler = cursorScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxCharsPerFlush: 10000,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  const chunks = Array.from({ length: 2000 }, (_, index) => `${index},`);
  for (const chunk of chunks) {
    batcher.enqueue("agent-large", chunk);
  }

  assert.equal(scheduler.pendingCount(), 1);

  const originalShift = Array.prototype.shift;
  let shiftCalls = 0;
  Array.prototype.shift = function patchedShift(...args) {
    shiftCalls += 1;
    return originalShift.apply(this, args);
  };
  try {
    scheduler.runOnce();
  } finally {
    Array.prototype.shift = originalShift;
  }

  assert.equal(
    shiftCalls,
    0,
    "large output backlog drain must not move the array once per chunk",
  );
  assert.deepEqual(writes, [
    { windowId: "agent-large", text: chunks.join("") },
  ]);
  assert.equal(batcher.pendingCount("agent-large"), 0);
  assert.equal(batcher.pendingWindowCount(), 0);
});

test("partial queue drains report only the unconsumed tail", () => {
  const scheduler = cursorScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxCharsPerFlush: 4,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-tail", "aa");
  batcher.enqueue("agent-tail", "bb");
  batcher.enqueue("agent-tail", "cc");
  batcher.enqueue("agent-tail", "dd");

  scheduler.runOnce();

  assert.deepEqual(writes, [{ windowId: "agent-tail", text: "aabb" }]);
  assert.equal(batcher.pendingCount("agent-tail"), 2);
  assert.equal(batcher.pendingCount(), 2);
  assert.equal(batcher.pendingWindowCount(), 1);
  assert.equal(scheduler.pendingCount(), 1);

  scheduler.runOnce();

  assert.deepEqual(writes, [
    { windowId: "agent-tail", text: "aabb" },
    { windowId: "agent-tail", text: "ccdd" },
  ]);
  assert.equal(batcher.pendingCount("agent-tail"), 0);
  assert.equal(batcher.pendingCount(), 0);
  assert.equal(batcher.pendingWindowCount(), 0);
});

test("multi-window terminal bursts respect the aggregate per-frame character budget", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxWindowsPerFlush: 8,
    maxTotalCharsPerFlush: 10,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-0", "aaaaa");
  batcher.enqueue("agent-1", "bbbbb");
  batcher.enqueue("agent-2", "ccccc");
  batcher.enqueue("agent-3", "ddddd");

  scheduler.runOnce();

  assert.deepEqual(writes, [
    { windowId: "agent-0", text: "aaaaa" },
    { windowId: "agent-1", text: "bbbbb" },
  ]);
  assert.equal(batcher.pendingWindowCount(), 2);
  assert.equal(scheduler.pendingCount(), 1, "remaining text schedules one follow-up frame");

  scheduler.runAll();

  assert.deepEqual(writes, [
    { windowId: "agent-0", text: "aaaaa" },
    { windowId: "agent-1", text: "bbbbb" },
    { windowId: "agent-2", text: "ccccc" },
    { windowId: "agent-3", text: "ddddd" },
  ]);
  assert.equal(batcher.pendingWindowCount(), 0);
});

test("aggregate frame budget does not split a single oversized chunk", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxTotalCharsPerFlush: 10,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-big", "xxxxxxxxxxxx");
  batcher.enqueue("agent-small", "ok");
  batcher.enqueue("agent-tail", "zz");

  scheduler.runOnce();

  assert.deepEqual(writes, [
    { windowId: "agent-big", text: "xxxxxxxxxxxx" },
  ]);
  assert.equal(batcher.pendingWindowCount(), 2);
  assert.equal(scheduler.pendingCount(), 1);

  scheduler.runAll();

  assert.deepEqual(writes, [
    { windowId: "agent-big", text: "xxxxxxxxxxxx" },
    { windowId: "agent-small", text: "ok" },
    { windowId: "agent-tail", text: "zz" },
  ]);
});

test("hidden terminal output remains queued without decode write or scheduler spin", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const mergeCalls = [];
  const eligibility = new Map([
    ["agent-hidden", false],
    ["agent-visible", true],
  ]);
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    canWrite: (windowId) => eligibility.get(windowId) !== false,
    mergeChunks: (chunks, windowId) => {
      mergeCalls.push({ windowId, chunks: [...chunks] });
      return chunks.join("");
    },
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-hidden", "hidden-a");
  batcher.enqueue("agent-hidden", "hidden-b");
  batcher.enqueue("agent-visible", "visible");

  scheduler.runOnce();

  assert.deepEqual(mergeCalls, [
    { windowId: "agent-visible", chunks: ["visible"] },
  ]);
  assert.deepEqual(writes, [{ windowId: "agent-visible", text: "visible" }]);
  assert.equal(batcher.pendingCount("agent-hidden"), 2);
  assert.equal(batcher.pendingWindowCount(), 1);
  assert.equal(
    scheduler.pendingCount(),
    0,
    "ineligible-only pending output must not schedule a follow-up spin",
  );
});

test("quiesced hidden terminal output does not reschedule until reveal", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const eligibility = new Map([["agent-hidden", false]]);
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    canWrite: (windowId) => eligibility.get(windowId) !== false,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-hidden", "hidden-a");
  scheduler.runOnce();

  assert.deepEqual(writes, []);
  assert.equal(batcher.pendingCount("agent-hidden"), 1);
  assert.equal(scheduler.pendingCount(), 0);

  batcher.enqueue("agent-hidden", "hidden-b");
  batcher.enqueue("agent-hidden", "hidden-c");

  assert.equal(
    scheduler.pendingCount(),
    0,
    "already-quiesced hidden output must not create empty scheduler frames",
  );
  assert.equal(batcher.pendingCount("agent-hidden"), 3);

  eligibility.set("agent-hidden", true);
  assert.equal(batcher.schedulePending("agent-hidden"), true);
  assert.equal(scheduler.pendingCount(), 1);

  scheduler.runOnce();

  assert.deepEqual(writes, [
    { windowId: "agent-hidden", text: "hidden-ahidden-bhidden-c" },
  ]);
  assert.equal(batcher.pendingCount("agent-hidden"), 0);
  assert.equal(scheduler.pendingCount(), 0);
});

test("quiesced hidden terminal output does not block visible output scheduling", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const eligibility = new Map([
    ["agent-hidden", false],
    ["agent-visible", true],
  ]);
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    canWrite: (windowId) => eligibility.get(windowId) !== false,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-hidden", "hidden-a");
  scheduler.runOnce();
  assert.equal(scheduler.pendingCount(), 0);

  batcher.enqueue("agent-hidden", "hidden-b");
  assert.equal(scheduler.pendingCount(), 0);

  batcher.enqueue("agent-visible", "visible");

  assert.equal(
    scheduler.pendingCount(),
    1,
    "eligible output must still schedule while hidden output is quiesced",
  );

  scheduler.runOnce();

  assert.deepEqual(writes, [{ windowId: "agent-visible", text: "visible" }]);
  assert.equal(batcher.pendingCount("agent-hidden"), 2);
  assert.equal(batcher.pendingWindowCount(), 1);
  assert.equal(scheduler.pendingCount(), 0);
});

test("schedulePending resumes hidden queued output through the budgeted flush path", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const eligibility = new Map([["agent-hidden", false]]);
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    canWrite: (windowId) => eligibility.get(windowId) !== false,
    maxCharsPerFlush: 6,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-hidden", "aa");
  batcher.enqueue("agent-hidden", "bb");
  batcher.enqueue("agent-hidden", "cc");
  batcher.enqueue("agent-hidden", "dd");
  scheduler.runOnce();

  assert.deepEqual(writes, []);
  assert.equal(batcher.pendingCount("agent-hidden"), 4);
  assert.equal(scheduler.pendingCount(), 0);

  eligibility.set("agent-hidden", true);
  assert.equal(batcher.schedulePending("agent-hidden"), true);
  assert.equal(scheduler.pendingCount(), 1);

  scheduler.runOnce();

  assert.deepEqual(writes, [{ windowId: "agent-hidden", text: "aabbcc" }]);
  assert.equal(
    batcher.pendingCount("agent-hidden"),
    1,
    "existing per-window budget must still hold after reveal",
  );
  assert.equal(scheduler.pendingCount(), 1, "remaining chunks use one follow-up frame");

  scheduler.runOnce();

  assert.deepEqual(writes, [
    { windowId: "agent-hidden", text: "aabbcc" },
    { windowId: "agent-hidden", text: "dd" },
  ]);
  assert.equal(batcher.pendingCount("agent-hidden"), 0);
  assert.equal(scheduler.pendingCount(), 0);
});

test("flushNow drains a window before its scheduled frame", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const flushed = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
    onFlush: (windowId) => flushed.push(windowId),
  });

  batcher.enqueue("agent-1", "before");
  assert.equal(batcher.flushNow("agent-1"), true);
  assert.deepEqual(writes, [{ windowId: "agent-1", text: "before" }]);
  assert.deepEqual(flushed, ["agent-1"]);

  scheduler.runAll();
  assert.equal(writes.length, 1, "stale scheduled frames must no-op after flushNow");
});

test("clear drops pending output for a window", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-1", "old");
  batcher.enqueue("agent-2", "keep");
  batcher.clear("agent-1");

  scheduler.runAll();

  assert.deepEqual(writes, [{ windowId: "agent-2", text: "keep" }]);
  assert.equal(batcher.pendingCount("agent-1"), 0);
});

test("flush budget rolls excess chunks into later scheduled flushes without splitting chunks", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxCharsPerFlush: 5,
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-1", "ab");
  batcher.enqueue("agent-1", "cd");
  batcher.enqueue("agent-1", "ef");
  batcher.enqueue("agent-1", "gh");

  scheduler.runOnce();
  assert.deepEqual(writes, [{ windowId: "agent-1", text: "abcd" }]);
  assert.equal(scheduler.pendingCount(), 1, "remaining chunks schedule another frame");

  scheduler.runOnce();
  assert.deepEqual(writes, [
    { windowId: "agent-1", text: "abcd" },
    { windowId: "agent-1", text: "efgh" },
  ]);
});

test("mergeChunks runs during the scheduled flush with original chunk order", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const mergeCalls = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    mergeChunks: (chunks, windowId) => {
      mergeCalls.push({ windowId, chunks: [...chunks] });
      return chunks.map((chunk) => chunk.toUpperCase()).join("|");
    },
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-1", "ab");
  batcher.enqueue("agent-1", "cd");

  assert.deepEqual(mergeCalls, [], "merge must not run on enqueue");

  scheduler.runOnce();

  assert.deepEqual(mergeCalls, [
    { windowId: "agent-1", chunks: ["ab", "cd"] },
  ]);
  assert.deepEqual(writes, [{ windowId: "agent-1", text: "AB|CD" }]);
});

test("defaults expose bounded per-frame budgets", () => {
  assert.equal(DEFAULT_MAX_CHARS_PER_FLUSH, 65536);
  assert.equal(DEFAULT_MAX_WINDOWS_PER_FLUSH, 8);
  assert.equal(DEFAULT_MAX_TOTAL_CHARS_PER_FLUSH, DEFAULT_MAX_CHARS_PER_FLUSH);
});

test("terminal output trace follows enqueue through the next paint", () => {
  const flushScheduler = manualScheduler();
  const paintScheduler = manualScheduler();
  const traces = [];
  let completeWrite;
  const batcher = createTerminalOutputBatcher({
    schedule: flushScheduler.schedule,
    schedulePaint: paintScheduler.schedule,
    onTrace: (kind, fields) => traces.push({ kind, fields }),
    write: (_windowId, _text, done) => {
      completeWrite = done;
    },
  });

  batcher.enqueue("agent-trace", "encoded-output", {
    sequence: 41,
    window_id: "agent-trace",
  });
  assert.deepEqual(
    traces.map(({ kind }) => kind),
    ["terminal_output_enqueue"],
  );

  flushScheduler.runOnce();
  assert.deepEqual(
    traces.map(({ kind }) => kind),
    ["terminal_output_enqueue", "terminal_output_flush_start"],
  );
  assert.equal(paintScheduler.pendingCount(), 0);

  completeWrite();
  assert.deepEqual(
    traces.map(({ kind }) => kind),
    [
      "terminal_output_enqueue",
      "terminal_output_flush_start",
      "terminal_output_write_complete",
    ],
  );
  assert.equal(paintScheduler.pendingCount(), 1);

  paintScheduler.runOnce();
  assert.deepEqual(
    traces.map(({ kind }) => kind),
    [
      "terminal_output_enqueue",
      "terminal_output_flush_start",
      "terminal_output_write_complete",
      "terminal_output_next_paint",
    ],
  );
  assert.ok(
    traces.every(({ fields }) => (
      fields.sequence === 41 && fields.window_id === "agent-trace"
    )),
    "every stage must preserve the same local sequence and window",
  );
});

test("one merged xterm write retains every terminal output sequence", () => {
  const flushScheduler = manualScheduler();
  const paintScheduler = manualScheduler();
  const traces = [];
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: flushScheduler.schedule,
    schedulePaint: paintScheduler.schedule,
    onTrace: (kind, fields) => traces.push({ kind, fields }),
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-merged", "a", {
    sequence: 7,
    window_id: "agent-merged",
  });
  batcher.enqueue("agent-merged", "b", {
    sequence: 8,
    window_id: "agent-merged",
  });
  flushScheduler.runOnce();
  paintScheduler.runOnce();

  assert.deepEqual(writes, [{ windowId: "agent-merged", text: "ab" }]);
  for (const kind of [
    "terminal_output_enqueue",
    "terminal_output_flush_start",
    "terminal_output_write_complete",
    "terminal_output_next_paint",
  ]) {
    assert.deepEqual(
      traces
        .filter((trace) => trace.kind === kind)
        .map((trace) => trace.fields.sequence),
      [7, 8],
      `${kind} must retain both merged output sequences`,
    );
  }
});

test("hidden output from an earlier trace cannot emit stages after reveal", () => {
  const flushScheduler = manualScheduler();
  const paintScheduler = manualScheduler();
  const traces = [];
  const writes = [];
  const firstEpoch = Symbol("trace-a");
  const secondEpoch = Symbol("trace-b");
  let currentEpoch = firstEpoch;
  let visible = false;
  const batcher = createTerminalOutputBatcher({
    schedule: flushScheduler.schedule,
    schedulePaint: paintScheduler.schedule,
    shouldTrace: () => true,
    readTraceEpoch: () => currentEpoch,
    canWrite: () => visible,
    onTrace: (kind, fields) => traces.push({ kind, fields }),
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-hidden-epoch", "old-output", {
    sequence: 1,
    window_id: "agent-hidden-epoch",
    epoch: firstEpoch,
  });
  assert.deepEqual(
    traces.map(({ kind }) => kind),
    ["terminal_output_enqueue"],
  );

  flushScheduler.runOnce();
  traces.length = 0;
  currentEpoch = secondEpoch;
  visible = true;
  batcher.schedulePending("agent-hidden-epoch");
  flushScheduler.runOnce();
  paintScheduler.runAll();

  assert.deepEqual(writes, [
    { windowId: "agent-hidden-epoch", text: "old-output" },
  ]);
  assert.deepEqual(
    traces,
    [],
    "trace A metadata must not produce flush/write/paint markers in trace B",
  );
});

test("pending and deferred stale metadata stays outside the current trace at enqueue", () => {
  const flushScheduler = manualScheduler();
  const paintScheduler = manualScheduler();
  const traces = [];
  const writes = [];
  const staleEpoch = Symbol("trace-a");
  const currentEpoch = Symbol("trace-b");
  const batcher = createTerminalOutputBatcher({
    schedule: flushScheduler.schedule,
    schedulePaint: paintScheduler.schedule,
    shouldTrace: () => true,
    readTraceEpoch: () => currentEpoch,
    onTrace: (kind, fields) => traces.push({ kind, fields }),
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  for (const [sequence, text] of [
    [11, "pending-output"],
    [12, "deferred-output"],
  ]) {
    batcher.enqueue("agent-replay", text, {
      sequence,
      window_id: "agent-replay",
      epoch: staleEpoch,
    });
  }
  flushScheduler.runOnce();
  paintScheduler.runAll();

  assert.deepEqual(writes, [
    { windowId: "agent-replay", text: "pending-outputdeferred-output" },
  ]);
  assert.deepEqual(traces, []);
});

test("mixed trace epochs preserve terminal stages only for current metadata", () => {
  const flushScheduler = manualScheduler();
  const paintScheduler = manualScheduler();
  const traces = [];
  const writes = [];
  const staleEpoch = Symbol("trace-a");
  const currentEpoch = Symbol("trace-b");
  const batcher = createTerminalOutputBatcher({
    schedule: flushScheduler.schedule,
    schedulePaint: paintScheduler.schedule,
    shouldTrace: () => true,
    readTraceEpoch: () => currentEpoch,
    onTrace: (kind, fields) => traces.push({ kind, fields }),
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-mixed", "old", {
    sequence: 21,
    window_id: "agent-mixed",
    epoch: staleEpoch,
  });
  batcher.enqueue("agent-mixed", "new", {
    sequence: 22,
    window_id: "agent-mixed",
    epoch: currentEpoch,
  });
  flushScheduler.runOnce();
  paintScheduler.runOnce();

  assert.deepEqual(writes, [{ windowId: "agent-mixed", text: "oldnew" }]);
  for (const kind of [
    "terminal_output_enqueue",
    "terminal_output_flush_start",
    "terminal_output_write_complete",
    "terminal_output_next_paint",
  ]) {
    assert.deepEqual(
      traces
        .filter((trace) => trace.kind === kind)
        .map((trace) => trace.fields),
      [{ sequence: 22, window_id: "agent-mixed" }],
      `${kind} must retain only current-epoch metadata`,
    );
  }
});

test("trace epoch is rechecked at write completion and next paint", () => {
  const flushScheduler = manualScheduler();
  const paintScheduler = manualScheduler();
  const traces = [];
  const firstEpoch = Symbol("trace-a");
  const secondEpoch = Symbol("trace-b");
  const thirdEpoch = Symbol("trace-c");
  let currentEpoch = firstEpoch;
  const completions = [];
  const batcher = createTerminalOutputBatcher({
    schedule: flushScheduler.schedule,
    schedulePaint: paintScheduler.schedule,
    shouldTrace: () => true,
    readTraceEpoch: () => currentEpoch,
    onTrace: (kind, fields) => traces.push({ kind, fields }),
    write: (_windowId, _text, done) => completions.push(done),
  });

  batcher.enqueue("agent-stage", "first", {
    sequence: 31,
    window_id: "agent-stage",
    epoch: firstEpoch,
  });
  flushScheduler.runOnce();
  currentEpoch = secondEpoch;
  completions.shift()();
  assert.equal(paintScheduler.pendingCount(), 0);

  batcher.enqueue("agent-stage", "second", {
    sequence: 32,
    window_id: "agent-stage",
    epoch: secondEpoch,
  });
  flushScheduler.runOnce();
  completions.shift()();
  assert.equal(paintScheduler.pendingCount(), 1);
  currentEpoch = thirdEpoch;
  paintScheduler.runOnce();

  assert.deepEqual(
    traces
      .filter(({ kind }) => kind === "terminal_output_write_complete")
      .map(({ fields }) => fields.sequence),
    [32],
  );
  assert.deepEqual(
    traces.filter(({ kind }) => kind === "terminal_output_next_paint"),
    [],
  );
});

test("terminal output markers use exact fields and never serialize sentinels", () => {
  const flushScheduler = manualScheduler();
  const paintScheduler = manualScheduler();
  const traces = [];
  const batcher = createTerminalOutputBatcher({
    schedule: flushScheduler.schedule,
    schedulePaint: paintScheduler.schedule,
    onTrace: (kind, fields) => traces.push({ kind, fields }),
    write: (_windowId, _text, done) => done(),
  });
  const metadata = {
    sequence: 99,
    window_id: "agent-private",
    typed_text: "sentinel-typed-text",
    data_base64: "sentinel-base64",
    output_bytes: "sentinel-output-bytes",
    credential: "sentinel-credential",
    env_secret: "sentinel-env-secret",
  };

  batcher.enqueue("agent-private", "sentinel-output-body", metadata);
  flushScheduler.runOnce();
  paintScheduler.runOnce();

  assert.equal(traces.length, 4);
  for (const trace of traces) {
    assert.deepEqual(Object.keys(trace.fields).sort(), ["sequence", "window_id"]);
  }
  assert.equal(
    new Set(traces.map(({ fields }) => fields)).size,
    traces.length,
    "each onTrace stage must receive a fresh exact-field projection",
  );
  const serialized = JSON.stringify(traces);
  for (const sentinel of [
    "sentinel-typed-text",
    "sentinel-base64",
    "sentinel-output-bytes",
    "sentinel-credential",
    "sentinel-env-secret",
    "sentinel-output-body",
  ]) {
    assert.equal(serialized.includes(sentinel), false, `${sentinel} must not leak`);
  }
});

test("disabled terminal tracing avoids metadata reads callbacks and paint scheduling", () => {
  const flushScheduler = manualScheduler();
  const paintScheduler = manualScheduler();
  const writes = [];
  const traces = [];
  const metadata = {};
  for (const field of ["sequence", "window_id", "epoch", "credential", "env_secret"]) {
    Object.defineProperty(metadata, field, {
      get() {
        throw new Error(`${field} must not be read while tracing is disabled`);
      },
    });
  }
  const batcher = createTerminalOutputBatcher({
    schedule: flushScheduler.schedule,
    schedulePaint: paintScheduler.schedule,
    shouldTrace: () => false,
    readTraceEpoch: () => {
      throw new Error("inactive tracing must not read the trace epoch");
    },
    onTrace: (kind, fields) => traces.push({ kind, fields }),
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  assert.doesNotThrow(() => {
    batcher.enqueue("agent-inactive", "plain-output", metadata);
    flushScheduler.runOnce();
  });
  assert.deepEqual(writes, [
    { windowId: "agent-inactive", text: "plain-output" },
  ]);
  assert.deepEqual(traces, []);
  assert.equal(paintScheduler.pendingCount(), 0);
});

test("terminal trace callback failures never interrupt xterm output", () => {
  const flushScheduler = manualScheduler();
  const paintScheduler = manualScheduler();
  const writes = [];
  const batcher = createTerminalOutputBatcher({
    schedule: flushScheduler.schedule,
    schedulePaint: paintScheduler.schedule,
    onTrace: () => {
      throw new Error("diagnostics failed");
    },
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  assert.doesNotThrow(() => {
    batcher.enqueue("agent-safe", "still-rendered", {
      sequence: 123,
      window_id: "agent-safe",
    });
    flushScheduler.runOnce();
    paintScheduler.runOnce();
  });
  assert.deepEqual(writes, [
    { windowId: "agent-safe", text: "still-rendered" },
  ]);
});
