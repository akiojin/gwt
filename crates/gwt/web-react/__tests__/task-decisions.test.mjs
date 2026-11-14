// @vitest-environment linkedom
import test from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import "../__tests__/jsx-loader.mjs";
import { TaskDecisions } from "../src/surfaces/TaskDecisions.jsx";
import { createUiStateStore } from "../../web/ui-state-store.js";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const minutesAgo = minutes => new Date(Date.now() - minutes * 60_000).toISOString();

const packetItem = (id, extra = {}) => ({
  id,
  question: "Which recovery message should the agent use?",
  result: "The launch error task is waiting for a wording choice.",
  impact: "The task stays stopped until you choose.",
  owner: "Noah",
  recommendation: "Use the shorter message",
  reason: "It names the next step without internal terms.",
  options: ["Use the shorter message", "Keep the detailed message"],
  detailText: null,
  createdAt: minutesAgo(8),
  ...extra,
});

const plainItem = (id, extra = {}) => ({
  id,
  question: "Approve the release note wording?",
  result: null,
  impact: null,
  owner: null,
  recommendation: null,
  reason: null,
  options: null,
  detailText: "Approve the release note wording?\nThe notes are ready in the draft.",
  createdAt: minutesAgo(65),
  ...extra,
});

const feed = (decisions, extra = {}) => ({
  loaded: true,
  error: null,
  decisions,
  sending: false,
  pendingId: null,
  sendError: null,
  answered: {},
  ...extra,
});

