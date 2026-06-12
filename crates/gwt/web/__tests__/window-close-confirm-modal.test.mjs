/* SPEC-3038 US-3 — Close Guard.
 *
 * Every window close (titlebar × and tab ×) routes through this confirmation
 * modal regardless of agent state (user-confirmed decision, 2026-06-10).
 * The renderer mirrors close-project-tab-confirm-modal.js: pure renderer,
 * `.modal-backdrop` + `.modal-shell` classes, focus-trap + focus restore,
 * backdrop click / Esc cancel, Cancel as the default focus.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { parseHTML } from "linkedom";

import { renderWindowCloseConfirmModal } from "../window-close-confirm-modal.js";

function setupDom() {
  const { document } = parseHTML(`
    <div class="modal-backdrop" id="window-close-confirm-modal" aria-hidden="true">
      <div class="modal-shell window-close-confirm-shell" role="dialog" aria-modal="true" tabindex="-1"></div>
    </div>
  `);
  return {
    document,
    modalEl: document.getElementById("window-close-confirm-modal"),
    dialogEl: document.querySelector(".window-close-confirm-shell"),
  };
}

// linkedom does not track focus()/activeElement faithfully (see
// focus-trap.test.mjs), so the createNode factory records focus() calls and
// the default-focus assertion checks the recorded log instead.
function createNodeFor(document, focusLog = []) {
  return (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    const originalFocus = typeof node.focus === "function" ? node.focus.bind(node) : null;
    node.focus = (opts) => {
      focusLog.push(className || tag);
      try {
        originalFocus?.(opts);
      } catch {
        /* linkedom focus quirks are irrelevant to the contract */
      }
    };
    return node;
  };
}

function render(deps, state) {
  const calls = { cancel: 0, confirm: 0, focusLog: [] };
  renderWindowCloseConfirmModal({
    modalEl: deps.modalEl,
    dialogEl: deps.dialogEl,
    state,
    createNode: createNodeFor(deps.document, calls.focusLog),
    onCancel: () => {
      calls.cancel += 1;
    },
    onConfirm: () => {
      calls.confirm += 1;
    },
  });
  return calls;
}

test("open renders window title, agent label, and runtime state", () => {
  const deps = setupDom();
  render(deps, {
    open: true,
    windowId: "win-1",
    windowTitle: "fix flaky tests",
    agentLabel: "Claude Code",
    runtimeLabel: "Running",
    running: true,
  });

  assert.ok(deps.modalEl.classList.contains("open"));
  assert.equal(deps.modalEl.getAttribute("aria-hidden"), null);
  const text = deps.dialogEl.textContent;
  assert.match(text, /Close window\?/);
  assert.match(text, /fix flaky tests/);
  assert.match(text, /Claude Code/);
  assert.match(text, /Running/);
});

test("running windows render a destructive warning; idle windows do not", () => {
  const deps = setupDom();
  render(deps, {
    open: true,
    windowId: "win-1",
    windowTitle: "agent",
    agentLabel: "Codex",
    runtimeLabel: "Running",
    running: true,
  });
  const warning = deps.dialogEl.querySelector(".window-close-confirm__warning");
  assert.ok(warning, "running close must show a destructive warning");
  assert.match(warning.textContent, /stopped/i);

  render(deps, {
    open: true,
    windowId: "win-2",
    windowTitle: "logs",
    agentLabel: "",
    runtimeLabel: "",
    running: false,
  });
  assert.equal(
    deps.dialogEl.querySelector(".window-close-confirm__warning"),
    null,
    "non-running close must not show the destructive warning",
  );
});

test("Cancel button receives default focus and fires onCancel", () => {
  const deps = setupDom();
  const calls = render(deps, {
    open: true,
    windowId: "win-1",
    windowTitle: "agent",
    agentLabel: "",
    runtimeLabel: "",
    running: false,
  });
  const cancel = deps.dialogEl.querySelector(
    '[data-role="window-close-cancel"]',
  );
  assert.ok(cancel, "expected Cancel button");
  assert.ok(
    calls.focusLog.some((entry) => String(entry).includes("window-close-confirm__cancel")),
    `Cancel must take default focus (focused: ${JSON.stringify(calls.focusLog)})`,
  );
  cancel.dispatchEvent(
    new deps.document.defaultView.Event("click", { bubbles: true }),
  );
  assert.equal(calls.cancel, 1);
  assert.equal(calls.confirm, 0);
});

test("confirm button fires onConfirm exactly once per click", () => {
  const deps = setupDom();
  const calls = render(deps, {
    open: true,
    windowId: "win-1",
    windowTitle: "agent",
    agentLabel: "",
    runtimeLabel: "Running",
    running: true,
  });
  const confirm = deps.dialogEl.querySelector(
    '[data-role="window-close-confirm"]',
  );
  assert.ok(confirm, "expected confirm button");
  assert.ok(
    confirm.className.includes("destructive"),
    "confirm action must be destructive-styled",
  );
  confirm.dispatchEvent(
    new deps.document.defaultView.Event("click", { bubbles: true }),
  );
  assert.equal(calls.confirm, 1);
  assert.equal(calls.cancel, 0);
});

test("backdrop click and Escape cancel without confirming", () => {
  const deps = setupDom();
  const calls = render(deps, {
    open: true,
    windowId: "win-1",
    windowTitle: "agent",
    agentLabel: "",
    runtimeLabel: "",
    running: false,
  });

  deps.modalEl.dispatchEvent(
    new deps.document.defaultView.Event("click", { bubbles: true }),
  );
  assert.equal(calls.cancel, 1, "backdrop click cancels");

  const escape = new deps.document.defaultView.Event("keydown", { bubbles: true });
  Object.defineProperty(escape, "key", { value: "Escape" });
  deps.document.dispatchEvent(escape);
  assert.equal(calls.cancel, 2, "Escape cancels");
  assert.equal(calls.confirm, 0);
});

test("closing the modal clears content, restores aria-hidden, and returns focus", () => {
  const deps = setupDom();
  const outside = deps.document.createElement("button");
  deps.document.body?.appendChild?.(outside);
  outside.focus?.();

  render(deps, {
    open: true,
    windowId: "win-1",
    windowTitle: "agent",
    agentLabel: "",
    runtimeLabel: "",
    running: false,
  });
  render(deps, { open: false, windowId: null });

  assert.equal(deps.modalEl.classList.contains("open"), false);
  assert.equal(deps.modalEl.getAttribute("aria-hidden"), "true");
  assert.equal(deps.dialogEl.firstChild, null, "dialog content cleared");
});
