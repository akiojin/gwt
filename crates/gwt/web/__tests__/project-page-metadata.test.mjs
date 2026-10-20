import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";
import { createProjectPageMetadata } from "../project-page-metadata.js";

function setup(project = true, permissions = null) {
  const { document, window: domWindow } = parseHTML('<html><head><title>gwt — Operator</title><link rel="icon" href="data:,"></head><body></body></html>');
  const window = {
    Event: domWindow.Event, MutationObserver: domWindow.MutationObserver,
    addEventListener: domWindow.addEventListener.bind(domWindow),
    dispatchEvent: domWindow.dispatchEvent.bind(domWindow),
    navigator: { permissions },
  };
  window.getComputedStyle = () => ({ getPropertyValue: (token) => token === "--color-state-blocked" ? "red" : "blue" });
  const sent = [];
  window.Notification = { permission: "default", requestPermission() { throw new Error("must not prompt"); } };
  document.hasFocus = () => false;
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
  return { document, window, sent, controller: createProjectPageMetadata({ document, window, project, send: (event) => sent.push(event) }) };
}
test("server aggregate renders exact project counts, unread, and semantic favicon state", () => {
  const { document, controller } = setup();
  controller.setProjectName("Repo A");
  controller.update({ running_count: 2, block_count: 1, error_count: 1, unread: true, revision: 5 });
  assert.equal(document.title, "● Repo A — RUN 2 · BLOCK 1 — gwt");
  const icon = document.querySelector('link[rel="icon"]');
  assert.equal(icon.dataset.state, "error");
  assert.equal(icon.dataset.unread, "true");
  assert.match(decodeURIComponent(icon.href), /red/);
  controller.update({ running_count: 0, block_count: 1, error_count: 0, unread: false, revision: 6 });
  assert.equal(document.title, "Repo A — RUN 0 · BLOCK 1 — gwt");
  assert.equal(icon.dataset.state, "blocked");
  controller.update({ running_count: 1, block_count: 0, unread: false, revision: 7 });
  assert.equal(icon.dataset.state, "running");
  controller.update({ running_count: 0, block_count: 0, unread: false, revision: 8 });
  assert.equal(icon.dataset.state, "idle");
});
test("Hub metadata remains fixed despite aggregate and name input", () => {
  const { document, sent, controller } = setup(false);
  controller.setProjectName("Repo A");
  controller.update({ running_count: 7, unread: true });
  controller.reportAttention();
  assert.equal(document.title, "gwt — Hub");
  assert.deepEqual(sent, []);
  assert.equal(document.querySelector('link[rel="icon"]').getAttribute("href"), "data:,");
});
test("metadata uses Operator tokens without literal palette colors", () => {
  const source = readFileSync(new URL("../project-page-metadata.js", import.meta.url), "utf8");
  assert.doesNotMatch(source, /#[0-9a-f]{3,8}\b|rgba?\(/i);
  assert.match(source, /--color-state-active/);
});

test("only visible focused clients acknowledge server unread, never clear it locally", () => {
  const { document, window, sent, controller } = setup();
  controller.update({ running_count: 2, unread: true, revision: 9 });
  assert.deepEqual(sent, []);
  document.hasFocus = () => true;
  window.dispatchEvent(new window.Event("focus"));
  const acknowledgements = () => sent.filter((event) => event.kind === "project_aggregate_ack");
  assert.deepEqual(acknowledgements(), [{ kind: "project_aggregate_ack", revision: 9, visible: true, focused: true }]);
  assert.match(document.title, /^●/);
  controller.update({ running_count: 2, unread: false, revision: 10 });
  assert.equal(acknowledgements().length, 1, "server acknowledgement must not cause a feedback loop");
  assert.doesNotMatch(document.title, /^●/);
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
  controller.update({ unread: true, revision: 11 });
  assert.equal(acknowledgements().length, 1);
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
  document.dispatchEvent(new window.Event("visibilitychange"));
  assert.equal(acknowledgements().at(-1).revision, 11);
});
test("old aggregate cannot overwrite newer metadata", () => {
  const { document, controller } = setup();
  controller.update({ running_count: 2, unread: false, revision: 5 });
  controller.update({ running_count: 99, unread: true, revision: 4 });
  assert.equal(document.title, "Project — RUN 2 · BLOCK 0 — gwt");
  controller.resetConnection();
  controller.update({ running_count: 1, revision: 1 });
  assert.equal(document.title, "Project — RUN 1 · BLOCK 0 — gwt");
});
test("metadata owns the received snapshot across theme, name, and attention updates", () => {
  const { document, window, sent, controller } = setup();
  const aggregate = { running_count: 2, block_count: 1, error_count: 1, unread: true, revision: 5 };
  controller.update(aggregate);
  Object.assign(aggregate, { running_count: 99, error_count: 0, unread: false, revision: 99 });
  controller.setProjectName("Repo A");
  document.hasFocus = () => true;
  window.dispatchEvent(new window.Event("focus"));
  assert.equal(document.title, "● Repo A — RUN 2 · BLOCK 1 — gwt");
  const icon = document.querySelector('link[rel="icon"]');
  assert.equal(icon.dataset.state, "error");
  assert.equal(icon.dataset.unread, "true");
  const acknowledgements = () => sent.filter(event => event.kind === "project_aggregate_ack");
  assert.equal(acknowledgements().at(-1).revision, 5);
  controller.resetConnection();
  window.dispatchEvent(new window.Event("focus"));
  assert.equal(acknowledgements().length, 1, "reconnect must not acknowledge an old connection's snapshot");
  controller.update({ running_count: 1, unread: false, revision: 1 });
  assert.equal(document.title, "Repo A — RUN 1 · BLOCK 0 — gwt");
  assert.equal(icon.dataset.state, "running");
});
test("unavailable favicon style API does not prevent title updates or acknowledgement", () => {
  const { document, window, sent, controller } = setup();
  window.getComputedStyle = () => { throw new Error("unavailable"); };
  document.hasFocus = () => true;
  assert.doesNotThrow(() => controller.update({ running_count: 3, unread: true, revision: 9 }));
  assert.equal(document.title, "● Project — RUN 3 · BLOCK 0 — gwt");
  assert.equal(sent.at(-1).revision, 9);
});


test("attention reporting is independent of unread and tracks focus, blur, visibility, and reconnect", () => {
  const { document, window, sent, controller } = setup();
  controller.reportAttention();
  assert.deepEqual(sent.at(-1), { kind: "project_client_attention", visible: true, focused: false, notification_permission: "default" });
  document.hasFocus = () => true;
  window.Notification.permission = "granted";
  window.dispatchEvent(new window.Event("focus"));
  assert.deepEqual(sent.at(-1), { kind: "project_client_attention", visible: true, focused: true, notification_permission: "granted" });
  document.hasFocus = () => false;
  window.dispatchEvent(new window.Event("blur"));
  assert.equal(sent.at(-1).focused, false);
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
  document.dispatchEvent(new window.Event("visibilitychange"));
  assert.equal(sent.at(-1).visible, false);
  window.Notification = undefined;
  controller.resetConnection();
  controller.reportAttention();
  assert.equal(sent.at(-1).notification_permission, "unsupported");
  assert.equal(sent.length, 5);
  assert.ok(sent.every((event) => event.kind === "project_client_attention"));
});

test("browser permission changes report current grant without requesting permission", async () => {
  const status = new EventTarget();
  const queries = [];
  const { window, sent } = setup(true, { query: async (descriptor) => { queries.push(descriptor); return status; } });
  await Promise.resolve();
  assert.deepEqual(queries, [{ name: "notifications" }]);
  window.Notification.permission = "denied";
  status.dispatchEvent(new Event("change"));
  assert.equal(sent.at(-1).notification_permission, "denied");
});


test("first aggregate after reconnect resyncs capability once after Project materialization", () => {
  const { sent, controller } = setup();
  controller.resetConnection();
  controller.reportAttention(); // Socket-open report can arrive before a Recent Project exists.
  assert.equal(sent.length, 1);
  controller.update({ unread: false, revision: 7 });
  assert.equal(sent.length, 2, "first accepted aggregate must repeat the capability report");
  assert.deepEqual(sent[1], sent[0]);
  controller.update({ unread: false, revision: 7 });
  controller.update({ unread: false, revision: 8 });
  assert.equal(sent.length, 2, "later aggregates must not reorder recipient election");
  controller.resetConnection();
  controller.reportAttention();
  controller.update({ unread: false, revision: 1 });
  assert.equal(sent.length, 4, "each fresh connection gets one materialization resync");
});
