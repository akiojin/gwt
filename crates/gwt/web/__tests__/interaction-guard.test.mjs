// Issue #2698 PR 1 (B7) — interaction-guard primitive tests
//
// The wizard's `renderLaunchWizard()` does a destructive DOM swap of
// `wizardBody.innerHTML`. If a `launch_wizard_state` message arrives
// while a user has a native `<select>` dropdown open, the swap drops
// the `<select>` element out from under the OS dropdown overlay and
// the user's selection is lost ("dropdown 全般で選択できない").
//
// The interaction-guard primitive coalesces inbound state updates
// into "at most one latest pending value" while the guard is active,
// and replays that single value on release. We test the pure logic
// here so the integration into app.js can lean on a verified API.

import assert from "node:assert/strict";
import test from "node:test";

import { createInteractionGuard } from "../interaction-guard.js";

test("inactive guard reports active=false and does not defer", () => {
  const flushed = [];
  const guard = createInteractionGuard({ onFlush: (v) => flushed.push(v) });
  assert.equal(guard.isActive(), false);
  assert.equal(guard.defer({ kind: "launch_wizard_state", n: 1 }), false);
  assert.equal(guard.hasPendingValue(), false);
  assert.deepEqual(flushed, [], "defer on inactive guard must not call onFlush");
});

test("active guard defers values without dispatching", () => {
  const flushed = [];
  const guard = createInteractionGuard({ onFlush: (v) => flushed.push(v) });
  guard.activate();
  assert.equal(guard.isActive(), true);
  assert.equal(guard.defer({ n: 1 }), true);
  assert.deepEqual(flushed, [], "no flush while guard is active");
  assert.equal(guard.hasPendingValue(), true);
});

test("multiple defers during activation coalesce to the latest only", () => {
  const flushed = [];
  const guard = createInteractionGuard({ onFlush: (v) => flushed.push(v) });
  guard.activate();
  guard.defer({ n: 1 });
  guard.defer({ n: 2 });
  guard.defer({ n: 3 });
  guard.release();
  assert.deepEqual(
    flushed,
    [{ n: 3 }],
    "only the latest deferred value is flushed on release",
  );
  assert.equal(guard.hasPendingValue(), false);
});

test("release without any pending value does not call onFlush", () => {
  const flushed = [];
  const guard = createInteractionGuard({ onFlush: (v) => flushed.push(v) });
  guard.activate();
  guard.release();
  assert.deepEqual(flushed, []);
});

test("release without prior activate is a no-op", () => {
  const flushed = [];
  const guard = createInteractionGuard({ onFlush: (v) => flushed.push(v) });
  guard.release();
  assert.deepEqual(flushed, []);
  assert.equal(guard.isActive(), false);
});

test("activate after release accepts a fresh defer cycle", () => {
  const flushed = [];
  const guard = createInteractionGuard({ onFlush: (v) => flushed.push(v) });
  guard.activate();
  guard.defer({ n: 1 });
  guard.release();
  guard.activate();
  guard.defer({ n: 2 });
  guard.release();
  assert.deepEqual(flushed, [{ n: 1 }, { n: 2 }]);
});

test("defer on inactive guard does not poison pending state", () => {
  const flushed = [];
  const guard = createInteractionGuard({ onFlush: (v) => flushed.push(v) });
  // Before activation: defer is ignored entirely.
  guard.defer({ n: "ignored" });
  guard.activate();
  guard.defer({ n: 2 });
  guard.release();
  assert.deepEqual(
    flushed,
    [{ n: 2 }],
    "only values deferred while active are eligible for flush",
  );
});

test("peekPending returns the latest deferred value without consuming it", () => {
  const guard = createInteractionGuard({ onFlush: () => {} });
  guard.activate();
  guard.defer({ n: 1 });
  guard.defer({ n: 2 });
  assert.deepEqual(guard.peekPending(), { n: 2 });
  assert.equal(guard.hasPendingValue(), true);
  // Release flushes and clears.
  guard.release();
  assert.equal(guard.hasPendingValue(), false);
});

test("activate while already active is idempotent (no spurious state change)", () => {
  const flushed = [];
  const guard = createInteractionGuard({ onFlush: (v) => flushed.push(v) });
  guard.activate();
  guard.defer({ n: 1 });
  guard.activate(); // already active — pending must not flush
  assert.deepEqual(flushed, []);
  assert.equal(guard.hasPendingValue(), true);
  guard.release();
  assert.deepEqual(flushed, [{ n: 1 }]);
});

test("createInteractionGuard without onFlush still operates without throwing", () => {
  const guard = createInteractionGuard();
  guard.activate();
  guard.defer({ n: 1 });
  // release should not throw even when no onFlush is registered.
  guard.release();
  assert.equal(guard.hasPendingValue(), false);
});

test("discard clears active and pending without invoking onFlush", () => {
  // Used when a user-initiated action (e.g. closing the wizard
  // locally) supersedes any pending backend state. We must not
  // replay the deferred value because the local action is the
  // intended truth.
  const flushed = [];
  const guard = createInteractionGuard({ onFlush: (v) => flushed.push(v) });
  guard.activate();
  guard.defer({ n: 1 });
  guard.discard();
  assert.deepEqual(flushed, [], "discard must not call onFlush");
  assert.equal(guard.isActive(), false);
  assert.equal(guard.hasPendingValue(), false);
});

test("discard on inactive guard without pending is a no-op", () => {
  const flushed = [];
  const guard = createInteractionGuard({ onFlush: (v) => flushed.push(v) });
  guard.discard();
  assert.equal(guard.isActive(), false);
  assert.equal(guard.hasPendingValue(), false);
  assert.deepEqual(flushed, []);
});
