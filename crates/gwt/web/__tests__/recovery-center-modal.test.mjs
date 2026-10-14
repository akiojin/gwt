import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseHTML } from "linkedom";

import { createRecoveryCenterController } from "../recovery-center-modal.js";

const here = dirname(fileURLToPath(import.meta.url));
const indexHtml = readFileSync(resolve(here, "..", "index.html"), "utf8");

function fixture() {
  const { document, window } = parseHTML(indexHtml);
  let activeElement = null;
  Object.defineProperty(document, "activeElement", {
    configurable: true,
    get: () => activeElement,
  });
  for (const element of document.querySelectorAll("button, select, [tabindex]")) {
    element.focus = () => {
      activeElement = element;
    };
  }
  const opener = document.createElement("button");
  opener.id = "recovery-opener";
  opener.focus = () => {
    activeElement = opener;
  };
  document.body.appendChild(opener);
  opener.focus();
  const sent = [];
  const focused = [];
  const requestIds = ["load-1", "action-1", "load-2", "action-2"];
  const controller = createRecoveryCenterController({
    document,
    modalEl: document.getElementById("recovery-center-modal"),
    dialogEl: document.querySelector("#recovery-center-modal > .modal-shell"),
    send: (message) => sent.push(message),
    focusBoardEntry: (entryId) => focused.push(entryId),
    createRequestId: () => requestIds.shift(),
  });
  return { document, window, opener, controller, sent, focused };
}

const items = [
  {
    action_handle: "opaque-pending",
    state: "pending",
    worktree_form: "ephemeral",
    title: "Pending delivery",
    summary: "Waiting for exact acknowledgement",
    updated_at: "2026-08-10T00:00:00Z",
    session_id: "must-not-render",
    project_root: "/must/not/render",
  },
  {
    action_handle: "opaque-acknowledged",
    state: "acknowledged",
    worktree_form: "branch-backed",
    title: "Acknowledged delivery",
    summary: "Available in Board history",
    updated_at: "2026-08-10T00:01:00Z",
    provider_receipt: "must-not-render",
  },
  {
    action_handle: "opaque-conflicted",
    state: "conflicted",
    worktree_form: "unknown",
    title: "Conflicted delivery",
    summary: "Needs operator attention",
    updated_at: "2026-08-10T00:02:00Z",
    recovery_id: "must-not-render",
  },
];

function ready(controller, overrides = {}) {
  controller.handleState({
    kind: "recovery_center_state",
    request_id: "load-1",
    generation: 11,
    status: "ready",
    items,
    ...overrides,
  });
}

function choose(select, value, window) {
  for (const option of select.querySelectorAll("option")) {
    if (option.value === value) option.setAttribute("selected", "");
    else option.removeAttribute("selected");
  }
  select.dispatchEvent(new window.Event("change"));
}

