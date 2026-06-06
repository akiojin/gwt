import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_MAX_CHARS_PER_FLUSH,
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

  assert.equal(scheduler.pendingCount(), 2, "each window gets its own scheduled flush");
  scheduler.runAll();

  assert.deepEqual(writes, [
    { windowId: "agent-a", text: "a1a2" },
    { windowId: "agent-b", text: "b1b2" },
  ]);
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

test("defaults expose a bounded per-frame character budget", () => {
  assert.equal(DEFAULT_MAX_CHARS_PER_FLUSH, 65536);
});
