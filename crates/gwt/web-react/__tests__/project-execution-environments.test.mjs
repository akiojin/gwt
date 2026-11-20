// @vitest-environment linkedom
import test from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import "../__tests__/jsx-loader.mjs";
import { ProjectExecutionEnvironments, addableEnvironments } from "../src/surfaces/ProjectExecutionEnvironments.jsx";
import { createUiStateStore } from "../../web/ui-state-store.js";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

// Issue #7741 AC-2: a project's available execution environments can be
// reordered, added to and removed from; the order is the priority.
const environment = (id, tool, toolName, name) => ({ id, name, via_tool: tool, via_tool_name: toolName });
const ENVIRONMENTS = [
  environment("claude", "claude", "Claude Code", "Claude Code 1"),
  environment("codex", "codex", "Codex", "Codex 1"),
  environment("claude:hermes", "hermes", "Hermes Agent", "Claude Code 1 (via Hermes Agent)"),
  environment("codex:hermes", "hermes", "Hermes Agent", "Codex 1 (via Hermes Agent)"),
];
const entry = id => {
  const found = ENVIRONMENTS.find(item => item.id === id);
  return { environment_id: id, tool: found.via_tool, tool_name: found.via_tool_name, name: found.name };
};
const view = (ids, saved = true) => ({
  environments: ENVIRONMENTS,
  project: { project_root: "/projects/alpha", saved, entries: ids.map(entry) },
});

async function mount(current, error = null) {
  const { document, window } = parseHTML("<!doctype html><html><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  const store = createUiStateStore({ view: current, error }, { scheduleFrame: callback => { callback(); return () => {}; } });
  const saved = [];
  let managed = 0;
  const root = createRoot(document.body);
  await act(async () => root.render(React.createElement(ProjectExecutionEnvironments, {
    store, onSave: ids => saved.push(ids), onManage: () => { managed += 1; },
  })));
  const button = label => [...document.querySelectorAll("button")].find(item =>
    item.textContent.trim() === label || item.getAttribute("aria-label") === label);
  const click = async element => { await act(async () => { element.click(); }); };
  return { document, store, saved, button, click, managed: () => managed };
}

test("the list shows each environment with its priority and saves a new order", async () => {
  const { document, saved, button, click } = await mount(view(["claude", "codex", "claude:hermes"]));
  const names = [...document.querySelectorAll(".project-environments__name")].map(node => node.textContent);
  assert.deepEqual(names, ["1. Claude Code 1", "2. Codex 1", "3. Claude Code 1 (via Hermes Agent)"]);

  await click(button("Move down Claude Code 1"));
  await click(button("Move up Claude Code 1 (via Hermes Agent)"));
  assert.deepEqual(saved, [["codex", "claude", "claude:hermes"], ["claude", "claude:hermes", "codex"]]);
  assert.equal(button("Move up Claude Code 1").hasAttribute("disabled"), true);
  assert.equal(button("Move up Claude Code 1").getAttribute("title"), "Already first");
  assert.equal(button("Move down Claude Code 1 (via Hermes Agent)").hasAttribute("disabled"), true);
});

test("the list follows the saved order when the store changes", async () => {
  const { document, store } = await mount(view(["claude", "codex"]));
  await act(async () => { store.update(() => ({ view: view(["codex", "claude"]), error: null })); });
  const names = [...document.querySelectorAll(".project-environments__name")].map(node => node.textContent);
  assert.deepEqual(names, ["1. Codex 1", "2. Claude Code 1"]);
});

test("an environment is removed from the list, but the last one stays with the reason shown", async () => {
  const many = await mount(view(["claude", "codex"]));
  await many.click(many.button("Remove Codex 1"));
  assert.deepEqual(many.saved, [["claude"]]);

  const one = await mount(view(["claude"]));
  const remove = one.button("Remove Claude Code 1");
  assert.equal(remove.hasAttribute("disabled"), true);
  const reason = one.document.getElementById(remove.getAttribute("aria-describedby"));
  assert.match(reason.textContent, /Keep at least one/);
});

test("an environment that is not on the list is added to the end", async () => {
  const { document, saved } = await mount(view(["claude"]));
  const options = [...document.querySelectorAll("select option")];
  assert.deepEqual(options.map(option => option.getAttribute("value")), ["codex", "claude:hermes", "codex:hermes"]);
  await act(async () => {
    document.querySelector("form").dispatchEvent(new document.defaultView.Event("submit", { bubbles: true, cancelable: true }));
  });
  assert.deepEqual(saved, [["claude", "codex"]]);
});

test("an environment whose tool is already listed cannot be added and says why", async () => {
  const current = view(["claude", "codex", "claude:hermes"]);
  assert.deepEqual(addableEnvironments(current), [{
    id: "codex:hermes", name: "Codex 1 (via Hermes Agent)",
    blockedReason: 'Already uses Hermes Agent through "Claude Code 1 (via Hermes Agent)"',
  }]);
  const { document, button } = await mount(current);
  assert.equal(document.querySelector("select option").hasAttribute("disabled"), true);
  assert.equal(button("Add to the end").hasAttribute("disabled"), true);
  assert.match(document.body.textContent, /The remaining environments use a tool that is already on the list\./);

  const all = await mount({ ...current, environments: ENVIRONMENTS.slice(0, 3) });
  assert.match(all.document.body.textContent, /Every execution environment is already on the list\./);
});

test("a list that was never saved says so, and a tool without an environment keeps its row", async () => {
  const unsaved = view(["claude"], false);
  unsaved.project.entries.push({ environment_id: "my-agent", tool: "my-agent", tool_name: "my-agent" });
  const { document } = await mount(unsaved);
  assert.match(document.body.textContent, /This is the order the project launches with today\./);
  assert.match(document.body.textContent, /2\. my-agent \(not registered as an execution environment\)/);
  assert.doesNotMatch((await mount(view(["claude"]))).document.body.textContent, /launches with today/);
});

test("loading, no project and a refused save are each told apart", async () => {
  assert.match((await mount(null)).document.body.textContent, /Loading execution environments…/);
  assert.match((await mount({ environments: [] })).document.body.textContent, /Open a project to choose its execution environments\./);
  const refused = await mount(view(["claude"]), "Two environments on the list start sessions with the same tool.");
  assert.match(refused.document.querySelector('[role="alert"]').textContent, /same tool/);
});

test("the list leads to the screen where environments are managed", async () => {
  const { button, click, managed } = await mount(view(["claude"]));
  await click(button("Manage execution environments"));
  assert.equal(managed(), 1);
});
