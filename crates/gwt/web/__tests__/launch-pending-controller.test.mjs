// SPEC-2359 W-17 (FR-398) — pending state for Resume / Launch requests.
//
// The controller is the single owner of "a Resume/Launch request is in
// flight": entry points call begin() before sending (double-click guard),
// the dispatcher settles on the backend ack/error, and a timeout clears a
// stuck pending entry when the backend never answers.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  continueWorkOutcomeNotice,
  createContinueWorkDispatcher,
  createLaunchPendingController,
  isStrongContinueWorkSuccess,
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

test("correlated Continue work settles only the exact operation and Work", () => {
  const timers = createFakeTimers();
  const controller = createLaunchPendingController(timers);

  assert.equal(
    controller.beginCorrelated(
      "continue:work-1",
      "continue-operation-1",
      "work-1",
      "Continue work",
    ),
    true,
  );
  assert.equal(
    controller.beginCorrelated(
      "continue:work-1",
      "continue-operation-2",
      "work-1",
      "Continue work",
    ),
    false,
    "a duplicate click cannot replace the in-flight operation",
  );
  assert.equal(
    controller.settleCorrelated({
      kind: "continue_work_outcome",
      operation_id: "stale-operation",
      work_id: "work-1",
    }),
    false,
    "a stale operation result has zero effect",
  );
  assert.equal(controller.isPending("continue:work-1"), true);
  assert.equal(
    controller.settleCorrelated({
      kind: "continue_work_outcome",
      operation_id: "continue-operation-1",
      work_id: "foreign-work",
    }),
    false,
    "a result for another Work has zero effect",
  );
  assert.equal(controller.isPending("continue:work-1"), true);
  assert.equal(
    controller.settleCorrelated({
      kind: "continue_work_outcome",
      operation_id: "continue-operation-1",
      work_id: "work-1",
    }),
    true,
  );
  assert.equal(controller.isPending("continue:work-1"), false);
});

test("Continue work timeout retains its operation id for an idempotent retry", () => {
  const timers = createFakeTimers();
  const controller = createLaunchPendingController(timers);

  controller.beginCorrelated(
    "continue:work-1",
    "continue-operation-1",
    "work-1",
    "Continue work",
  );
  timers.fireAll();

  assert.equal(controller.isPending("continue:work-1"), false);
  assert.equal(
    controller.correlatedOperation("continue:work-1"),
    "continue-operation-1",
    "timeout must retain the operation selected for authoritative retry",
  );
  assert.equal(controller.retryCorrelated("continue:work-1"), true);
  assert.equal(controller.isPending("continue:work-1"), true);
  assert.equal(
    controller.correlatedOperation("continue:work-1"),
    "continue-operation-1",
  );
});

test("Continue work dispatcher emits only opaque Work intent and reuses its operation on retry", () => {
  const timers = createFakeTimers();
  const pending = createLaunchPendingController(timers);
  const sent = [];
  const dispatcher = createContinueWorkDispatcher({
    launchPending: pending,
    send: (message) => sent.push(message),
    createOperationId: () => "continue-operation-1",
  });
  const bounds = { x: 10, y: 20, width: 800, height: 600 };

  assert.equal(dispatcher.dispatch("work-opaque-1", bounds), true);
  assert.deepEqual(sent, [{
    kind: "continue_work",
    operation_id: "continue-operation-1",
    work_id: "work-opaque-1",
    bounds,
  }]);
  assert.deepEqual(
    Object.keys(sent[0]).sort(),
    ["bounds", "kind", "operation_id", "work_id"],
    "Session, conversation, generation, binding, and Host authority never cross the public wire",
  );

  assert.equal(
    dispatcher.dispatch("work-opaque-1", bounds),
    false,
    "a duplicate click cannot send a second request",
  );
  timers.fireAll();
  assert.equal(dispatcher.dispatch("work-opaque-1", bounds), true);
  assert.equal(sent.length, 2);
  assert.equal(
    sent[1].operation_id,
    "continue-operation-1",
    "a timeout retry preserves the idempotency key",
  );
});

test("Continue work dispatcher ignores stale outcomes and settles only strong exact correlation", () => {
  const timers = createFakeTimers();
  const pending = createLaunchPendingController(timers);
  const dispatcher = createContinueWorkDispatcher({
    launchPending: pending,
    send() {},
    createOperationId: () => "continue-operation-1",
  });
  dispatcher.dispatch("work-opaque-1", { x: 0, y: 0, width: 1, height: 1 });

  assert.equal(dispatcher.handleOutcome({
    kind: "continue_work_outcome",
    operation_id: "stale-operation",
    work_id: "work-opaque-1",
    outcome: "continued_conversation",
  }), null);
  assert.equal(pending.isPending("continue:work-opaque-1"), true);

  assert.equal(dispatcher.handleOutcome({
    kind: "continue_work_outcome",
    operation_id: "continue-operation-1",
    work_id: "work-opaque-1",
    outcome: "continued_conversation",
    retryable: false,
  })?.outcome, "continued_conversation");
  assert.equal(pending.isPending("continue:work-opaque-1"), false);
});

test("Continue work outcome presentation distinguishes strong success, fallback, conflict, and failure", () => {
  assert.equal(isStrongContinueWorkSuccess({ outcome: "focused_existing" }), true);
  assert.equal(isStrongContinueWorkSuccess({ outcome: "continued_conversation" }), true);
  assert.equal(isStrongContinueWorkSuccess({ outcome: "started_with_handoff" }), true);
  assert.equal(isStrongContinueWorkSuccess({ outcome: "conflict_unknown" }), false);
  assert.equal(isStrongContinueWorkSuccess({ outcome: "failed" }), false);

  assert.deepEqual(
    continueWorkOutcomeNotice({ outcome: "started_with_handoff" }),
    {
      level: "info",
      title: "Work continued",
      message: "The previous conversation was unavailable, so a new conversation started with handoff context.",
    },
  );
  assert.deepEqual(
    continueWorkOutcomeNotice({
      outcome: "conflict_unknown",
      message: "The current owner could not be verified.",
      retryable: true,
    }),
    {
      level: "warn",
      title: "Continue work needs attention",
      message: "The current owner could not be verified.",
    },
  );
  assert.deepEqual(
    continueWorkOutcomeNotice({
      outcome: "failed",
      message: "The candidate did not become ready.",
      retryable: true,
    }),
    {
      level: "error",
      title: "Continue work failed",
      message: "The candidate did not become ready. You can try again.",
    },
  );
});
