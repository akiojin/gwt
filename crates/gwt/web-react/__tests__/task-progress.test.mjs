// @vitest-environment linkedom
import test from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import "../__tests__/jsx-loader.mjs";
import { TaskProgress } from "../src/surfaces/TaskProgress.jsx";
import { createUiStateStore } from "../../web/ui-state-store.js";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const minutesAgo = minutes => new Date(Date.now() - minutes * 60_000).toISOString();

const task = (key, extra = {}) => ({
  key,
  parent_key: null,
  title: `Task ${key}`,
  status: "in_progress",
  assignee: "Maya",
  wait_reason: null,
  next_action: "Run the verification pass",
  review_required: false,
  source_key: null,
  child_progress: null,
  criteria_preview: null,
  ...extra,
});

const feed = (tasks, extra = {}) => ({
  loaded: true,
  overview: { state: "ready", revision: 4, total: tasks.length, pending_intake: 0, tasks },
  error: null,
  activeWindows: null,
  ...extra,
});

async function mount(state) {
  const { document, window } = parseHTML("<!doctype html><html><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  const model = createUiStateStore(state);
  const calls = { created: 0, reviews: 0, details: [], allTasks: 0 };
  const root = createRoot(document.body);
  await act(async () =>
    root.render(
      React.createElement(TaskProgress, {
        store: model,
        onCreateTask: () => { calls.created += 1; },
        onReviewDecision: () => { calls.reviews += 1; },
        onViewTaskDetails: key => calls.details.push(key),
        onViewAllTasks: () => { calls.allTasks += 1; },
      }),
    ),
  );
  const settle = async next => {
    await act(async () => {
      model.update(() => next);
    });
  };
  const click = async element => {
    await act(async () => {
      element.click();
    });
  };
  return { document, model, root, settle, click, calls };
}

const cards = document => [...document.querySelectorAll(".task-card")];
const buttonsIn = card => [...card.querySelectorAll("button")];

test("loading state names progress without inventing a zero count", async () => {
  const { document } = await mount(feed([], { loaded: false, overview: null }));
  assert.match(document.body.textContent, /Loading progress/);
  assert.equal(cards(document).length, 0);
});

test("absent overview says no request tasks have been recorded yet", async () => {
  const { document } = await mount(feed([], { overview: { state: "absent" } }));
  assert.match(document.body.textContent, /No request tasks have been recorded yet/);
  assert.equal(cards(document).length, 0);
});

test("load failure without an overview is a typed error with a next step", async () => {
  const { document } = await mount(
    feed([], { overview: null, error: "Request tasks are unavailable." }),
  );
  assert.match(document.body.textContent, /Unable to refresh progress/);
  assert.match(document.body.textContent, /Request tasks are unavailable\./);
  assert.match(document.body.textContent, /reopen the PM window/i);
  assert.equal(cards(document).length, 0);
});

test("load failure keeps the last confirmed progress visible as stale", async () => {
  const { document } = await mount(feed([task("one")], { error: "Connection lost." }));
  assert.equal(cards(document).length, 1);
  assert.match(document.body.textContent, /Connection lost\./);
  assert.match(document.body.textContent, /last confirmed progress/);
});

test("card leads with purpose, assignee and current stage", async () => {
  const { document } = await mount(feed([task("one")]));
  const [card] = cards(document);
  assert.match(card.textContent, /Task one/);
  assert.match(card.textContent, /Maya/);
  assert.match(card.textContent, /Run the verification pass/);
  assert.match(card.textContent, /In progress/);
});

test("a waiting card shows the blocker as its stage", async () => {
  const waiting = task("two", {
    status: "waiting",
    assignee: "Noah",
    wait_reason: "a wording choice",
    next_action: null,
  });
  const { document } = await mount(feed([waiting]));
  const [card] = cards(document);
  assert.match(card.textContent, /Waiting/);
  assert.match(card.textContent, /a wording choice/);
  assert.match(card.textContent, /Needs your decision/);
});

test("counts come from the snapshot totals, not the displayed rows", async () => {
  const state = feed([task("one"), task("two")]);
  state.overview.total = 57;
  state.overview.pending_intake = 3;
  const { document } = await mount(state);
  assert.equal(cards(document).length, 2);
  assert.match(document.body.textContent, /57 tasks/);
  assert.match(document.body.textContent, /3 awaiting classification/);
  assert.match(document.body.textContent, /Showing 2 of 57/);
});

test("metrics the snapshot does not provide are not rendered at all", async () => {
  const { document } = await mount(feed([task("one")]));
  const [card] = cards(document);
  assert.doesNotMatch(card.textContent, /token|cost|\$/i);
  assert.equal(card.querySelector(".task-card__metric"), null);
});

test("the internal task key is reachable only inside the card details", async () => {
  const withCriteria = task("opaque-key-123", {
    title: "Ship the recovery flow",
    criteria_preview: {
      purpose: "Ship the recovery flow",
      completion_conditions: ["The flow opens"],
      completion_condition_total: 1,
    },
  });
  const { document } = await mount(feed([withCriteria]));
  const [card] = cards(document);
  const outside = [...card.children]
    .filter(node => node.tagName !== "DETAILS")
    .map(node => node.textContent)
    .join(" ");
  assert.doesNotMatch(outside, /opaque-key-123/);
  const details = card.querySelector("details");
  assert.ok(details, "the card carries a details disclosure");
  assert.match(details.textContent, /Reference: opaque-key-123/);
  assert.match(details.textContent, /Ship the recovery flow/);
  assert.match(details.textContent, /The flow opens/);
});

test("child progress and parentage are shown without merging tasks", async () => {
  const parent = task("parent");
  const child = task("child", {
    parent_key: "parent",
    child_progress: { total: 3, completed: 1, cancelled: 1 },
  });
  const { document } = await mount(feed([parent, child]));
  const [, childCard] = cards(document);
  assert.match(childCard.textContent, /Subtasks: 1 of 3 completed/);
  assert.match(childCard.textContent, /1 cancelled/);
  const details = childCard.querySelector("details");
  assert.match(details.textContent, /Part of: Task parent/);
});

test("review actions route to Decide and to the task details", async () => {
  const waiting = task("two", { status: "waiting", wait_reason: "a wording choice" });
  const { document, click, calls } = await mount(feed([waiting]));
  const [card] = cards(document);
  const review = buttonsIn(card).find(button => button.textContent === "Review decision");
  assert.ok(review, "a decision task offers the Decide route");
  await click(review);
  assert.equal(calls.reviews, 1);
  const detailsButton = buttonsIn(card).find(button => button.textContent === "View task details");
  assert.ok(detailsButton, "the existing task detail read is offered");
  await click(detailsButton);
  assert.deepEqual(calls.details, ["two"]);
});

test("pending tasks are grouped under a Queued heading after active work", async () => {
  const active = task("one");
  const queued = task("nine", { status: "pending", assignee: null, next_action: null });
  const { document } = await mount(feed([active, queued]));
  const heading = document.querySelector(".task-progress__section");
  assert.ok(heading, "queued work has its own heading");
  assert.match(heading.textContent, /Queued/);
  const [activeCard, queuedCard] = cards(document);
  assert.ok(
    heading.compareDocumentPosition(activeCard) & 2, // active card precedes the heading
    "the active card comes before the Queued heading",
  );
  assert.ok(
    heading.compareDocumentPosition(queuedCard) & 4, // queued card follows the heading
    "the queued card comes after the Queued heading",
  );
  assert.match(queuedCard.textContent, /Unassigned/);
  assert.match(queuedCard.textContent, /Queued/);
});

test("completed tasks leave the card list and point at the full Tasks view", async () => {
  const done = task("done", { status: "completed" });
  const { document, click, calls } = await mount(feed([task("one"), done]));
  assert.equal(cards(document).length, 1);
  assert.match(document.body.textContent, /1 completed or cancelled task/);
  const all = [...document.querySelectorAll("button")].find(button => button.textContent === "View all tasks");
  assert.ok(all, "the full list stays reachable");
  await click(all);
  assert.equal(calls.allTasks, 1);
});

test("live-fact agent windows render in a separate Other work frame", async () => {
  const state = feed([task("one")], {
    activeWindows: [{ issueNumber: 42, title: "Repair the login redirect", lastActivityAt: minutesAgo(5) }],
  });
  const { document } = await mount(state);
  const frame = document.querySelector(".task-progress__other");
  assert.ok(frame, "unlinked work is framed separately");
  assert.match(frame.textContent, /Other active work/);
  assert.match(frame.textContent, /separate from the request tasks/i);
  assert.match(frame.textContent, /#42 Repair the login redirect/);
  assert.match(frame.textContent, /Last activity 5 minutes ago/);
  assert.equal(frame.querySelector(".task-card"), null, "live facts never become task cards");
  assert.equal(cards(document).length, 1);
});

test("the Other work frame is omitted when live facts are unavailable or empty", async () => {
  const unavailable = await mount(feed([task("one")], { activeWindows: null }));
  assert.equal(unavailable.document.querySelector(".task-progress__other"), null);
  const empty = await mount(feed([task("one")], { activeWindows: [] }));
  assert.equal(empty.document.querySelector(".task-progress__other"), null);
});

test("Create task routes through the caller", async () => {
  const { document, click, calls } = await mount(feed([]));
  const create = [...document.querySelectorAll("button")].find(button => button.textContent === "+ Create task");
  assert.ok(create, "the create action is offered like Home");
  await click(create);
  assert.equal(calls.created, 1);
});
