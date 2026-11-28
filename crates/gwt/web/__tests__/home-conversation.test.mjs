/* Issue #7639 (SPEC #7007 T-021) — Home, the conversation with the Project
 * Manager, on the page at `/`.
 *
 * AC-1: Home is the PM conversation and shows no project work status or count.
 * AC-2: a decision is answered from the PM's message and the answer goes to
 *       the project it belongs to.
 * AC-3: the path stays whole with no project and with twenty.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";

import { mountHubApp } from "../hub-app.js";

const indexHtml = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const alpha = { id: "tab-a", project_key: "0123456789abcdef", title: "Alpha", kind: "git" };
const beta = { id: "tab-b", project_key: "fedcba9876543210", title: "Beta", kind: "git" };

const minutesAgo = minutes => new Date(Date.now() - minutes * 60_000).toISOString();

// One item of a project's gwtd decision queue (Issue #7742).
const decision = (id, minutes, question, options) => ({
  id, issue: 7742, kind: "product", target: id, question, materials: `${question} is waiting on you.`,
  impact: "Only this project is affected.", options, recommendation: options[0],
  reason: "Nothing else changes by waiting.", status: "open", created_at: minutesAgo(minutes), deferred_seq: null,
});
const project = (source, decisions, extra = {}) => ({
  project_key: source.project_key, title: source.title, pm_available: true,
  waiting_count: decisions.length, decisions, ...extra,
});
// The queue of a project as it stands after an operation.
const queue = items => ({ waiting_count: items.length, items });

// React commits a click's state change on a later task in this DOM.
const settle = async () => {
  await new Promise(resolve => setImmediate(resolve));
  await new Promise(resolve => setImmediate(resolve));
};
const click = async node => {
  node.click();
  await settle();
};

function fixture(search = "") {
  const { document, window } = parseHTML(indexHtml);
  globalThis.document = document;
  globalThis.window = window;
  const sockets = [];
  class FakeSocket {
    static OPEN = 1;
    constructor(url) {
      this.url = url;
      this.readyState = 0;
      this.handlers = {};
      this.sent = [];
      sockets.push(this);
    }
    addEventListener(kind, handler) { this.handlers[kind] = handler; }
    send(raw) { this.sent.push(JSON.parse(raw)); }
    open() { this.readyState = 1; this.handlers.open(); }
    deliver(event) { this.handlers.message({ data: JSON.stringify(event) }); }
  }
  const timers = [];
  const win = Object.assign(window, {
    location: { href: `http://127.0.0.1:4545/${search}`, pathname: "/" },
    WebSocket: FakeSocket,
    setTimeout: callback => { timers.push(callback); return timers.length; },
    clearTimeout() {},
    setInterval: () => 1,
    clearInterval() {},
  });
  const app = mountHubApp({ window: win, document });
  const socket = sockets[0];
  socket.open();
  const reads = () => socket.sent.filter(message => message.kind === "load_home_conversation");
  const sends = () => socket.sent.filter(message => message.kind === "home_pm_send");
  const answers = () => socket.sent.filter(message => message.kind === "home_answer_decision");
  const defers = () => socket.sent.filter(message => message.kind === "home_defer_decision");
  // Deliver the catalog, then answer the decisions read it triggers.
  const load = (projects, conversation, recent = []) => {
    socket.deliver({ kind: "hub_state", hub: { app_version: "9.9.9", projects, recent_projects: recent } });
    socket.deliver({ kind: "home_conversation", request_id: reads().at(-1).request_id, projects: conversation });
  };
  const home = () => document.querySelector("[data-home]");
  const topics = () => [...home().querySelectorAll(".gwt-home__timeline > [data-home-topic]")];
  return { app, document, socket, timers, reads, sends, answers, defers, load, home, topics };
}

const dNew = decision("d-new", 26, "Save and push the unsaved changes?", ["Save and push", "Leave them"]);
const dOld = decision("d-old", 60, "Approve the plan for the refresh?", ["Approve and start", "Ask for changes"]);
const dMid = decision("d-mid", 48, "Fix the mapping and continue the import?", ["Fix and continue", "Stop the import"]);
const twoProjects = [project(alpha, [dNew, dOld]), project(beta, [dMid])];

test("AC-1: Home is the conversation with the PM and carries no project status or count", () => {
  const { document, home, load, topics } = fixture();
  load([alpha, beta], twoProjects);
  assert.equal(document.querySelector("[data-hub]").dataset.hubView, "home");
  assert.equal(home().getAttribute("aria-label"), "Conversation with the Project Manager");
  assert.equal(home().querySelector(".gwt-home__name").textContent, "Project Manager");
  assert.equal(home().querySelector('[role="log"]').getAttribute("aria-label"), "Conversation with the Project Manager");
  // Oldest at the top, newest at the bottom (design item 230).
  assert.deepEqual(topics().map(topic => topic.dataset.homeTopic), [
    `${alpha.project_key}:d-old`, `${beta.project_key}:d-mid`, `${alpha.project_key}:d-new`,
  ]);
  // A decision reads as a PM message: prose, with the project named in its meta line.
  const first = topics()[0];
  assert.match(first.querySelector(".gwt-home__meta").textContent, /Project Manager.*Alpha.*Decision waiting.*Approve the plan for the refresh\?/);
  assert.match(first.textContent, /I recommend: Approve and start\. Nothing else changes by waiting\./);
  assert.match(first.textContent, /Only this project is affected\./);
  assert.equal(first.querySelector("dl, table, h2, h3"), null, "no card structure inside a message");
  // The header's PM entry counts the same decisions Home shows.
  assert.equal(home().querySelector("[data-home-waitjump]").textContent, "3 decisions waiting");
  assert.equal(document.querySelector('.app-head [data-destination="pm"] .app-head__count').textContent, "3");
  // No project list, link, execution status or owner health is part of Home.
  for (const selector of ["a[href^='/p/']", "[data-hub-list]", "[data-project-execution]", "[data-project-health]", "[data-close-project]"]) {
    assert.equal(home().querySelector(selector), null, `${selector} must not appear on Home`);
  }
  assert.doesNotMatch(home().textContent, /\b(working|running|open projects?|issues?|themes?)\b/i);
});

test("AC-2: an option in the PM's message answers the decision of its own project", async () => {
  const { document, home, load, sends, answers, socket, topics } = fixture();
  load([alpha, beta], twoProjects);
  const betaTopic = topics()[1];
  await click(betaTopic.querySelector('[data-home-option="0"]'));
  // Issue #7742: the answer is a queue operation of that project, not a
  // message typed into a PM pane, so it does not need a running PM.
  assert.deepEqual(sends(), []);
  assert.deepEqual(answers(), [{
    kind: "home_answer_decision", project_key: beta.project_key, decision_id: "d-mid",
    answer: "1. Fix and continue", request_id: answers()[0].request_id,
  }]);
  assert.equal(topics()[1].querySelector('[role="status"]').textContent, "Sending…");
  assert.equal(topics()[0].querySelector('[data-home-option="0"]').disabled, true, "one answer is in flight at a time");
  assert.equal(home().querySelector("[data-home-resolved]"), null, "an unconfirmed answer settles nothing");

  socket.deliver({ kind: "decision_result", id: beta.project_key, request_id: answers()[0].request_id,
    decision_id: "d-mid", action: "answer", ok: true, error: null, decisions: queue([]) });
  await settle();
  // The topic moves to the resolved list, above the open topics (design item 250).
  assert.deepEqual(topics().map(topic => topic.dataset.homeTopic), [`${alpha.project_key}:d-old`, `${alpha.project_key}:d-new`]);
  const resolved = home().querySelector("[data-home-resolved]");
  assert.equal(resolved.nextElementSibling.hasAttribute("data-home-greeting"), true);
  assert.match(resolved.querySelector("summary").textContent, /Resolved.*Fix the mapping and continue the import\?.*Fix and continue/);
  assert.equal(home().querySelector("[data-home-waitjump]").textContent, "2 decisions waiting");
  assert.equal(document.querySelector('.app-head [data-destination="pm"] .app-head__count').textContent, "2");
});

test("AC-2: a refused answer keeps the decision open and says why", async () => {
  const { home, load, answers, socket, topics } = fixture();
  load([alpha], [twoProjects[0]]);
  await click(topics()[0].querySelector('[data-home-option="1"]'));
  socket.deliver({ kind: "decision_result", id: alpha.project_key, request_id: answers()[0].request_id,
    decision_id: "d-old", action: "answer", ok: false, error: "decision d-old is no longer open (answered)" });
  await settle();
  assert.equal(topics().length, 2);
  assert.match(topics()[0].querySelector('[role="alert"]').textContent, /no longer open/);
  assert.equal(topics()[0].querySelector('[data-home-option="1"]').disabled, false);
  assert.equal(home().querySelector("[data-home-resolved]"), null);
});

test("an unconfirmed answer is released after its bounded wait", async () => {
  const { load, timers, topics } = fixture();
  load([alpha], [twoProjects[0]]);
  await click(topics()[0].querySelector('[data-home-option="0"]'));
  timers.at(-1)();
  await settle();
  assert.match(topics()[0].querySelector('[role="alert"]').textContent, /Delivery was not confirmed/);
  assert.equal(topics().length, 2);
});

test("AC-4: Decide later moves the decision to the queue's tail and keeps it counted", async () => {
  const { document, home, load, sends, answers, defers, socket, topics } = fixture();
  load([alpha], [twoProjects[0]]);
  await click(topics()[0].querySelector("[data-home-defer]"));
  assert.deepEqual([...sends(), ...answers()], [], "deferring answers nothing");
  assert.deepEqual(defers(), [{ kind: "home_defer_decision", project_key: alpha.project_key,
    decision_id: "d-old", request_id: defers()[0].request_id }]);
  socket.deliver({ kind: "decision_result", id: alpha.project_key, request_id: defers()[0].request_id,
    decision_id: "d-old", action: "defer", ok: true, error: null,
    decisions: queue([dNew, { ...dOld, deferred_seq: 1 }]) });
  await settle();
  assert.equal(topics().length, 2);
  assert.equal(topics()[0].querySelector('[role="status"]').textContent, "Kept waiting. Nothing was decided.");
  assert.equal(home().querySelector("[data-home-waitjump]").textContent, "2 decisions waiting");
  assert.equal(document.querySelector('.app-head [data-destination="pm"] .app-head__count').textContent, "2");
});

test("AC-4: the count is the queues' own waiting count, never a recount of the topics", () => {
  const { document, home, load, topics } = fixture();
  // An agent-registered decision without options keeps the free-form reply.
  const open = { ...decision("d-agent", 5, "Which export format?", []), recommendation: null, reason: null };
  load([alpha, beta], [project(alpha, [open]), project(beta, [], { waiting_count: 0 })]);
  assert.equal(topics().length, 1);
  assert.equal(topics()[0].querySelector("[data-home-option]"), null);
  assert.ok(topics()[0].querySelector("[data-home-reply]"));
  assert.equal(home().querySelector("[data-home-waitjump]").textContent, "1 decision waiting");
  assert.equal(document.querySelector('.app-head [data-destination="pm"] .app-head__count').textContent, "1");
});

test("a reply goes through the one composer, to the topic it names", async () => {
  const { app, home, load, answers, socket, topics } = fixture();
  load([alpha, beta], twoProjects);
  assert.equal(home().querySelectorAll("textarea").length, 1, "one composer, no per-topic reply box");
  await click(topics()[1].querySelector("[data-home-reply]"));
  const chip = home().querySelector("[data-home-replychip]");
  assert.match(chip.textContent, /Replying to.*Fix the mapping and continue the import\?/);
  assert.equal(home().querySelector("[data-home-project]"), null, "a reply already has its project");
  app.home.actions.submit("Stop it until Monday", alpha.project_key);
  assert.deepEqual(answers().map(({ project_key, decision_id, answer }) => ({ project_key, decision_id, answer })), [{
    project_key: beta.project_key, decision_id: "d-mid", answer: "Stop it until Monday",
  }]);
  socket.deliver({ kind: "decision_result", id: beta.project_key, request_id: answers()[0].request_id,
    decision_id: "d-mid", action: "answer", ok: true, error: null, decisions: queue([]) });
  await settle();
  assert.equal(home().querySelector("[data-home-replychip]"), null);
  assert.match(home().querySelector("[data-home-resolved] summary").textContent, /Stop it until Monday/);

  await click(topics()[0].querySelector("[data-home-reply]"));
  await click(home().querySelector(".gwt-home__replychip-clear"));
  assert.equal(home().querySelector("[data-home-replychip]"), null, "the reply target can be dropped at any time");
});

test("a new message is sent to the chosen project and never to a silent pick", async () => {
  const { app, home, load, sends, socket } = fixture();
  load([alpha, beta], twoProjects);
  assert.ok(home().querySelector("[data-home-project]"), "several projects are open, so the project is chosen");
  assert.equal(app.home.actions.submit("Move the nightly update", ""), false);
  assert.deepEqual(sends(), []);
  assert.equal(app.home.actions.submit("Move the nightly update", beta.project_key), true);
  assert.deepEqual(sends().map(({ project_key, text }) => ({ project_key, text })),
    [{ project_key: beta.project_key, text: "Move the nightly update\r" }]);
  socket.deliver({ kind: "pane_send_result", ok: true, window_id: "tab-b::pm", error: null, request_id: sends()[0].request_id });
  await settle();
  const sent = home().querySelector("[data-home-sent]");
  assert.match(sent.textContent, /You.*to Beta.*Move the nightly update.*Delivered to the Project Manager of Beta\./);
});

test("native history replaces each confirmed local send once, including repeated text", async () => {
  const { app, home, load, sends, socket } = fixture();
  const text = "Check the nightly update";
  const native = id => ({ id, role: "user", text });
  const loadHistory = messages => load([alpha], [project(alpha, [], { conversation: { messages } })]);
  const receipts = () => home().querySelectorAll("[data-home-sent]").length;
  loadHistory([native("older")]);
  for (const answer of [text, text]) {
    assert.ok(app.home.actions.submit(answer, alpha.project_key));
    socket.deliver({ kind: "pane_send_result", request_id: sends().at(-1).request_id, ok: true });
  }
  await settle();
  assert.equal(receipts(), 2);
  loadHistory([native("older"), native("first")]);
  await settle();
  assert.equal(receipts(), 1);
  loadHistory([native("older"), native("first")]);
  await settle();
  assert.equal(receipts(), 1);
  loadHistory([native("older"), native("first"), native("second")]);
  await settle();
  assert.equal(receipts(), 0);
  assert.equal(home().querySelectorAll(".gwt-home__msg--me").length, 3);
});

test("a resolved topic is deleted only after a confirmation", async () => {
  const { document, home, load, answers, socket, topics } = fixture();
  load([alpha], [twoProjects[0]]);
  await click(topics()[0].querySelector('[data-home-option="0"]'));
  socket.deliver({ kind: "decision_result", id: alpha.project_key, request_id: answers()[0].request_id,
    decision_id: "d-old", action: "answer", ok: true, error: null, decisions: queue([dNew]) });
  await settle();
  await click(home().querySelector("[data-home-remove]"));
  const dialog = document.querySelector("body > [data-home-remove-dialog].modal-backdrop.open > .modal-shell");
  assert.equal(dialog.getAttribute("role"), "dialog");
  assert.equal(dialog.getAttribute("aria-modal"), "true");
  assert.equal(document.getElementById(dialog.getAttribute("aria-labelledby")).textContent, "Delete this topic?");
  await click(dialog.querySelector("[data-home-remove-cancel]"));
  assert.ok(home().querySelector("[data-home-resolved] details"), "cancel keeps the topic");
  await click(home().querySelector("[data-home-remove]"));
  await click(document.querySelector("[data-home-remove-confirm]"));
  assert.equal(document.querySelector("[data-home-remove-dialog]"), null);
  assert.equal(home().querySelector("[data-home-resolved]"), null);
  assert.equal(topics().length, 1, "a deleted topic does not come back as waiting");
});

test("AC-3: with no project the PM guides to the ways of adding one", async () => {
  const { document, home, load, socket } = fixture();
  load([], []);
  assert.match(home().querySelector("[data-home-greeting]").textContent, /There are no projects yet/);
  assert.deepEqual([...home().querySelectorAll("[data-home-entry]")].map(entry => entry.dataset.homeEntry), ["github", "folder", "clone"]);
  assert.equal(home().querySelector("[data-home-waitjump]").disabled, true);
  assert.equal(home().querySelector("[data-home-send]").disabled, true);
  await click(home().querySelector('[data-home-entry="folder"]'));
  assert.deepEqual(socket.sent.at(-1), { kind: "open_project_dialog" });
  await click(home().querySelector('[data-home-entry="github"]'));
  const modal = document.getElementById("clone-project-modal");
  assert.ok(modal.classList.contains("open"));
  assert.ok(modal.querySelector("#clone-project-search-input"), "Import from GitHub opens the repository search");
});

test("AC-3: with only recent projects the PM points to Projects", async () => {
  const { document, home, load } = fixture();
  load([], [], [{ path: "/Users/me/beta", title: "Beta", kind: "git", project_key: beta.project_key }]);
  assert.match(home().querySelector("[data-home-greeting]").textContent, /No project is open right now/);
  assert.equal(home().querySelector('[data-home-entry="folder"]'), null);
  await click(home().querySelector('[data-home-entry="projects"]'));
  assert.equal(document.querySelector("[data-hub]").dataset.hubView, "projects");
  assert.doesNotMatch(document.body.innerHTML, /\/Users\/me/, "no filesystem path reaches the page");
});

test("AC-3: twenty open projects stay one conversation, and Projects is one step away", async () => {
  const { document, home, load, topics } = fixture();
  const many = Array.from({ length: 20 }, (_, index) => ({
    id: `tab-${index}`, project_key: index.toString(16).padStart(16, "0"), title: `Project ${index + 1}`, kind: "git",
  }));
  load(many, many.map((source, index) => project(source, index === 7 ? [decision("d-1", 5, "Ship it?", ["Ship", "Hold"])] : [])));
  assert.equal(topics().length, 1);
  assert.match(topics()[0].querySelector(".gwt-home__meta").textContent, /Project 8/);
  assert.equal(home().querySelector("[data-home-waitjump]").textContent, "1 decision waiting");
  assert.equal(home().querySelectorAll("[data-home-project] option").length, 21, "every open project can be addressed");
  assert.equal(home().querySelector("a[href^='/p/']"), null, "no project list on Home");
  await click(document.querySelector('.app-head [data-destination="projects"]'));
  assert.equal(document.querySelector("[data-hub]").dataset.hubView, "projects");
  assert.equal(document.querySelectorAll('[data-hub-list="open"] a').length, 20);
  await click(document.querySelector('.app-head [data-destination="pm"]'));
  assert.equal(document.querySelector("[data-hub]").dataset.hubView, "home");
});

test("a late or repeated decisions snapshot never replaces a newer one", () => {
  const { load, reads, socket, topics } = fixture();
  load([alpha], [twoProjects[0]]);
  const stale = reads().at(-1).request_id;
  socket.deliver({ kind: "home_conversation", request_id: stale, projects: [] });
  assert.equal(topics().length, 2);
});

test("a project whose decisions cannot be read is named, not hidden", () => {
  const { home, load } = fixture();
  load([alpha], [project(alpha, [], { pm_available: false, error: "store locked" })]);
  assert.equal(home().querySelector('.gwt-home__note[role="alert"]').textContent, "Decisions of Alpha could not be read: store locked");
});

test("Home attachment controls use Operator tokens and wrap at narrow widths", () => {
  const css = readFileSync(new URL("../styles/app.css", import.meta.url), "utf8");
  const styles = css.split(".gwt-home__attachments {")[1].split(".gwt-home__draft {")[0];
  assert.match(styles, /flex-wrap:\s*wrap/);
  assert.match(styles, /var\(--color-border\)/);
  assert.match(styles, /var\(--color-surface\)/);
  assert.doesNotMatch(styles, /#[\da-f]{3,8}\b|rgba?\(/i);
});

test("Issue #7864 mounts only the most recent 50 articles in native order", async () => {
  const f = fixture();
  f.load([alpha], [project(alpha, [], { conversation: { availability: "ready",
    messages: Array.from({ length: 2500 }, (_, index) => ({
      id: String(index), role: "assistant", text: `Article ${index}`,
    })),
  } })]);
  await settle();
  const articles = [...f.home().querySelectorAll(".gwt-home__timeline > article:not([data-home-greeting])")];
  assert.deepEqual(articles.map(node => node.lastElementChild.textContent),
    Array.from({ length: 50 }, (_, index) => `Article ${2450 + index}`));
  assert.equal(f.home().querySelector(".gwt-home__timeline > details"), null);
});

// Issue #8086 AC-1/AC-2: the native history is the one conversation every
// client reads. A tab that never sent the message — another client, or one
// opened or reloaded later — shows the owner's message between the PM's
// replies, and the sending tab's local receipt yields to the same message.
test("Issue #8086: every tab shows both sides of the PM conversation in order", async () => {
  const text = "Ship the nightly fix";
  const history = [
    { id: "pm-1", role: "assistant", text: "Working on the queue.", timestamp: "2026-11-28T06:00:00.000Z" },
    { id: "owner-1", role: "user", text, timestamp: "2026-11-28T06:01:00.000Z" },
    { id: "pm-2", role: "assistant", text: "Shipping it now.", timestamp: "2026-11-28T06:02:00.000Z" },
  ];
  const conversation = messages => [project(alpha, [], { conversation: { conversation_id: "c", messages } })];
  const timeline = tab => [...tab.home().querySelectorAll(".gwt-home__timeline > article:not([data-home-greeting])")]
    .map(node => node.textContent);

  const sender = fixture();
  sender.load([alpha], conversation(history.slice(0, 1)));
  assert.ok(sender.app.home.actions.submit(text, alpha.project_key));
  sender.socket.deliver({ kind: "pane_send_result", ok: true, request_id: sender.sends()[0].request_id });
  await settle();
  sender.load([alpha], conversation(history));
  await settle();
  assert.equal(sender.home().querySelectorAll("[data-home-sent]").length, 0);
  const sent = timeline(sender);

  const other = fixture();
  other.load([alpha], conversation(history));
  await settle();
  const seen = timeline(other);
  assert.deepEqual(seen, sent, "another tab shows the same conversation as the sender");
  assert.equal(seen.length, 3);
  assert.match(seen[0], /Working on the queue\./);
  assert.match(seen[1], /You.*to Alpha.*Ship the nightly fix/);
  assert.match(seen[2], /Shipping it now\./);
});
