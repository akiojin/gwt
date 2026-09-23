import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";
import { createProjectPageMetadata } from "../project-page-metadata.js";

function setup(project = true) {
  const { document, window } = parseHTML('<html><head><title>gwt — Operator</title><link rel="icon" href="data:,"></head><body></body></html>');
  window.getComputedStyle = () => ({ getPropertyValue: (token) => token === "--color-state-blocked" ? "red" : "blue" });
  const sent = [];
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
  const { document, controller } = setup(false);
  controller.setProjectName("Repo A");
  controller.update({ running_count: 7, unread: true });
  assert.equal(document.title, "gwt — Hub");
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
  assert.deepEqual(sent, [{ kind: "project_aggregate_ack", revision: 9, visible: true, focused: true }]);
  assert.match(document.title, /^●/);
  controller.update({ running_count: 2, unread: false, revision: 10 });
  assert.equal(sent.length, 1, "server acknowledgement must not cause a feedback loop");
  assert.doesNotMatch(document.title, /^●/);
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
  controller.update({ unread: true, revision: 11 });
  assert.equal(sent.length, 1);
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
  document.dispatchEvent(new window.Event("visibilitychange"));
  assert.equal(sent.at(-1).revision, 11);
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
test("unavailable favicon style API does not prevent title updates or acknowledgement", () => {
  const { document, window, sent, controller } = setup();
  window.getComputedStyle = () => { throw new Error("unavailable"); };
  document.hasFocus = () => true;
  assert.doesNotThrow(() => controller.update({ running_count: 3, unread: true, revision: 9 }));
  assert.equal(document.title, "● Project — RUN 3 · BLOCK 0 — gwt");
  assert.equal(sent.at(-1).revision, 9);
});
