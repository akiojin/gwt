import assert from "node:assert/strict";
import test from "node:test";

import * as notifications from "../agent-completion-notifications.js";

import {
  createAgentCompletionNotifier,
  createAgentAttentionToaster,
} from "../agent-completion-notifications.js";

function intent(overrides = {}) {
  return {
    transition_id: "runtime-one:1", project_key: "0123456789abcdef",
    window_id: "agent-1", category: "stopped", title: "Agent stopped",
    body: "Fixture agent stopped in Repo One.", ...overrides,
  };
}

test("server intents deliver once by transition ID without runtime or reconnect authority", () => {
  const desktop = [];
  const notifier = createAgentCompletionNotifier({
    getDesktopNotificationPermission: () => "granted",
    showDesktopNotification: (notice) => desktop.push(notice),
  });
  assert.equal(typeof notifier.handleIntent, "function");
  assert.equal(notifier.handleRuntimeState, undefined, "runtime frames cannot trigger OS notifications");
  assert.equal(notifier.reset, undefined, "socket reconnect cannot erase transition dedupe");
  assert.equal(notifier.forgetWindow, undefined, "window removal cannot erase transition dedupe");
  const stopped = intent();
  assert.deepEqual(notifier.handleIntent(stopped), stopped);
  assert.equal(notifier.handleIntent({ ...stopped }), null);
  const error = intent({ transition_id: "runtime-one:2", category: "error", title: "Agent error", body: "Stop-block hit an error" });
  assert.deepEqual(notifier.handleIntent(error), error);
  const human = intent({ transition_id: "runtime-one:3", category: "needs_human", window_id: null, title: "Needs human" });
  assert.deepEqual(notifier.handleIntent(human), human);
  assert.deepEqual(desktop, [stopped, error, human]);
});

test("desktop-only intents never prompt or publish an in-app toast when permission is not granted", () => {
  let permission = "default";
  const desktop = [];
  const toasts = [];
  const notifier = createAgentCompletionNotifier({
    getDesktopNotificationPermission: () => permission,
    showDesktopNotification: (notice) => desktop.push(notice),
    showToast: (notice) => toasts.push(notice),
  });
  for (const value of ["default", "denied", "unsupported"]) {
    permission = value;
    assert.equal(notifier.handleIntent(intent({ transition_id: value })), null);
  }
  permission = "granted";
  assert.equal(notifier.handleIntent(intent({ transition_id: "default" })), null, "granting later does not replay a consumed intent");
  assert.deepEqual(desktop, []);
  assert.deepEqual(toasts, []);
});

test("Idle, Waiting, and malformed messages cannot become desktop intents", () => {
  const desktop = [];
  const notifier = createAgentCompletionNotifier({
    getDesktopNotificationPermission: () => "granted",
    showDesktopNotification: (notice) => desktop.push(notice),
  });
  for (const value of [null, {}, intent({ transition_id: "" }), intent({ category: "idle" }), intent({ category: "waiting" })]) {
    assert.equal(notifier.handleIntent(value), null);
  }
  assert.deepEqual(desktop, []);
});

test("desktop notification click focuses its Project, activates its window, and closes", () => {
  const actions = [];
  const notifications = [];
  class NotificationProbe {
    static permission = "granted";
    static requestPermission() { throw new Error("must never request permission"); }
    constructor(title, options) { this.title = title; this.options = options; notifications.push(this); }
    close() { actions.push("close"); }
  }
  const notifier = createAgentCompletionNotifier({
    window: { Notification: NotificationProbe, focus: () => actions.push("focus") },
    onActivate: (notice) => actions.push(notice.window_id),
  });
  notifier.handleIntent(intent());
  assert.equal(notifications.length, 1);
  assert.equal(notifications[0].title, "Agent stopped");
  assert.equal(notifications[0].options.body, "Fixture agent stopped in Repo One.");
  assert.match(notifications[0].options.tag, /runtime-one:1/);
  notifications[0].onclick();
  assert.deepEqual(actions, ["focus", "agent-1", "close"]);
});

test("unsupported OS constructor failures are logged safely once and remain deduplicated", () => {
  let attempts = 0;
  const warnings = [];
  class NotificationProbe {
    static permission = "granted";
    constructor() { attempts++; throw new Error("OS notifications unavailable: /private/payload"); }
  }
  const notifier = createAgentCompletionNotifier({ window: { Notification: NotificationProbe, console: { warn: (...args) => warnings.push(args) } } });
  assert.equal(notifier.handleIntent(intent()), null);
  assert.equal(notifier.handleIntent(intent()), null);
  assert.equal(attempts, 1);
  assert.deepEqual(warnings, [["gwt: desktop notification delivery failed"]]);
});

// SPEC-2356 Anshin Addendum (FR-040) — in-app attention toaster.
function collectAttentionToaster() {
  const toasts = [];
  const toaster = createAgentAttentionToaster({
    showToast: (notice) => toasts.push(notice),
    now: () => 1000,
  });
  return { toaster, toasts };
}

