// Issue #7731 (SPEC #7007 slice N-4): the first-run setup on the Hub page.
//
// AC-1: the screen never says it is shown only at first launch, and opening a
//       step again shows the same screen.
// AC-2: one row chooses the Project Manager; nothing goes on until one is chosen.
// AC-3: the setup ends with a registered tool and a designated Project Manager.
// AC-4: without a registered tool it does not go on, and says why.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";

import { mountHubApp } from "../hub-app.js";

const indexHtml = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const claude = { name: "Claude Code 1", tool: "claude", installed: true };
const snapshot = ({ tools = [claude], members = [], pm = null, needs_setup = false } = {}) =>
  ({ kind: "first_run_setup", view: { tools, members, pm, needs_setup, name_max_chars: 40 } });

function fixture(search = "") {
  const { document, window } = parseHTML(indexHtml);
  const sockets = [];
  class FakeSocket {
    static OPEN = 1;
    constructor() { this.readyState = 0; this.handlers = {}; this.sent = []; sockets.push(this); }
    addEventListener(kind, handler) { this.handlers[kind] = handler; }
    send(raw) {
      const message = JSON.parse(raw);
      if (message.kind.endsWith("first_run_setup")) this.sent.push(message);
    }
    open() { this.readyState = 1; this.handlers.open(); }
    deliver(event) { this.handlers.message({ data: JSON.stringify(event) }); }
  }
  const win = Object.assign(window, {
    location: { href: `http://127.0.0.1:4545/${search}`, pathname: "/" },
    WebSocket: FakeSocket,
    setTimeout: () => 1, clearTimeout() {}, setInterval: () => 1, clearInterval() {},
    requestAnimationFrame: callback => { callback(); return 1; }, cancelAnimationFrame() {},
  });
  const app = mountHubApp({ window: win, document });
  const socket = sockets[0];
  socket.open();
  const setup = () => document.querySelector("[data-setup]");
  const next = () => setup().querySelector("[data-setup-next]");
  return { app, document, socket, setup, next, text: () => setup().textContent };
}

test("AC-3: a machine that needs setup lands on it; one that does not stays on Home", () => {
  const fresh = fixture();
  assert.deepEqual(fresh.socket.sent, [{ kind: "get_first_run_setup" }]);
  fresh.socket.deliver(snapshot({ tools: [], needs_setup: true }));
  assert.equal(fresh.document.querySelector("[data-hub]").dataset.hubView, "setup");
  assert.equal(fresh.setup().dataset.setupStep, "welcome");

  const existing = fixture();
  existing.socket.deliver(snapshot({ members: [{ member_id: "m1", name: "Aoi" }] }));
  assert.equal(existing.document.querySelector("[data-hub]").dataset.hubView, "home");
});

test("AC-1: the setup opens again on the step a Settings link names and never claims to be one-time", () => {
  const { socket, setup, text, document } = fixture("?view=setup&step=pm");
  socket.deliver(snapshot({ members: [{ member_id: "m1", name: "Aoi" }], pm: { member_id: "m1", name: "Aoi" } }));
  assert.equal(document.querySelector("[data-hub]").dataset.hubView, "setup");
  assert.equal(setup().dataset.setupStep, "pm");
  assert.equal(setup().querySelector("h1").textContent, "First-run setup");
  assert.match(text(), /Every step stays available from Settings\./);
  assert.doesNotMatch(text(), /only (at|on|during|shown)|first launch/i);
  // The saved designation is the row's current choice.
  assert.equal([...setup().querySelectorAll('input[type="radio"]')].find(input => input.checked).value, "m1");
});

test("AC-4: without a registered tool the setup does not go on and says why", () => {
  const { socket, setup, next, app } = fixture();
  socket.deliver(snapshot({ tools: [{ name: "Codex 1", tool: "codex", installed: false }], needs_setup: true }));
  next().click();
  assert.equal(setup().dataset.setupStep, "tools");
  assert.equal(next().disabled, true);
  assert.equal(setup().querySelector("[data-setup-reason]").textContent,
    "Next is unavailable because no tool is registered on this machine.");
  assert.match(setup().querySelector("[data-setup-tools-empty]").textContent, /Install a supported agent tool/);
  // A later step never opens ahead of the gate.
  app.setup.showStep("pm");
  assert.equal(setup().dataset.setupStep, "tools");

  // Checking again has the backend detect the installed tools again.
  setup().querySelector("[data-setup-retry]").click();
  assert.deepEqual(socket.sent.at(-1), { kind: "get_first_run_setup", refresh: true });
  socket.deliver(snapshot({ needs_setup: true }));
  assert.equal(next().disabled, false);
  assert.equal(setup().querySelector("[data-setup-reason]"), null);
});

