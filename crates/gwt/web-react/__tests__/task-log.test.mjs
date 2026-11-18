// @vitest-environment linkedom
import test from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import "../__tests__/jsx-loader.mjs";
import { TaskLog } from "../src/surfaces/TaskLog.jsx";
import { logGroupsFromHistoryRows } from "../../web/task-log.js";
import { createUiStateStore } from "../../web/ui-state-store.js";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const minutesAgo = minutes => new Date(Date.now() - minutes * 60_000).toISOString();

const group = (id, extra = {}) => ({
  id,
  key: null,
  category: "information",
  title: "Agent finished a step",
  message: "The step completed and the result was recorded.",
  details: null,
  count: 1,
  latestAt: minutesAgo(8),
  resolved: false,
  activatable: false,
  ...extra,
});

async function mount(state) {
  const { document, window } = parseHTML("<!doctype html><html><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  const model = createUiStateStore(state);
  const calls = { viewed: [] };
  const root = createRoot(document.body);
  await act(async () =>
    root.render(
      React.createElement(TaskLog, {
        store: model,
        onViewNotification: entry => calls.viewed.push(entry),
      }),
    ),
  );
  const click = async element => {
    await act(async () => {
      element.click();
    });
  };
  return { document, model, root, click, calls };
}

const cards = document => [...document.querySelectorAll(".task-log__card")];
const buttonsIn = card => [...card.querySelectorAll("button")];

test("empty state says no updates have been grouped yet", async () => {
  const { document } = await mount({ groups: [] });
  assert.match(document.body.textContent, /No updates yet/);
  assert.equal(cards(document).length, 0);
});

test("an unresolved error group is Action required with its count and times", async () => {
  const entry = group(1, {
    category: "action",
    title: "Input failed",
    message: "Your input was not delivered to the window. Reopen the window and enter it again.",
    count: 3,
  });
  const { document } = await mount({ groups: [entry] });
  const [card] = cards(document);
  assert.match(card.textContent, /Action required/);
  assert.match(card.textContent, /Input failed/);
  assert.match(card.textContent, /was not delivered/);
  assert.match(card.textContent, /Repeated 3 times/);
  assert.match(card.textContent, /8 minutes ago/);
  const time = card.querySelector("time");
  assert.ok(time, "the exact timestamp stays attached to the row");
  assert.equal(time.getAttribute("datetime"), entry.latestAt);
});

test("a single occurrence carries no repeat count", async () => {
  const { document } = await mount({ groups: [group(1)] });
  const [card] = cards(document);
  assert.doesNotMatch(card.textContent, /Repeated/);
  assert.match(card.textContent, /Information/);
});

test("the original record text is reachable inside the group details", async () => {
  const entry = group(1, {
    category: "action",
    title: "Input failed",
    message: "Your input was not delivered to the window.",
    details: "owner health: Some(Degraded); owner identity mismatch. Automatic recovery stopped after 10 attempts",
  });
  const { document } = await mount({ groups: [entry] });
  const [card] = cards(document);
  const details = card.querySelector("details");
  assert.ok(details, "the group carries a disclosure for the original record");
  assert.match(details.textContent, /Original record/);
  assert.match(details.textContent, /owner identity mismatch/);
  const outside = [...card.children]
    .filter(node => node.tagName !== "DETAILS")
    .map(node => node.textContent)
    .join(" ");
  assert.doesNotMatch(outside, /owner identity mismatch/, "internal wording stays out of the primary text");
});

test("a completed group is labelled Completed", async () => {
  const { document } = await mount({ groups: [group(1, { category: "completed", resolved: true })] });
  const [card] = cards(document);
  assert.match(card.textContent, /Completed/);
});

test("groups render in the order the history supplies, newest first", async () => {
  const { document } = await mount({
    groups: [group(2, { title: "Newest entry" }), group(1, { title: "Older entry" })],
  });
  const [first, second] = cards(document);
  assert.match(first.textContent, /Newest entry/);
  assert.match(second.textContent, /Older entry/);
});

test("View in Notifications hands the group back to the caller", async () => {
  const entry = group(7, { key: "window-command:agent-1:input" });
  const { document, click, calls } = await mount({ groups: [entry] });
  const [card] = cards(document);
  const view = buttonsIn(card).find(button => button.textContent === "View in Notifications");
  assert.ok(view, "the original record stays reachable in the drawer");
  await click(view);
  assert.equal(calls.viewed.length, 1);
  assert.equal(calls.viewed[0].id, 7);
  assert.equal(calls.viewed[0].key, "window-command:agent-1:input");
});

// --- logGroupsFromHistoryRows: the #5016 history rows are the groups ---

const row = (extra = {}) => ({
  id: 1,
  surface: false,
  key: "",
  identity: "[]",
  kind: "attention",
  level: "info",
  title: "Agent finished a step",
  message: "Done.",
  issueNumber: null,
  details: null,
  latestAt: minutesAgo(8),
  count: 1,
  resolved: false,
  unread: true,
  activatable: false,
  ...extra,
});

test("history rows map to groups without re-grouping or re-counting", () => {
  const groups = logGroupsFromHistoryRows([
    row({ id: 3, title: "Newer", count: 5 }),
    row({ id: 2, title: "Older" }),
  ]);
  assert.deepEqual(groups.map(entry => entry.id), [3, 2]);
  assert.equal(groups[0].count, 5);
  assert.equal(groups[0].title, "Newer");
  assert.equal(groups[0].message, "Done.");
});

test("category mapping follows level and resolution", () => {
  const categoryOf = extra => logGroupsFromHistoryRows([row(extra)])[0].category;
  assert.equal(categoryOf({ level: "error" }), "action");
  assert.equal(categoryOf({ level: "warn" }), "action");
  assert.equal(categoryOf({ level: "success" }), "completed");
  assert.equal(categoryOf({ level: "done" }), "completed");
  assert.equal(categoryOf({ level: "error", resolved: true }), "completed");
  assert.equal(categoryOf({ level: "info" }), "information");
  assert.equal(categoryOf({ level: "neutral" }), "information");
});

test("issue numbers join the title and keyed rows keep their key", () => {
  const [entry] = logGroupsFromHistoryRows([
    row({ issueNumber: 5016, surface: true, key: "window-command:a:stop", details: "raw text" }),
  ]);
  assert.equal(entry.title, "Agent finished a step #5016");
  assert.equal(entry.key, "window-command:a:stop");
  assert.equal(entry.details, "raw text");
});

test("no rows produce no groups", () => {
  assert.deepEqual(logGroupsFromHistoryRows([]), []);
  assert.deepEqual(logGroupsFromHistoryRows(null), []);
  assert.deepEqual(logGroupsFromHistoryRows(undefined), []);
});
