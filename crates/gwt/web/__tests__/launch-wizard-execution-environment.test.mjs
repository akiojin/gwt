// Issue #7652 (SPEC #7007 items 189, 197, 243 and 251) — the launch wizard
// names the execution environment a launch runs on, where it stands on the
// project's list and why each entry ahead of it is not used; it says that
// model, reasoning and speed chosen there apply to that launch only; and it
// says where the place to run comes from.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";

const surface = readFileSync(new URL("../launch-wizard-surface.js", import.meta.url), "utf8");
const css = readFileSync(new URL("../styles/app.css", import.meta.url), "utf8");

function surfaceFunction(name) {
  const start = surface.indexOf(`      function ${name}(`);
  assert.notEqual(start, -1, `${name} must exist`);
  return surface.slice(start, surface.indexOf("\n      }\n", start) + 9);
}

function render(name, ...args) {
  const { document } = parseHTML("<section id='parent'></section>");
  const createNode = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const parent = document.getElementById("parent");
  new Function("createNode", "parent", "args", `${surfaceFunction(name)}; ${name}(parent, ...args);`)(
    createNode,
    parent,
    args,
  );
  return parent;
}

test("the environment row gives its place and the reason for each entry ahead of it", () => {
  const parent = render("appendExecutionEnvironmentNote", {
    name: "Codex 1",
    via_tool_name: "Codex",
    priority: 2,
    list_len: 2,
    note: "Number 2 of 2 on this project's list.",
    skipped: [
      { priority: 1, name: "Claude Code 1", reason: "Claude Code is not installed on this machine." },
    ],
  });
  const note = parent.querySelector(".launch-environment");
  assert.ok(note, "the row renders");
  assert.equal(note.getAttribute("role"), "note");
  assert.equal(note.dataset.priority, "2");
  assert.equal(
    note.querySelector(".launch-environment__title").textContent,
    "Execution environment · Codex 1",
  );
  assert.match(note.textContent, /Number 2 of 2 on this project's list\./);
  const skipped = [...note.querySelectorAll(".launch-environment__skipped")];
  assert.deepEqual(
    skipped.map((row) => row.textContent),
    ["1. Claude Code 1 — Claude Code is not installed on this machine."],
  );
  assert.match(note.textContent, /tried in the order of this project's list/);
});

test("nothing renders without an environment", () => {
  assert.equal(render("appendExecutionEnvironmentNote", null).children.length, 0);
});

test("a launch says its choices are overrides; the saved-settings form does not", () => {
  const launch = render("appendLaunchOverrideNote", null);
  const note = launch.querySelector(".launch-override-note");
  assert.match(note.textContent, /apply to this launch only/);
  assert.match(note.textContent, /priority over the execution environment/);
  assert.match(note.textContent, /saved traits stay as they are/);
  // Issues settings saves what it shows, so it carries no such note.
  assert.equal(render("appendLaunchOverrideNote", { sets: [] }).children.length, 0);
});

test("the surface wires the three additions to the backend view", () => {
  assert.match(
    surface,
    /appendExecutionEnvironmentNote\(\s*section,\s*launchWizard\.execution_environment,?\s*\)/,
  );
  assert.match(
    surface,
    /appendLaunchOverrideNote\(\s*section,\s*launchWizard\.issue_monitor_pool,?\s*\)/,
  );
  assert.match(surface, /launchWizard\.runtime_target_note/);
});

test("the environment row is styled with Operator tokens only", () => {
  const start = css.indexOf(".launch-environment");
  assert.notEqual(start, -1);
  const block = css.slice(start, css.indexOf("}", css.indexOf(".launch-environment__title")));
  assert.doesNotMatch(block, /#[0-9a-fA-F]{3,8}\b|rgba?\(/);
});
