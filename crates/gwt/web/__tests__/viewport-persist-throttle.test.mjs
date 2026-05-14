// Issue #2698 PR 2 (B1) — viewport-persist-throttle primitive tests
//
// `persistViewport()` previously sent a `update_viewport` WebSocket
// message on every wheel/zoom event (60-120 Hz on a Retina trackpad).
// Backend then re-broadcasts the resulting workspace_state to every
// connected client, driving a frontend re-render storm — even though
// the only relevant viewport state is the LATEST value.
//
// `createViewportPersistThrottle` collapses the spam to:
// - one send 100ms after the most recent event (tail debounce), and
// - one forced send every 500ms while events keep arriving (max wait),
//   guaranteeing backend sees fresh state during sustained gestures
// - flushNow() forces immediate dispatch of any pending state.

import assert from "node:assert/strict";
import test from "node:test";

import { createViewportPersistThrottle } from "../viewport-persist-throttle.js";

function manualClock() {
  let now = 0;
  return {
    advance(ms) {
      now += ms;
    },
    nowFn: () => now,
  };
}

function manualScheduler() {
  // Approximation of setTimeout: a min-heap-like list of
  // { fireAt, callback, id, cancelled }.
  const timers = [];
  let nextId = 1;
  return {
    setTimeout(callback, delay) {
      const id = nextId++;
      timers.push({ id, fireAt: delay, callback, cancelled: false });
      return id;
    },
    clearTimeout(id) {
      const t = timers.find((entry) => entry.id === id);
      if (t) t.cancelled = true;
    },
    advance(elapsed) {
      // Decrement all timers by elapsed and fire any that hit <= 0.
      const fired = [];
      for (const t of timers) {
        t.fireAt -= elapsed;
      }
      // Fire timers whose deadline has passed, in insertion order.
      while (true) {
        const idx = timers.findIndex((t) => !t.cancelled && t.fireAt <= 0);
        if (idx === -1) break;
        const [t] = timers.splice(idx, 1);
        fired.push(t);
        t.callback();
      }
      // Drop cancelled timers.
      for (let i = timers.length - 1; i >= 0; i -= 1) {
        if (timers[i].cancelled) timers.splice(i, 1);
      }
      return fired.length;
    },
    pendingCount() {
      return timers.filter((t) => !t.cancelled).length;
    },
  };
}

test("first schedule queues a tail send but does not dispatch synchronously", () => {
  const sent = [];
  const clock = manualClock();
  const scheduler = manualScheduler();
  const throttle = createViewportPersistThrottle({
    send: (payload) => sent.push(payload),
    tailMs: 100,
    maxWaitMs: 500,
    now: clock.nowFn,
    setTimeoutImpl: scheduler.setTimeout,
    clearTimeoutImpl: scheduler.clearTimeout,
  });

  throttle.schedule({ x: 10, y: 20, zoom: 1 });
  assert.deepEqual(sent, [], "no synchronous send");
  assert.equal(scheduler.pendingCount(), 1, "one tail timer scheduled");
});

test("tail timer fires after tailMs of quiet with the latest payload", () => {
  const sent = [];
  const clock = manualClock();
  const scheduler = manualScheduler();
  const throttle = createViewportPersistThrottle({
    send: (payload) => sent.push(payload),
    tailMs: 100,
    maxWaitMs: 500,
    now: clock.nowFn,
    setTimeoutImpl: scheduler.setTimeout,
    clearTimeoutImpl: scheduler.clearTimeout,
  });

  throttle.schedule({ x: 10, y: 20, zoom: 1 });
  throttle.schedule({ x: 11, y: 21, zoom: 1 });
  throttle.schedule({ x: 12, y: 22, zoom: 1 });
  clock.advance(100);
  scheduler.advance(100);
  assert.deepEqual(sent, [{ x: 12, y: 22, zoom: 1 }], "latest payload sent");
});

test("max-wait bound forces a flush during sustained gestures", () => {
  // Simulate continuous scroll: schedule every 16ms (60Hz). Without
  // max-wait, the tail timer would keep being reset and never fire.
  const sent = [];
  const clock = manualClock();
  const scheduler = manualScheduler();
  const throttle = createViewportPersistThrottle({
    send: (payload) => sent.push(payload),
    tailMs: 100,
    maxWaitMs: 500,
    now: clock.nowFn,
    setTimeoutImpl: scheduler.setTimeout,
    clearTimeoutImpl: scheduler.clearTimeout,
  });

  for (let i = 0; i < 50; i += 1) {
    throttle.schedule({ x: i, y: 0, zoom: 1 });
    clock.advance(16);
    scheduler.advance(16);
  }

  // 50 * 16ms = 800ms of continuous scroll. With maxWaitMs=500 we
  // expect at least one forced flush along the way.
  assert.ok(sent.length >= 1, `expected at least one flush, got ${sent.length}`);
  // Importantly, total sends should be a tiny fraction of 50.
  assert.ok(
    sent.length <= 3,
    `expected ≤3 sends during 800ms continuous scroll, got ${sent.length}`,
  );
});

