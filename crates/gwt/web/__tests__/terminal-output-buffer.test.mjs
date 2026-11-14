import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import * as terminalOutput from "../terminal-output-buffer.js";

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

function hiddenTasks(t, hidden = true) {
  const tasks = manualScheduler();
  const listeners = new Set(), stalled = [];
  const visibility = { hidden,
    addEventListener: (type, listener) => { assert.equal(type, "visibilitychange"); listeners.add(listener); },
    removeEventListener: (type, listener) => { assert.equal(type, "visibilitychange"); listeners.delete(listener); },
  };
  let closed = 0;
  for (const [key, value] of Object.entries({
    document: visibility,
    MessageChannel: class {
      port1 = { onmessage: null, close: () => { closed += 1; } };
      port2 = { close: () => { closed += 1; },
        postMessage: () => tasks.schedule(() => this.port1.onmessage?.()) };
    },
    requestAnimationFrame: callback => {
      assert.equal(visibility.hidden, false, "hidden work must not wait for an animation frame");
      stalled.push(callback); return stalled.length;
    },
    cancelAnimationFrame: () => {},
  })) {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
    Object.defineProperty(globalThis, key, { configurable: true, value });
    t.after(() => {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete globalThis[key];
    });
  }
  // Native background timers may be clamped; do not advance them in this test.
  t.mock.method(globalThis, "setTimeout", callback => { stalled.push(callback); return stalled.length; });
  t.mock.method(globalThis, "clearTimeout", () => {});
  return { tasks, stalled, closed: () => closed, listeners,
    hide() { visibility.hidden = true; for (const listener of [...listeners]) listener(); },
  };
}

test("output queued while visible drains once after the document becomes hidden", (t) => {
  const { tasks, stalled, closed, listeners, hide } = hiddenTasks(t, false);
  const writes = [];
  const batcher = createTerminalOutputBatcher({ write: (id, text) => writes.push([id, text]) });
  batcher.enqueue("agent", "a");
  batcher.enqueue("agent", "b");
  hide();
  assert.equal(tasks.pendingCount(), 1, "pending output must escape suspended frames and timers");
  tasks.runAll();
  for (const callback of stalled) callback();
  assert.deepEqual(writes, [["agent", "ab"]]);
  assert.equal(closed(), 2);
  assert.equal(listeners.size, 0);
});

test("hidden output batches advance by tasks while preserving order and window budgets", (t) => {
  const { tasks, closed } = hiddenTasks(t);
  const writes = [];
  const batcher = createTerminalOutputBatcher({ maxWindowsPerFlush: 1,
    write: (id, text) => writes.push([id, text]) });
  batcher.enqueue("first", "a");
  batcher.enqueue("first", "b");
  batcher.enqueue("second", "c");
  assert.deepEqual(writes, []);
  assert.equal(tasks.pendingCount(), 1);
  tasks.runOnce();
  assert.deepEqual(writes, [["first", "ab"]]);
  assert.equal(tasks.pendingCount(), 1);
  tasks.runOnce();
  assert.deepEqual(writes, [["first", "ab"], ["second", "c"]]);
  assert.equal(closed(), 4, "both ports close after each one-shot task");
});

test("vendored xterm coalesces hidden redraws and cancels disposed work without frames", (t) => {
  const { tasks, stalled, listeners, hide } = hiddenTasks(t);
  const source = readFileSync(new URL("../vendor/xterm/xterm.mjs", import.meta.url), "utf8");
  const start = source.indexOf("Ws=class{") + 3;
  const end = source.indexOf(",zs={}", start);
  assert.ok(start > 3 && end > start, "pinned xterm RenderDebouncer is present");
  const Debouncer = new Function("gwtScheduleTerminalTask", `return (${source.slice(start, end)});`)(
    (...args) => terminalOutput.scheduleTerminalTask(...args));
  const draws = [], callbacks = [];
  const debouncer = new Debouncer((first, last) => draws.push([first, last]), { window: globalThis });
  debouncer.refresh(0, 1, 4);
  debouncer.refresh(1, 3, 4);
  const handle = debouncer.addRefreshCallback(() => callbacks.push("done"));
  assert.notEqual(handle, undefined, "xterm decorations use the handle to deduplicate callbacks");
  assert.deepEqual(draws, []);
  assert.equal(tasks.pendingCount(), 1);
  tasks.runOnce();
  assert.deepEqual(draws, [[0, 3]]);
  assert.deepEqual(callbacks, ["done"]);
  debouncer.refresh(0, 1, 4);
  debouncer.dispose();
  tasks.runAll();
  assert.deepEqual(draws, [[0, 3]], "disposed terminal must not redraw");

  globalThis.document.hidden = false;
  const pending = new Debouncer((first, last) => draws.push([first, last]), { window: globalThis });
  pending.refresh(1, 2, 4);
  hide();
  assert.equal(tasks.pendingCount(), 1, "a visible redraw must survive a hidden transition");
  tasks.runAll();
  for (const callback of stalled) callback();
  assert.deepEqual(draws, [[0, 3], [1, 2]]);
  assert.equal(listeners.size, 0);
  globalThis.document.hidden = false;
  pending.refresh(0, 1, 4);
  pending.dispose();
  hide();
  tasks.runAll();
  for (const callback of stalled) callback();
  assert.equal(listeners.size, 0, "disposed redraws release the pending visibility listener");
  assert.deepEqual(draws, [[0, 3], [1, 2]]);
});

