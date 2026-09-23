import assert from "node:assert/strict";
import test from "node:test";

import { parseHTML } from "linkedom";
import * as notifications from "../agent-completion-notifications.js";

import {
  createAgentCompletionNotifier,
  createAgentAttentionToaster,
} from "../agent-completion-notifications.js";

function setupDocument({ hidden = true, focused = false } = {}) {
  const { document } = parseHTML("<main></main>");
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    get: () => (hidden ? "hidden" : "visible"),
  });
  document.hasFocus = () => focused;
  return document;
}

function makeWindow(overrides = {}) {
  return {
    id: "agent-1",
    preset: "codex",
    title: "Codex",
    dynamic_title: "Codex",
    ...overrides,
  };
}

function makeProject(overrides = {}) {
  return {
    id: "tab-1",
    title: "Repo One",
    project_root: "/repo/one",
    ...overrides,
  };
}

test("Idle after a long run never implies completion", () => {
  let now = 1_000;
  const toasts = [];
  const desktop = [];
  const unread = [];
  const notifier = createAgentCompletionNotifier({
    document: setupDocument({ hidden: true, focused: false }),
    now: () => now,
    minRunningMs: 300_000,
    getDesktopNotificationPermission: () => "granted",
    showToast: (notice) => toasts.push(notice),
    showDesktopNotification: (notice) => desktop.push(notice),
    onProjectUnread: (projectId) => unread.push(projectId),
  });

  assert.equal(
    notifier.handleRuntimeState({
      windowId: "agent-1",
      runtimeState: "running",
      windowData: makeWindow(),
      projectTab: makeProject(),
    }),
    null,
  );

  now += 300_001;
  const notice = notifier.handleRuntimeState({
    windowId: "agent-1",
    runtimeState: "idle",
    windowData: makeWindow(),
    projectTab: makeProject(),
  });

  assert.equal(notice, null);
  assert.deepEqual(toasts, []);
  assert.deepEqual(desktop, []);
  assert.deepEqual(unread, []);
});

test("T-603: notifier does not treat sustained running -> waiting as turn completion", () => {
  let now = 1_000;
  const toasts = [];
  const desktop = [];
  const unread = [];
  const notifier = createAgentCompletionNotifier({
    document: setupDocument({ hidden: true, focused: false }),
    now: () => now,
    minRunningMs: 300_000,
    getDesktopNotificationPermission: () => "granted",
    showToast: (notice) => toasts.push(notice),
    showDesktopNotification: (notice) => desktop.push(notice),
    onProjectUnread: (projectId) => unread.push(projectId),
  });

  notifier.handleRuntimeState({
    windowId: "agent-1",
    runtimeState: "running",
    windowData: makeWindow(),
    projectTab: makeProject(),
  });

  now += 300_001;
  const notice = notifier.handleRuntimeState({
    windowId: "agent-1",
    runtimeState: "waiting",
    windowData: makeWindow(),
    projectTab: makeProject(),
  });

  assert.equal(notice, null);
  assert.deepEqual(toasts, []);
  assert.deepEqual(desktop, []);
  assert.deepEqual(unread, []);
});

test("notifier suppresses short runs and focused windows", () => {
  let now = 10_000;
  const toasts = [];
  const desktop = [];
  const notifier = createAgentCompletionNotifier({
    document: setupDocument({ hidden: false, focused: true }),
    now: () => now,
    minRunningMs: 300_000,
    getDesktopNotificationPermission: () => "default",
    showToast: (notice) => toasts.push(notice),
    showDesktopNotification: (notice) => desktop.push(notice),
  });

  notifier.handleRuntimeState({
    windowId: "agent-1",
    runtimeState: "running",
    windowData: makeWindow(),
    projectTab: makeProject(),
  });

  now += 299_999;
  assert.equal(
    notifier.handleRuntimeState({
      windowId: "agent-1",
      runtimeState: "stopped",
      windowData: makeWindow(),
      projectTab: makeProject(),
    }),
    null,
  );

  notifier.handleRuntimeState({
    windowId: "agent-1",
    runtimeState: "running",
    windowData: makeWindow(),
    projectTab: makeProject(),
  });

  now += 300_001;
  assert.equal(
    notifier.handleRuntimeState({
      windowId: "agent-1",
      runtimeState: "stopped",
      windowData: makeWindow(),
      projectTab: makeProject(),
    }),
    null,
  );
  assert.deepEqual(toasts, []);
  assert.deepEqual(desktop, []);
});