test("flushNow drains pending payload immediately", () => {
  const sent = [];
  const clock = manualClock();
  const scheduler = manualScheduler();
  const throttle = createViewportPersistThrottle({
    send: (payload) => sent.push(payload),
    tailMs: 100,
    maxWaitMs: 500,
    now: clock.nowFn,
    setTimeoutImpl: scheduler.setTimeout,
    clearTimeoutImpl: scheduler.clearTimeout,
  });

  throttle.schedule({ x: 1, y: 2, zoom: 1 });
  throttle.schedule({ x: 3, y: 4, zoom: 1 });
  assert.deepEqual(sent, []);
  throttle.flushNow();
  assert.deepEqual(sent, [{ x: 3, y: 4, zoom: 1 }]);
  assert.equal(
    scheduler.pendingCount(),
    0,
    "flushNow must clear pending timers",
  );
});

test("flushNow with no pending payload is a no-op", () => {
  const sent = [];
  const clock = manualClock();
  const scheduler = manualScheduler();
  const throttle = createViewportPersistThrottle({
    send: (payload) => sent.push(payload),
    tailMs: 100,
    maxWaitMs: 500,
    now: clock.nowFn,
    setTimeoutImpl: scheduler.setTimeout,
    clearTimeoutImpl: scheduler.clearTimeout,
  });

  throttle.flushNow();
  assert.deepEqual(sent, []);
});

test("schedule after flushNow restarts the tail window cleanly", () => {
  const sent = [];
  const clock = manualClock();
  const scheduler = manualScheduler();
  const throttle = createViewportPersistThrottle({
    send: (payload) => sent.push(payload),
    tailMs: 100,
    maxWaitMs: 500,
    now: clock.nowFn,
    setTimeoutImpl: scheduler.setTimeout,
    clearTimeoutImpl: scheduler.clearTimeout,
  });

  throttle.schedule({ x: 1, y: 0, zoom: 1 });
  throttle.flushNow();
  assert.deepEqual(sent, [{ x: 1, y: 0, zoom: 1 }]);

  // New cycle: schedule again and verify the tail fires.
  throttle.schedule({ x: 2, y: 0, zoom: 1 });
  clock.advance(100);
  scheduler.advance(100);
  assert.deepEqual(sent, [
    { x: 1, y: 0, zoom: 1 },
    { x: 2, y: 0, zoom: 1 },
  ]);
});

test("rapid-then-quiet: 60Hz burst followed by silence produces a single tail send", () => {
  // The classical mouse-wheel pattern: a flurry of events then nothing.
  const sent = [];
  const clock = manualClock();
  const scheduler = manualScheduler();
  const throttle = createViewportPersistThrottle({
    send: (payload) => sent.push(payload),
    tailMs: 100,
    maxWaitMs: 500,
    now: clock.nowFn,
    setTimeoutImpl: scheduler.setTimeout,
    clearTimeoutImpl: scheduler.clearTimeout,
  });

  // Burst: 8 events over ~128ms (60Hz). Under maxWaitMs=500, no forced flush yet.
  for (let i = 0; i < 8; i += 1) {
    throttle.schedule({ x: i, y: 0, zoom: 1 });
    clock.advance(16);
    scheduler.advance(16);
  }
  assert.equal(sent.length, 0, "no forced flush during 128ms burst");

  // Silence: advance past the tail window.
  clock.advance(100);
  scheduler.advance(100);
  assert.deepEqual(
    sent,
    [{ x: 7, y: 0, zoom: 1 }],
    "exactly one tail send with the latest payload",
  );
});

test("payload always reflects the most recent value at flush time", () => {
  const sent = [];
  const clock = manualClock();
  const scheduler = manualScheduler();
  const throttle = createViewportPersistThrottle({
    send: (payload) => sent.push(payload),
    tailMs: 100,
    maxWaitMs: 500,
    now: clock.nowFn,
    setTimeoutImpl: scheduler.setTimeout,
    clearTimeoutImpl: scheduler.clearTimeout,
  });

  throttle.schedule({ x: 1, y: 1, zoom: 1 });
  throttle.schedule({ x: 2, y: 2, zoom: 1 });
  throttle.schedule({ x: 999, y: 999, zoom: 1.5 });
  clock.advance(100);
  scheduler.advance(100);
  assert.deepEqual(sent, [{ x: 999, y: 999, zoom: 1.5 }]);
});
