// @vitest-environment linkedom
/* Issue #7728 — the one project stop/start (design items 56, 179 and 191).
 *
 * AC-1: one confirmation and one operation for every entry point.
 * AC-2: monitoring has no switch of its own; it is part of the project stop.
 */
import test from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import "../__tests__/jsx-loader.mjs";
import { ProjectAutonomy } from "../src/surfaces/ProjectAutonomy.jsx";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

function mount(autonomy) {
  const { document, window } = parseHTML("<!doctype html><html><body><div id=\"root\"></div></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  const sent = [];
  const root = createRoot(document.getElementById("root"));
  const render = next => act(() => root.render(React.createElement(ProjectAutonomy, {
    autonomy: next, projectTitle: "gwt", onSetRunning: running => sent.push(running),
  })));
  render(autonomy);
  const state = () => document.querySelector("[data-project-autonomy-state]");
  const action = () => document.querySelector("[data-project-autonomy-action]");
  const click = node => act(() => node.click());
  return { document, sent, render, state, action, click };
}

test("a running project stops only after the shared confirmation", () => {
  const page = mount({ monitor_enabled: true, pm_paused: false, active_agents: 2 });
  assert.equal(page.state().textContent, "Running");
  assert.equal(page.action().getAttribute("data-project-autonomy-action"), "stop");

  page.click(page.action());
  const dialog = page.document.querySelector("[data-project-stop-dialog] [role=dialog]");
  assert.ok(dialog, "Stop opens the confirmation");
  assert.equal(dialog.getAttribute("aria-modal"), "true");
  assert.match(dialog.textContent, /Stop this project\?/);
  assert.match(dialog.textContent, /New assignments and the Project Manager patrol stop now/);
  assert.match(dialog.textContent, /cannot be cancelled/);
  assert.deepEqual(page.sent, [], "nothing is sent before confirmation");

  page.click(page.document.querySelector("[data-project-stop-cancel]"));
  assert.equal(page.document.querySelector("[data-project-stop-dialog]"), null);
  assert.deepEqual(page.sent, []);

  page.click(page.action());
  page.click(page.document.querySelector("[data-project-stop-confirm]"));
  assert.deepEqual(page.sent, [false], "confirm sends one stop");
  assert.equal(page.document.querySelector("[data-project-stop-dialog]"), null);
});

test("Stop pending is one-way, and a stopped project starts without a dialog", () => {
  const page = mount({ monitor_enabled: false, pm_paused: true, active_agents: 1 });
  assert.equal(page.state().textContent, "Stop pending · 1 agent finishing");
  assert.equal(page.action(), null, "no cancel while agents finish");

  page.render({ monitor_enabled: false, pm_paused: true, active_agents: 0 });
  assert.equal(page.state().textContent, "Stopped");
  assert.equal(page.action().textContent, "Start");
  page.click(page.action());
  assert.deepEqual(page.sent, [true]);
  assert.equal(page.document.querySelector("[data-project-stop-dialog]"), null);
});

test("only one half running is Partly running with Start and Stop; missing inputs show Unknown", () => {
  for (const autonomy of [{ monitor_enabled: false, pm_paused: false }, { monitor_enabled: true, pm_paused: true }]) {
    const page = mount({ ...autonomy, active_agents: 0 });
    assert.equal(page.state().textContent, "Partly running");
    const actions = [...page.document.querySelectorAll("[data-project-autonomy-action]")];
    assert.deepEqual(actions.map(button => button.getAttribute("data-project-autonomy-action")), ["start", "stop"]);
    page.click(actions[0]);
    assert.deepEqual(page.sent, [true], "Start brings both halves up");
  }
  const unknown = mount({ monitor_enabled: true, pm_paused: null });
  assert.equal(unknown.state().textContent, "Unknown");
  assert.equal(unknown.action(), null, "no action from a guessed state");
});
