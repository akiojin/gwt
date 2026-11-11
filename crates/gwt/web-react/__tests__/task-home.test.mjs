import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { createElement } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { createUiStateStore } from "../../web/ui-state-store.js";
import { TaskHome } from "../src/surfaces/TaskHome.jsx";

// The development react-dom build reads `window` for event timing.
globalThis.window ??= parseHTML("<html></html>").window;

function mount(state, props = {}) {
  const { document } = parseHTML('<html><body><div id="root"></div></body></html>');
  const container = document.getElementById("root");
  const store = createUiStateStore(state);
  const root = createRoot(container);
  flushSync(() => root.render(createElement(TaskHome, { store, ...props })));
  return { container, store, root };
}

const readyState = {
  overview: {
    state: "ready",
    total: 3,
    pending_intake: 1,
    tasks: [
      { key: "t1", title: "Improve keyboard navigation", status: "in_progress", assignee: "Maya", next_action: "Testing the keyboard shortcuts" },
      { key: "t2", title: "Make launch errors easier to understand", status: "waiting", assignee: "Noah", wait_reason: "Waiting for a wording choice", review_required: true },
      { key: "t3", title: "Tidy the settings page", status: "pending", assignee: null },
    ],
  },
  decisionsWaiting: 2,
  completedToday: 4,
};

test("home leads with the decisions waiting and a plain-language summary", () => {
  const { container, root } = mount(readyState);
  const text = container.textContent;
  assert.match(text, /2 decisions need your attention/);
  assert.match(text, /Create task/);
  assert.match(text, /Tasks in progress/);
  assert.match(text, /Tasks queued/);
  assert.match(text, /Completed today/);
  root.unmount();
});

test("counts come from the overview totals, never from the visible rows", () => {
  const { container, root } = mount(readyState);
  const counts = container.querySelector(".task-home__counts");
  assert.ok(counts, "counts block exists");
  // 2 unfinished non-queued rows are visible, but the summary must say
  // 2 in progress (in_progress + waiting), 2 queued (pending + intake).
  assert.match(counts.textContent, /2\s*Tasks in progress/);
  assert.match(counts.textContent, /2\s*Tasks queued/);
  assert.match(counts.textContent, /4\s*Completed today/);
  root.unmount();
});

test("recent cards show purpose, owner, stage and the decision link", () => {
  const { container, root } = mount(readyState);
  const text = container.textContent;
  assert.match(text, /Improve keyboard navigation/);
  assert.match(text, /Maya/);
  assert.match(text, /Testing the keyboard shortcuts/);
  assert.match(text, /Needs your decision/);
  assert.match(text, /Review decision/);
  root.unmount();
});

test("unknown state is never rendered as zero decisions", () => {
  const { container, root } = mount({ overview: null, decisionsWaiting: null, completedToday: null });
  const text = container.textContent;
  assert.match(text, /Unable to refresh/);
  assert.doesNotMatch(text, /No decisions waiting/);
  assert.doesNotMatch(text, /0\s*decisions/);
  root.unmount();
});

test("no decisions waiting is stated plainly when the count is known zero", () => {
  const { container, root } = mount({ ...readyState, decisionsWaiting: 0 });
  assert.match(container.textContent, /No decisions waiting/);
  root.unmount();
});

test("absent overview says no tasks recorded yet", () => {
  const { container, root } = mount({ overview: { state: "absent", total: 0, pending_intake: 0, tasks: [] }, decisionsWaiting: 0, completedToday: 0 });
  assert.match(container.textContent, /No request tasks have been recorded yet/);
  root.unmount();
});

test("stale overview keeps the last confirmed summary and marks it stale", () => {
  const { container, root } = mount({ ...readyState, stale: true });
  const text = container.textContent;
  assert.match(text, /Showing the last confirmed summary/);
  assert.match(text, /2 decisions need your attention/);
  root.unmount();
});

test("create task and decision actions reach the host callbacks", () => {
  const calls = [];
  const { container, root } = mount(readyState, {
    onCreateTask: () => calls.push("create"),
    onReviewDecisions: () => calls.push("decide"),
  });
  const buttons = [...container.querySelectorAll("button")];
  buttons.find(button => button.textContent.includes("Create task")).click();
  buttons.find(button => button.textContent.includes("Review 2 decisions")).click();
  assert.deepEqual(calls, ["create", "decide"]);
  root.unmount();
});

test("store updates re-render the home summary on the next frame", async () => {
  const { container, store, root } = mount({ ...readyState, decisionsWaiting: 0 });
  assert.match(container.textContent, /No decisions waiting/);
  store.update(state => ({ ...state, decisionsWaiting: 1 }));
  // The surface subscribes with frame batching (frame: true), like the
  // existing WorkspaceStateNotice: wait for the scheduled frame to flush.
  await new Promise(resolve => setTimeout(resolve, 250));
  assert.match(container.textContent, /1 decision needs your attention/);
  root.unmount();
});
