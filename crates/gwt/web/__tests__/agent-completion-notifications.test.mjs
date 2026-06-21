import assert from "node:assert/strict";
import test from "node:test";

import { parseHTML } from "linkedom";

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

test("notifier emits a quiet turn-complete notice only after a long hidden run returns idle", () => {
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

  assert.equal(notice.kind, "turn_complete");
  assert.equal(notice.projectId, "tab-1");
  assert.equal(notice.windowId, "agent-1");
  assert.match(notice.title, /Turn complete/);
  assert.match(notice.body, /Codex/);
  assert.match(notice.body, /Repo One/);
  assert.deepEqual(toasts, [notice]);
  assert.deepEqual(desktop, [notice]);
  assert.deepEqual(unread, ["tab-1"]);
});

test("notifier suppresses short runs, focused windows, and default desktop permission", () => {
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
      runtimeState: "idle",
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
      runtimeState: "idle",
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