test("AC-2: one row chooses the Project Manager, and nothing goes on until one is chosen", () => {
  const { socket, setup, next, app } = fixture();
  socket.deliver(snapshot({ members: [{ member_id: "m1", name: "Aoi" }, { member_id: "m2", name: "Ren" }] }));
  app.showView("setup");
  app.setup.showStep("pm");
  assert.equal(setup().querySelectorAll("[data-setup-pm]").length, 1, "one row designates the PM");
  assert.deepEqual([...setup().querySelectorAll('input[type="radio"]')].map(input => input.parentElement.textContent),
    ["Aoi", "Ren", "A new agent"]);
  assert.doesNotMatch(setup().textContent, /becomes the Project Manager|you create/i, "no fixed sentence names the PM");
  assert.equal(next().disabled, true);
  assert.equal(setup().querySelector("[data-setup-reason]").textContent, "Next becomes available once a Project Manager is chosen.");
  // The step after stays closed until a PM is saved.
  app.setup.showStep("project");
  assert.equal(setup().dataset.setupStep, "pm");

  setup().querySelector('input[value="m2"]').click();
  assert.equal(next().disabled, false);
  setup().querySelector("form").dispatchEvent(new (setup().ownerDocument.defaultView.Event)("submit", { bubbles: true, cancelable: true }));
  assert.deepEqual(socket.sent.at(-1), { kind: "save_first_run_setup", pm_member_id: "m2" });
  socket.deliver(snapshot({ members: [{ member_id: "m1", name: "Aoi" }, { member_id: "m2", name: "Ren" }], pm: { member_id: "m2", name: "Ren" } }));
  assert.equal(setup().dataset.setupStep, "project");
});

test("AC-3: a new machine finishes with a registered tool and a named Project Manager", () => {
  const { socket, setup, next, document } = fixture();
  socket.deliver(snapshot({ needs_setup: true }));
  next().click();
  assert.equal(setup().querySelector("[data-setup-tool]").textContent, "Claude Code 1Registered");
  next().click();
  // Nobody is in the ledger, so the Project Manager is a new agent.
  assert.equal([...setup().querySelectorAll('input[type="radio"]')].find(input => input.checked).value, "new");
  assert.equal(next().disabled, true);
  assert.equal(setup().querySelector("[data-setup-reason]").textContent, "Next becomes available once the new agent has a name.");
  const name = setup().querySelector("[data-setup-pm-name]");
  name.value = " Aoi ";
  name.dispatchEvent(new (document.defaultView.Event)("input", { bubbles: true }));
  assert.equal(next().disabled, false);
  setup().querySelector("form").dispatchEvent(new (document.defaultView.Event)("submit", { bubbles: true, cancelable: true }));
  assert.deepEqual(socket.sent.at(-1), { kind: "save_first_run_setup", pm_name: "Aoi" });

  // A refusal keeps the step and says why.
  socket.deliver({ kind: "first_run_setup_error", message: "Register a tool before choosing the Project Manager." });
  assert.equal(setup().dataset.setupStep, "pm");
  assert.equal(setup().querySelector("[data-setup-error]").textContent, "Register a tool before choosing the Project Manager.");

  setup().querySelector("form").dispatchEvent(new (document.defaultView.Event)("submit", { bubbles: true, cancelable: true }));
  socket.deliver(snapshot({ pm: { name: "Aoi" } }));
  assert.equal(setup().dataset.setupStep, "project");
  next().click();
  assert.equal(setup().dataset.setupStep, "done");
  assert.match(setup().textContent, /1 tool registered\./);
  assert.match(setup().textContent, /Aoi serves as Project Manager\./);
  setup().querySelector("[data-setup-home]").click();
  assert.equal(document.querySelector("[data-hub]").dataset.hubView, "home");
});
