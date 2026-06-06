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