test("notifier reports stopped and error transitions as separate categories", () => {
  let now = 0;
  const notices = [];
  const notifier = createAgentCompletionNotifier({
    document: setupDocument({ hidden: true, focused: false }),
    now: () => now,
    minRunningMs: 300_000,
    getDesktopNotificationPermission: () => "denied",
    showToast: (notice) => notices.push(notice),
    showDesktopNotification: () => {
      throw new Error("desktop notifications must not be attempted when denied");
    },
  });

  notifier.handleRuntimeState({
    windowId: "agent-1",
    runtimeState: "running",
    windowData: makeWindow(),
    projectTab: makeProject(),
  });
  now += 301_000;
  assert.equal(
    notifier.handleRuntimeState({
      windowId: "agent-1",
      runtimeState: "stopped",
      windowData: makeWindow(),
      projectTab: makeProject(),
    }).kind,
    "agent_stopped",
  );

  notifier.handleRuntimeState({
    windowId: "agent-2",
    runtimeState: "running",
    windowData: makeWindow({ id: "agent-2", dynamic_title: "Claude" }),
    projectTab: makeProject({ id: "tab-2", title: "Repo Two" }),
  });
  now += 301_000;
  assert.equal(
    notifier.handleRuntimeState({
      windowId: "agent-2",
      runtimeState: "error",
      windowData: makeWindow({ id: "agent-2", dynamic_title: "Claude" }),
      projectTab: makeProject({ id: "tab-2", title: "Repo Two" }),
    }).kind,
    "agent_error",
  );

  assert.deepEqual(
    notices.map((notice) => notice.kind),
    ["agent_stopped", "agent_error"],
  );
});

test("notifier includes error detail in agent error notices", () => {
  let now = 0;
  const notices = [];
  const notifier = createAgentCompletionNotifier({
    document: setupDocument({ hidden: true, focused: false }),
    now: () => now,
    minRunningMs: 300_000,
    getDesktopNotificationPermission: () => "denied",
    showToast: (notice) => notices.push(notice),
  });

  notifier.handleRuntimeState({
    windowId: "agent-1",
    runtimeState: "running",
    windowData: makeWindow(),
    projectTab: makeProject(),
  });
  now += 301_000;
  const notice = notifier.handleRuntimeState({
    windowId: "agent-1",
    runtimeState: "error",
    windowData: makeWindow(),
    projectTab: makeProject(),
    statusDetail: "Stop-block hit an error",
  });

  assert.equal(notice.kind, "agent_error");
  assert.match(notice.body, /Stop-block hit an error/);
  assert.deepEqual(notices, [notice]);
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

test("five minutes means consecutive Running; duplicates preserve time and resets discard it", () => {
  let now = 0;
  const notices = [];
  const desktop = [];
  let permission = "default";
  const notifier = createAgentCompletionNotifier({now: () => now, isAttentionAway: () => true,
    getDesktopNotificationPermission: () => permission, showToast: n => notices.push(n),
    showDesktopNotification: n => desktop.push(n)});
  const send = runtimeState => notifier.handleRuntimeState({windowId: "one", runtimeState});
  send("running"); now = 299999; send("running"); assert.equal(send("stopped"), null);
  send("running"); now += 300000; send("running");
  assert.equal(send("stopped").kind, "agent_stopped");
  assert.equal(send("stopped"), null);
  send("running"); now += 300000; send("starting"); send("running");
  assert.equal(send("error"), null);
  send("running"); now += 300000; notifier.reset();
  assert.equal(send("stopped"), null);
  assert.equal(notices.length, 1);
  assert.deepEqual(desktop, []);
  permission = "granted"; send("running"); now += 300000;
  const grantedNotice = send("stopped");
  assert.deepEqual(desktop, [grantedNotice]);
});

test("Stopped attention says stopped, without claiming work finished", () => {
  const {toaster} = collectAttentionToaster();
  const notice = toaster.handleRuntimeState({windowId: "one", runtimeState: "stopped"});
  assert.equal(notice.title, "Agent stopped");
  assert.doesNotMatch(notice.title, /finish|complete/i);
});