test("Recovery Center CSS references defined Operator tokens", () => {
  const components = readFileSync(resolve(here, "..", "styles", "components.css"), "utf8");
  const recoveryCss = components.split("/* SPEC-1921 Phase 80 — read-only durable delivery Recovery Center. */")[1];
  assert.ok(recoveryCss, "Recovery Center styles must be present");
  const tokenSources = ["tokens.css", "typography.css"]
    .map((file) => readFileSync(resolve(here, "..", "styles", file), "utf8"))
    .join("\n");
  const defined = new Set(Array.from(tokenSources.matchAll(/(--[\w-]+)\s*:/g), ([, name]) => name));
  const referenced = new Set(Array.from(recoveryCss.matchAll(/var\(\s*(--[\w-]+)/g), ([, name]) => name));
  assert.deepEqual([...referenced].filter((name) => !defined.has(name)), []);
});

test("Recovery Center uses the shared modal and WAI-ARIA primitives", () => {
  const { document } = fixture();
  const modal = document.getElementById("recovery-center-modal");
  const shell = modal.querySelector(":scope > .modal-shell");
  assert.ok(modal.classList.contains("modal-backdrop"));
  assert.equal(shell.getAttribute("role"), "dialog");
  assert.equal(shell.getAttribute("aria-modal"), "true");
  assert.equal(shell.getAttribute("aria-labelledby"), "recovery-center-title");
  assert.ok(shell.querySelector(":scope > .modal-header"));
  assert.ok(shell.querySelector(":scope > .modal-body"));
  assert.ok(shell.querySelector(":scope > .modal-footer"));
  assert.doesNotMatch(modal.textContent, /\b(?:Intake|Execution)\b/);
});

test("open requests a fresh projection and defaults to Pending plus Conflicted attention", () => {
  const { controller, sent, document } = fixture();
  controller.open();
  assert.deepEqual(sent, [{ kind: "load_recovery_center", request_id: "load-1" }]);
  assert.match(document.querySelector("#recovery-center-modal .modal-body").textContent, /Loading/);
  ready(controller);

  const rows = document.querySelectorAll(".recovery-center-row");
  assert.equal(rows.length, 2);
  assert.match(rows[0].textContent + rows[1].textContent, /Pending delivery/);
  assert.match(rows[0].textContent + rows[1].textContent, /Conflicted delivery/);
  assert.doesNotMatch(rows[0].textContent + rows[1].textContent, /Acknowledged delivery/);
  assert.doesNotMatch(document.getElementById("recovery-center-modal").textContent, /must-not-render/);
});

test("state and worktree filters operate independently", () => {
  const { controller, document, window } = fixture();
  controller.open();
  ready(controller);
  const stateFilter = document.getElementById("recovery-center-state-filter");
  const worktreeFilter = document.getElementById("recovery-center-worktree-filter");

  choose(stateFilter, "all", window);
  assert.equal(document.querySelectorAll(".recovery-center-row").length, 3);

  choose(worktreeFilter, "branch-backed", window);
  const rows = document.querySelectorAll(".recovery-center-row");
  assert.equal(rows.length, 1);
  assert.match(rows[0].textContent, /Acknowledged delivery/);

  choose(stateFilter, "pending", window);
  assert.equal(document.querySelectorAll(".recovery-center-row").length, 0);
});

test("only Acknowledged rows can request a Board history deep-link", () => {
  const { controller, sent, document, window } = fixture();
  controller.open();
  ready(controller);
  choose(document.getElementById("recovery-center-state-filter"), "all", window);

  document.querySelector('[data-action-handle="opaque-pending"]').click();
  assert.equal(sent.length, 1, "Pending selection stays inside the Center");
  const openBoard = document.querySelector(
    '[data-action-handle="opaque-acknowledged"] .recovery-center-row__board-action',
  );
  assert.ok(openBoard);
  openBoard.click();
  assert.deepEqual(sent[1], {
    kind: "open_recovery_center_board_entry",
    request_id: "action-1",
    generation: 11,
    action_handle: "opaque-acknowledged",
  });
});

test("stale responses and reconnect generations cannot repopulate private or old rows", () => {
  const { controller, sent, document } = fixture();
  controller.open();
  controller.handleState({
    kind: "recovery_center_state",
    request_id: "stale-load",
    generation: 4,
    status: "ready",
    items,
  });
  assert.equal(document.querySelectorAll(".recovery-center-row").length, 0);

  ready(controller);
  assert.equal(document.querySelectorAll(".recovery-center-row").length, 2);
  controller.reconnect();
  assert.equal(document.querySelectorAll(".recovery-center-row").length, 0);
  assert.deepEqual(sent[1], { kind: "load_recovery_center", request_id: "action-1" });
  controller.handleState({
    kind: "recovery_center_state",
    request_id: "load-1",
    generation: 11,
    status: "ready",
    items,
  });
  assert.equal(document.querySelectorAll(".recovery-center-row").length, 0);
});

test("exact Board action response closes and reuses the existing focusBoardEntry path", () => {
  const { controller, focused, document, window } = fixture();
  controller.open();
  ready(controller);
  const stateFilter = document.getElementById("recovery-center-state-filter");
  choose(stateFilter, "all", window);
  document
    .querySelector('[data-action-handle="opaque-acknowledged"] .recovery-center-row__board-action')
    .click();

  controller.handleBoardEntry({
    kind: "recovery_center_board_entry",
    request_id: "stale-action",
    generation: 11,
    board_entry_id: "wrong-entry",
  });
  assert.deepEqual(focused, []);
  controller.handleBoardEntry({
    kind: "recovery_center_board_entry",
    request_id: "action-1",
    generation: 11,
    board_entry_id: "public-board-entry",
  });
  assert.deepEqual(focused, ["public-board-entry"]);
  assert.equal(document.getElementById("recovery-center-modal").classList.contains("open"), false);
  assert.equal(document.querySelectorAll(".recovery-center-row").length, 0);
});

test("Escape and backdrop close restore the invoking focus", () => {
  const { controller, document, window, opener } = fixture();
  controller.open();
  const escape = new window.Event("keydown");
  Object.defineProperty(escape, "key", { value: "Escape" });
  document.dispatchEvent(escape);
  assert.equal(document.activeElement, opener);

  controller.open();
  const modal = document.getElementById("recovery-center-modal");
  modal.dispatchEvent(new window.Event("click", { bubbles: true }));
  assert.equal(modal.classList.contains("open"), false);
  assert.equal(document.activeElement, opener);
});