test("FR-040: needs_input (waiting) fires an in-app toast even while present", () => {
  const { toaster, toasts } = collectAttentionToaster();
  const notice = toaster.handleRuntimeState({
    windowId: "w-1",
    runtimeState: "waiting",
    windowData: { title: "codex-1" },
  });
  assert.ok(notice, "waiting must produce a toast");
  assert.equal(notice.flavor, "needs_input");
  assert.equal(notice.windowId, "w-1");
  assert.match(notice.body, /codex-1/);
  assert.equal(toasts.length, 1);
});

test("FR-040: blocked/error and done states also toast", () => {
  const { toaster, toasts } = collectAttentionToaster();
  toaster.handleRuntimeState({ windowId: "w-err", runtimeState: "error", windowData: { title: "a" } });
  toaster.handleRuntimeState({ windowId: "w-done", runtimeState: "stopped", windowData: { title: "b" } });
  toaster.handleRuntimeState({ windowId: "w-exit", runtimeState: "exited", windowData: { title: "c" } });
  assert.deepEqual(
    toasts.map((t) => t.flavor),
    ["error", "done", "done"],
  );
});

test("FR-040: error toast includes the runtime status detail", () => {
  const { toaster, toasts } = collectAttentionToaster();
  const notice = toaster.handleRuntimeState({
    windowId: "w-err",
    runtimeState: "error",
    windowData: { title: "a" },
    statusDetail: "Stop-block hit an error",
  });
  assert.equal(notice.flavor, "error");
  assert.match(notice.body, /Stop-block hit an error/);
  assert.deepEqual(toasts, [notice]);
});

test("FR-040: running / starting / idle never toast", () => {
  const { toaster, toasts } = collectAttentionToaster();
  for (const state of ["running", "starting", "idle", "ready"]) {
    toaster.handleRuntimeState({ windowId: "w-1", runtimeState: state, windowData: {} });
  }
  assert.equal(toasts.length, 0);
});

test("FR-040: the same flavor does not re-toast across repeated frames", () => {
  const { toaster, toasts } = collectAttentionToaster();
  toaster.handleRuntimeState({ windowId: "w-1", runtimeState: "waiting", windowData: {} });
  toaster.handleRuntimeState({ windowId: "w-1", runtimeState: "waiting", windowData: {} });
  toaster.handleRuntimeState({ windowId: "w-1", runtimeState: "waiting", windowData: {} });
  assert.equal(toasts.length, 1, "repeated waiting frames must not spam");
});

test("FR-040: leaving and re-entering an attention state toasts again", () => {
  const { toaster, toasts } = collectAttentionToaster();
  toaster.handleRuntimeState({ windowId: "w-1", runtimeState: "waiting", windowData: {} });
  toaster.handleRuntimeState({ windowId: "w-1", runtimeState: "running", windowData: {} });
  toaster.handleRuntimeState({ windowId: "w-1", runtimeState: "waiting", windowData: {} });
  assert.equal(toasts.length, 2, "re-entry into waiting after running must toast again");
});

test("FR-040: error -> done transition toasts each distinct flavor", () => {
  const { toaster, toasts } = collectAttentionToaster();
  toaster.handleRuntimeState({ windowId: "w-1", runtimeState: "error", windowData: {} });
  toaster.handleRuntimeState({ windowId: "w-1", runtimeState: "stopped", windowData: {} });
  assert.deepEqual(toasts.map((t) => t.flavor), ["error", "done"]);
});

test("FR-040: forgetWindow clears dedupe so a fresh window toasts", () => {
  const { toaster, toasts } = collectAttentionToaster();
  toaster.handleRuntimeState({ windowId: "w-1", runtimeState: "waiting", windowData: {} });
  toaster.forgetWindow("w-1");
  toaster.handleRuntimeState({ windowId: "w-1", runtimeState: "waiting", windowData: {} });
  assert.equal(toasts.length, 2);
});

test("FR-040: missing windowId yields no toast", () => {
  const { toaster, toasts } = collectAttentionToaster();
  const notice = toaster.handleRuntimeState({ windowId: "", runtimeState: "waiting" });
  assert.equal(notice, null);
  assert.equal(toasts.length, 0);
});


test("typed inputs retain source authority and never infer NeedsHuman from Waiting", () => {
  const {notificationForTransition} = notifications;
  assert.equal(typeof notificationForTransition, "function");
  assert.equal(notificationForTransition({source: "runtime", state: "waiting"}), null);
  assert.equal(notificationForTransition({source: "runtime", state: "needs_human"}), null);
  assert.equal(notificationForTransition({source: "monitor", state: "stopped"}), null);
  const notice = notificationForTransition({source: "monitor", state: "needs_human", issueNumber: 4665});
  assert.equal(notice.title, "Needs human");
  assert.match(notice.body, /#4665/);
  assert.doesNotMatch(notice.title, /complete|finish/i);
});

test("Stopped attention says stopped, without claiming work finished", () => {
  const {toaster} = collectAttentionToaster();
  const notice = toaster.handleRuntimeState({windowId: "one", runtimeState: "stopped"});
  assert.equal(notice.title, "Agent stopped");
  assert.doesNotMatch(notice.title, /finish|complete/i);
});