test("vendored xterm parser starts and yields hidden tasks at its existing budget", (t) => {
  const { tasks, stalled, listeners, hide } = hiddenTasks(t);
  const source = readFileSync(new URL("../vendor/xterm/xterm.mjs", import.meta.url), "utf8");
  const start = source.indexOf("In=class extends Ie{") + 3;
  const end = source.indexOf(",Nn=class", start);
  assert.ok(start > 3 && end > start, "pinned xterm WriteBuffer is present");
  let clock = 1;
  const Parser = new Function("Ie", "et", "performance", "gwtScheduleTerminalTask",
    `return (${source.slice(start, end)});`)(
    class { _register(value) { return value; } }, class { event() {} fire() {} },
    { now: () => clock }, callback => terminalOutput.scheduleTerminalTask(callback));
  const parsed = [], done = [];
  const parser = new Parser(text => { parsed.push(text); clock += 13; });
  parser.write("a", () => done.push("a"));
  parser.write("b", () => done.push("b"));
  assert.deepEqual(parsed, []);
  assert.equal(tasks.pendingCount(), 1);
  tasks.runOnce();
  assert.deepEqual(parsed, ["a"]);
  assert.deepEqual(done, ["a"]);
  assert.equal(tasks.pendingCount(), 1, "12ms budget yields to another task");
  tasks.runOnce();
  assert.deepEqual(parsed, ["a", "b"]);
  assert.deepEqual(done, ["a", "b"]);
  assert.equal(tasks.pendingCount(), 0);
  globalThis.document.hidden = false;
  parser.write("c", () => done.push("c"));
  hide();
  assert.equal(tasks.pendingCount(), 1, "a visible parser task must survive a hidden transition");
  tasks.runAll();
  for (const callback of stalled) callback();
  assert.deepEqual(parsed, ["a", "b", "c"]);
  assert.deepEqual(done, ["a", "b", "c"]);
  assert.equal(listeners.size, 0);
});

test("default output scheduler drains without animation frames and ignores the losing callback", (t) => {
  const frames = new Map(), timers = new Map(), writes = [];
  let nextId = 0;
  for (const [key, value] of Object.entries({
    requestAnimationFrame: (cb) => { frames.set(++nextId, cb); return nextId; },
    cancelAnimationFrame: (id) => frames.delete(id),
  })) {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
    Object.defineProperty(globalThis, key, { configurable: true, value });
    t.after(() => {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete globalThis[key];
    });
  }
  t.mock.method(globalThis, "setTimeout", (cb, delay) => {
    timers.set(++nextId, { cb, delay }); return nextId;
  });
  t.mock.method(globalThis, "clearTimeout", (id) => timers.delete(id));
  const batcher = createTerminalOutputBatcher({ write: (id, text) => writes.push([id, text]) });
  for (const winner of ["timer", "frame"]) {
    batcher.enqueue("agent", "a");
    batcher.enqueue("agent", "b");
    assert.equal(frames.size, 1);
    assert.equal(timers.size, 1, "output needs a fallback even when rAF exists");
    const frame = frames.values().next().value;
    const timer = timers.values().next().value;
    assert.ok(timer.delay > 0 && timer.delay <= 1000, "fallback is inside AC13's bound");
    (winner === "timer" ? timer.cb : frame)();
    assert.equal(frames.size, 0);
    assert.equal(timers.size, 0);
    assert.deepEqual(writes.at(-1), ["agent", "ab"]);
    batcher.enqueue("agent", "next");
    const count = writes.length;
    (winner === "timer" ? frame : timer.cb)();
    assert.equal(writes.length, count, "a stale loser must not drain the next batch");
    frames.values().next().value();
    assert.deepEqual(writes.at(-1), ["agent", "next"]);
  }
});

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