async function mount(state) {
  const { document, window } = parseHTML("<!doctype html><html><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  const model = createUiStateStore(state);
  const calls = { answers: [], replies: [] };
  const root = createRoot(document.body);
  await act(async () =>
    root.render(
      React.createElement(TaskDecisions, {
        store: model,
        onAnswerDecision: (item, optionIndex) => calls.answers.push([item.id, optionIndex]),
        onReplyInChat: item => calls.replies.push(item.id),
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

const cards = document => [...document.querySelectorAll(".task-decisions__card")];
const buttonsIn = card => [...card.querySelectorAll("button")];
const optionButtons = card =>
  buttonsIn(card).filter(button => button.classList.contains("task-decisions__option"));

function recordFocus(document) {
  const focused = [];
  for (const element of document.querySelectorAll("button, h1")) {
    element.focus = () => focused.push(element);
  }
  return focused;
}

test("loading state names decisions without inventing a zero count", async () => {
  const { document } = await mount(feed([], { loaded: false }));
  assert.match(document.body.textContent, /Loading decisions/);
  assert.equal(cards(document).length, 0);
});

test("empty state says no decisions are waiting", async () => {
  const { document } = await mount(feed([]));
  assert.match(document.body.textContent, /No decisions waiting/);
  assert.equal(cards(document).length, 0);
});

test("load failure is shown instead of an empty list", async () => {
  const { document } = await mount(
    feed([], { error: "Request tasks are unavailable." }),
  );
  assert.match(document.body.textContent, /Unable to refresh decisions/);
  assert.match(document.body.textContent, /Request tasks are unavailable\./);
});

test("load failure keeps the last known decisions visible as stale", async () => {
  const item = packetItem("d1");
  const { document } = await mount(feed([item], { error: "Connection lost." }));
  assert.equal(cards(document).length, 1);
  assert.match(document.body.textContent, /Connection lost\./);
});

test("typed decision packet renders question, impact, recommendation and numbered options", async () => {
  const { document } = await mount(feed([packetItem("d1")]));
  const [card] = cards(document);
  assert.match(card.textContent, /Which recovery message should the agent use\?/);
  assert.match(card.textContent, /The task stays stopped until you choose\./);
  assert.match(card.textContent, /Recommended: Use the shorter message/);
  assert.match(card.textContent, /It names the next step without internal terms\./);
  assert.match(card.textContent, /Noah · 8 minutes ago/);
  const options = optionButtons(card);
  assert.equal(options.length, 2);
  assert.equal(options[0].textContent, "1. Use the shorter message");
  assert.equal(options[1].textContent, "2. Keep the detailed message");
  assert.ok(options[0].classList.contains("task-decisions__option--recommended"));
  assert.match(document.body.textContent, /1 decision/);
});

test("only decision items handed to the surface are listed, with the total count", async () => {
  const { document } = await mount(feed([packetItem("d1"), plainItem("d2")]));
  assert.equal(cards(document).length, 2);
  assert.match(document.body.textContent, /2 decisions/);
});

test("choosing an option reports the decision and the option index once", async () => {
  const { document, click, calls } = await mount(feed([packetItem("d1")]));
  const [, second] = optionButtons(cards(document)[0]);
  await click(second);
  assert.deepEqual(calls.answers, [["d1", 1]]);
});

test("while a send is in flight every option is disabled against double submission", async () => {
  const item = packetItem("d1");
  const { document, settle, click, calls } = await mount(feed([item]));
  const [first] = optionButtons(cards(document)[0]);
  await click(first);
  await settle(feed([item], { sending: true, pendingId: "d1" }));
  const card = cards(document)[0];
  for (const button of optionButtons(card)) assert.equal(button.disabled, true);
  assert.match(card.textContent, /Sending/);
  optionButtons(card)[1].click();
  assert.deepEqual(calls.answers, [["d1", 0]], "disabled options cannot submit again");
});

test("a failed send keeps the decision actionable and shows the error on its card", async () => {
  const item = packetItem("d1");
  const { document } = await mount(
    feed([item], {
      sendError: { id: "d1", message: "The PM connection is not connected. Reconnect, then send again." },
    }),
  );
  const card = cards(document)[0];
  const alert = card.querySelector('[role="alert"]');
  assert.ok(alert, "the failure is announced on the card");
  assert.match(alert.textContent, /PM connection is not connected/);
  for (const button of optionButtons(card)) assert.equal(button.disabled, false);
});

test("Later keeps the decision unresolved and moves focus to the next decision", async () => {
  const first = packetItem("d1");
  const second = packetItem("d2", { question: "Ship the fix now?" });
  const { document, click, calls } = await mount(feed([first, second]));
  const focused = recordFocus(document);
  const later = buttonsIn(cards(document)[0]).find(button => button.textContent === "Later");
  await click(later);
  assert.deepEqual(calls.answers, [], "Later never sends an answer");
  assert.equal(cards(document).length, 2, "the decision stays in the list");
  assert.equal(focused.at(-1), optionButtons(cards(document)[1])[0],
    "focus advances to the next decision");
});

test("an answered decision stays listed as answer sent and cannot be answered again", async () => {
  const item = packetItem("d1");
  const { document } = await mount(
    feed([item], { answered: { d1: "Keep the detailed message" } }),
  );
  const card = cards(document)[0];
  assert.match(card.textContent, /Answer sent: Keep the detailed message/);
  assert.equal(optionButtons(card).length, 0, "answered decisions offer no options");
  assert.match(document.body.textContent, /1 decision/, "the count still matches the listed items");
});

test("focus moves to the next decision when the focused decision is answered", async () => {
  const first = packetItem("d1");
  const second = packetItem("d2", { question: "Ship the fix now?" });
  const { document, settle, click } = await mount(feed([first, second]));
  const focused = recordFocus(document);
  await click(optionButtons(cards(document)[0])[0]);
  await settle(feed([first, second], { answered: { d1: "Use the shorter message" } }));
  assert.equal(focused.at(-1), optionButtons(cards(document)[1])[0]);
});

test("focus moves to the heading when the last waiting decision is answered", async () => {
  const item = packetItem("d1");
  const { document, settle, click } = await mount(feed([item]));
  const focused = recordFocus(document);
  await click(optionButtons(cards(document)[0])[0]);
  await settle(feed([item], { answered: { d1: "Use the shorter message" } }));
  assert.equal(focused.at(-1), document.querySelector(".task-decisions h1"));
});

test("a decision without a typed packet offers the chat reply path instead of options", async () => {
  const { document, click, calls } = await mount(feed([plainItem("d2")]));
  const card = cards(document)[0];
  assert.equal(optionButtons(card).length, 0);
  assert.match(card.textContent, /The notes are ready in the draft\./);
  const reply = buttonsIn(card).find(button => button.textContent === "Reply in chat");
  assert.ok(reply, "the existing reply path is offered");
  await click(reply);
  assert.deepEqual(calls.replies, ["d2"]);
});

test("a typed packet also offers a free-form reply through the chat path", async () => {
  const { document, click, calls } = await mount(feed([packetItem("d1")]));
  const other = buttonsIn(cards(document)[0]).find(button =>
    button.textContent.includes("Reply in chat"));
  assert.ok(other, "an Other path exists next to the numbered options");
  await click(other);
  assert.deepEqual(calls.replies, ["d1"]);
  assert.deepEqual(calls.answers, [], "the free-form path never picks an option");
});
