// Issue #2704 — `shouldSkipTerminalFocusActivation` guards `renderWorkspace`'s
// terminal focus activation so the Clone Project modal (and other modals with
// text inputs) can hold keyboard focus while the background terminal is still
// streaming `workspace_state` events.
//
// The guard is intentionally framework-agnostic: it takes a `doc` and an
// optional `modalElements` array and returns true when xterm focus stealing
// should be suppressed for one activation cycle. The caller still runs
// refresh + fit + sendGeometry — only the `terminal.focus()` step is skipped.

import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";

import { shouldSkipTerminalFocusActivation } from "../clone-modal-focus-guard.js";

function makeDoc(html = "<!doctype html><html><body></body></html>") {
  const { document } = parseHTML(html);
  return document;
}

test("returns false when no modal is open and no text input is focused", () => {
  const document = makeDoc();
  const modal = document.createElement("div");
  modal.classList.add("modal-backdrop");
  document.body.appendChild(modal);
  assert.equal(
    shouldSkipTerminalFocusActivation({
      doc: document,
      modalElements: [modal],
    }),
    false,
  );
});

test("returns true when any supplied modal element has the .open class", () => {
  const document = makeDoc();
  const modalA = document.createElement("div");
  const modalB = document.createElement("div");
  modalB.classList.add("open");
  document.body.append(modalA, modalB);
  assert.equal(
    shouldSkipTerminalFocusActivation({
      doc: document,
      modalElements: [modalA, modalB],
    }),
    true,
  );
});

test("returns true when document.activeElement is an INPUT", () => {
  const document = makeDoc();
  const input = document.createElement("input");
  input.type = "text";
  document.body.appendChild(input);
  // linkedom does not auto-track activeElement on focus(); set it directly.
  Object.defineProperty(document, "activeElement", { value: input, configurable: true });
  assert.equal(
    shouldSkipTerminalFocusActivation({ doc: document, modalElements: [] }),
    true,
  );
});

test("returns true when document.activeElement is a TEXTAREA", () => {
  const document = makeDoc();
  const textarea = document.createElement("textarea");
  document.body.appendChild(textarea);
  Object.defineProperty(document, "activeElement", { value: textarea, configurable: true });
  assert.equal(
    shouldSkipTerminalFocusActivation({ doc: document, modalElements: [] }),
    true,
  );
});

test("returns false when document.activeElement is xterm helper textarea", () => {
  const document = makeDoc();
  const textarea = document.createElement("textarea");
  textarea.className = "xterm-helper-textarea";
  document.body.appendChild(textarea);
  Object.defineProperty(document, "activeElement", { value: textarea, configurable: true });
  assert.equal(
    shouldSkipTerminalFocusActivation({ doc: document, modalElements: [] }),
    false,
  );
});

test("modal open still suppresses focus when xterm helper textarea is active", () => {
  const document = makeDoc();
  const modal = document.createElement("div");
  modal.classList.add("open");
  const textarea = document.createElement("textarea");
  textarea.className = "xterm-helper-textarea";
  document.body.append(modal, textarea);
  Object.defineProperty(document, "activeElement", { value: textarea, configurable: true });
  assert.equal(
    shouldSkipTerminalFocusActivation({ doc: document, modalElements: [modal] }),
    true,
  );
});

test("returns true when activeElement is contentEditable", () => {
  const document = makeDoc();
  const editable = document.createElement("div");
  editable.setAttribute("contenteditable", "true");
  // linkedom does not derive isContentEditable from the attribute; stub it.
  Object.defineProperty(editable, "isContentEditable", { value: true, configurable: true });
  document.body.appendChild(editable);
  Object.defineProperty(document, "activeElement", { value: editable, configurable: true });
  assert.equal(
    shouldSkipTerminalFocusActivation({ doc: document, modalElements: [] }),
    true,
  );
});

test("ignores activeElement that is not text-input-like (e.g. BUTTON)", () => {
  const document = makeDoc();
  const button = document.createElement("button");
  document.body.appendChild(button);
  Object.defineProperty(document, "activeElement", { value: button, configurable: true });
  assert.equal(
    shouldSkipTerminalFocusActivation({ doc: document, modalElements: [] }),
    false,
  );
});

test("tolerates nullish entries in modalElements without throwing", () => {
  const document = makeDoc();
  assert.equal(
    shouldSkipTerminalFocusActivation({
      doc: document,
      modalElements: [null, undefined],
    }),
    false,
  );
});

test("tolerates missing modalElements / doc arguments", () => {
  // Defensive: even when the caller passes nothing, the guard must not throw.
  assert.equal(shouldSkipTerminalFocusActivation({}), false);
  assert.equal(shouldSkipTerminalFocusActivation(), false);
});