// SPEC-5016 AC-15: a hidden (quiesced) window must not retain unbounded raw
// output. Past the per-window cap the queue is released and the existing
// backend snapshot repair restores the screen instead.
test("quiesced terminal output is bounded per window and requests a snapshot on overflow", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const overflows = [];
  const eligibility = new Map([["agent-hidden", false], ["agent-visible", true]]);
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxPendingCharsPerWindow: 10,
    canWrite: (windowId) => eligibility.get(windowId) !== false,
    onOverflow: (windowId) => overflows.push(windowId),
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-hidden", "aaaa");
  scheduler.runOnce();
  assert.equal(batcher.pendingCount("agent-hidden"), 1, "first flush quiesces the hidden window");
  batcher.enqueue("agent-hidden", "bbbb");
  assert.deepEqual(overflows, [], "output within the cap stays queued");
  assert.equal(batcher.pendingCount("agent-hidden"), 2);

  batcher.enqueue("agent-hidden", "cccc");
  assert.deepEqual(overflows, ["agent-hidden"], "crossing the cap requests one snapshot");
  assert.equal(batcher.pendingCount("agent-hidden"), 0, "the over-cap queue is released");
  assert.equal(batcher.pendingWindowCount(), 0);
  assert.equal(batcher.isOverflowed("agent-hidden"), true);

  // Until the snapshot replaces the screen, later raw chunks are already
  // contained in that snapshot, so they are discarded rather than retained.
  assert.equal(batcher.enqueue("agent-hidden", "dddd"), false);
  assert.equal(batcher.pendingCount("agent-hidden"), 0);
  assert.deepEqual(overflows, ["agent-hidden"], "no duplicate request below another cap");
  batcher.enqueue("agent-hidden", "eeeeeeee");
  assert.deepEqual(overflows, ["agent-hidden", "agent-hidden"],
    "a lost request is retried after another cap of discarded output");

  // Revealing the window re-requests the pending snapshot without writing
  // stale bytes, once per batch of newly discarded output.
  eligibility.set("agent-hidden", true);
  assert.equal(batcher.schedulePending("agent-hidden"), false);
  assert.deepEqual(overflows, ["agent-hidden", "agent-hidden"],
    "nothing arrived since the last request");
  batcher.enqueue("agent-hidden", "ff");
  assert.equal(batcher.schedulePending("agent-hidden"), false);
  assert.equal(batcher.schedulePending("agent-hidden"), false);
  assert.deepEqual(overflows, ["agent-hidden", "agent-hidden", "agent-hidden"],
    "repeated layout passes request once");
  scheduler.runAll();
  assert.deepEqual(writes, []);

  // The snapshot path clears the window; live output resumes normally.
  batcher.clear("agent-hidden");
  assert.equal(batcher.isOverflowed("agent-hidden"), false);
  batcher.enqueue("agent-hidden", "live");
  batcher.enqueue("agent-visible", "other");
  scheduler.runAll();
  assert.deepEqual(writes, [
    { windowId: "agent-hidden", text: "live" },
    { windowId: "agent-visible", text: "other" },
  ]);
});

test("writable terminal output is never dropped by the hidden retention cap", () => {
  const scheduler = manualScheduler();
  const writes = [];
  const overflows = [];
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxPendingCharsPerWindow: 4,
    onOverflow: (windowId) => overflows.push(windowId),
    write: (windowId, text, done) => {
      writes.push({ windowId, text });
      done();
    },
  });

  batcher.enqueue("agent-1", "abc");
  batcher.enqueue("agent-1", "def");
  batcher.enqueue("agent-1", "ghi");
  scheduler.runAll();

  assert.deepEqual(overflows, []);
  assert.deepEqual(writes, [{ windowId: "agent-1", text: "abcdefghi" }]);
});

test("a throwing overflow callback does not break the output queue", () => {
  const scheduler = manualScheduler();
  const batcher = createTerminalOutputBatcher({
    schedule: scheduler.schedule,
    maxPendingCharsPerWindow: 2,
    canWrite: () => false,
    onOverflow: () => { throw new Error("socket closed"); },
    write: () => {},
  });
  batcher.enqueue("agent-1", "a");
  scheduler.runOnce();
  assert.doesNotThrow(() => batcher.enqueue("agent-1", "bc"));
  assert.equal(batcher.pendingCount("agent-1"), 0);
  assert.equal(batcher.isOverflowed("agent-1"), true);
});

test("default hidden retention cap is bounded and wired to the snapshot request", () => {
  assert.equal(terminalOutput.DEFAULT_MAX_PENDING_CHARS_PER_WINDOW, 4 * 1024 * 1024);
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  assert.match(app,
    /onOverflow: \(windowId\) => send\(\{ kind: "request_terminal_snapshot", id: windowId \}\)/,
    "app.js must turn a dropped hidden queue into a snapshot request");
});
