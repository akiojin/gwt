// SPEC-2359 W-17 (FR-398) — pending state for Resume / Launch requests.
//
// The controller is the single owner of "a Resume/Launch request is in
// flight": entry points call begin() before sending (double-click guard),
// the dispatcher settles on the backend ack/error, and a timeout clears a
// stuck pending entry when the backend never answers.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  createLaunchPendingController,
  LAUNCH_PENDING_TIMEOUT_MS,
} from "../launch-pending-controller.js";

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
    fire(id) {
      const timer = timers.get(id);
      timers.delete(id);
      if (timer) timer.callback();
    },
    fireAll() {
      for (const id of [...timers.keys()]) this.fire(id);
    },
    size() {
      return timers.size;
    },
  };
}

test("begin marks the key pending once and guards double-sends", () => {
  const timers = createFakeTimers();
  const controller = createLaunchPendingController(timers);

  assert.equal(controller.begin("session:work-1", "Resume"), true);
  assert.equal(controller.isPending("session:work-1"), true);
  assert.equal(
    controller.begin("session:work-1", "Resume"),
    false,
    "second begin for the same key must be rejected (double-click guard)",
  );
  assert.equal(controller.pendingCount(), 1);
});

test("settleAck clears pending by session id and by branch", () => {
  const timers = createFakeTimers();
  const controller = createLaunchPendingController(timers);

  controller.begin("session:work-1", "Resume");
  controller.begin("branch:feature/x", "Resume");

  controller.settleAck({ session_id: "work-1", branch: "feature/x" });

  assert.equal(controller.isPending("session:work-1"), false);
  assert.equal(controller.isPending("branch:feature/x"), false);
  assert.equal(timers.size(), 0, "settling clears the timeout timers");
});

test("settleWhere clears every key with the given prefix", () => {
  const timers = createFakeTimers();
  const controller = createLaunchPendingController(timers);

  controller.begin("branch:feature/a", "Resume");
  controller.begin("branch:feature/b", "Resume");
  controller.begin("session:work-1", "Resume");

  controller.settleWhere("branch:");

  assert.equal(controller.isPending("branch:feature/a"), false);
  assert.equal(controller.isPending("branch:feature/b"), false);
  assert.equal(controller.isPending("session:work-1"), true);
});

test("timeout clears the pending entry and surfaces a one-shot notice", () => {
  const timers = createFakeTimers();
  let changes = 0;
  const controller = createLaunchPendingController({
    ...timers,
    onChange: () => {
      changes += 1;
    },
  });

  controller.begin("session:work-1", "Resume");
  assert.equal(changes, 1, "begin notifies listeners");

  timers.fireAll();

  assert.equal(controller.isPending("session:work-1"), false);
  assert.equal(changes, 2, "timeout notifies listeners");
  const notice = controller.consumeTimeoutNotice();
  assert.match(notice, /timed out/i);
  assert.equal(
    controller.consumeTimeoutNotice(),
    "",
    "notice is one-shot — consuming clears it",
  );
});

test("timeout duration uses the exported constant", () => {
  const recorded = [];
  const controller = createLaunchPendingController({
    setTimeoutFn(callback, ms) {
      recorded.push(ms);
      return 1;
    },
    clearTimeoutFn() {},
  });
  controller.begin("session:work-1", "Resume");
  assert.deepEqual(recorded, [LAUNCH_PENDING_TIMEOUT_MS]);
});
