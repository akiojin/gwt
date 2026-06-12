// SPEC-2359 W-17 (FR-399) — explicit full-screen overlay while the
// WebSocket bridge is down.
//
// Before this phase the only disconnect signal was a small status-strip
// label; every click during an outage was silently queued or dropped, which
// users experienced as "the app stopped responding". The overlay makes the
// outage explicit, blocks interaction that could not succeed anyway, and
// clears itself on reconnect. A short grace period keeps quick reconnect
// flaps (e.g. server-side queue rebalance) from flashing the overlay.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import {
  createConnectionOverlay,
  CONNECTION_OVERLAY_GRACE_MS,
} from "../connection-overlay.js";

function createFakeTimers() {
  const timers = new Map();
  let nextId = 1;
  return {
    setTimeoutFn(callback, ms) {
      const id = nextId;
      nextId += 1;
      timers.set(id, { callback, ms });
      return id;
    },
    clearTimeoutFn(id) {
      timers.delete(id);
    },
    fireAll() {
      for (const id of [...timers.keys()]) {
        const timer = timers.get(id);
        timers.delete(id);
        timer.callback();
      }
    },
    size() {
      return timers.size;
    },
    lastDelay: () => [...timers.values()].at(-1)?.ms,
  };
}

function createFixture() {
  const { document } = parseHTML("<body></body>");
  return { document };
}

test("disconnect shows the overlay only after the grace period", () => {
  const fixture = createFixture();
  const timers = createFakeTimers();
  const overlay = createConnectionOverlay({
    document: fixture.document,
    ...timers,
  });

  overlay.setConnected(false);
  assert.equal(
    fixture.document.querySelector(".connection-overlay"),
    null,
    "no overlay before the grace period elapses",
  );
  assert.equal(timers.lastDelay(), CONNECTION_OVERLAY_GRACE_MS);

  timers.fireAll();

  const node = fixture.document.querySelector(".connection-overlay");
  assert.ok(node, "overlay appears once the grace period elapses");
  assert.match(node.textContent, /Reconnecting/);
});

test("a quick reconnect flap never shows the overlay", () => {
  const fixture = createFixture();
  const timers = createFakeTimers();
  const overlay = createConnectionOverlay({
    document: fixture.document,
    ...timers,
  });

  overlay.setConnected(false);
  overlay.setConnected(true);
  timers.fireAll();

  assert.equal(
    fixture.document.querySelector(".connection-overlay"),
    null,
    "grace timer must be cancelled by the reconnect",
  );
});

test("reconnect removes a visible overlay", () => {
  const fixture = createFixture();
  const timers = createFakeTimers();
  const overlay = createConnectionOverlay({
    document: fixture.document,
    ...timers,
  });

  overlay.setConnected(false);
  timers.fireAll();
  assert.ok(fixture.document.querySelector(".connection-overlay"));

  overlay.setConnected(true);
  assert.equal(fixture.document.querySelector(".connection-overlay"), null);
});

test("repeated disconnect notifications do not stack overlays or timers", () => {
  const fixture = createFixture();
  const timers = createFakeTimers();
  const overlay = createConnectionOverlay({
    document: fixture.document,
    ...timers,
  });

  overlay.setConnected(false);
  overlay.setConnected(false);
  assert.equal(timers.size(), 1, "one grace timer at a time");
  timers.fireAll();
  overlay.setConnected(false);

  assert.equal(
    fixture.document.querySelectorAll(".connection-overlay").length,
    1,
    "a single overlay element",
  );
});
