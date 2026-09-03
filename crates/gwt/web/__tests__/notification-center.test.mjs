// SPEC #3206 v2 — notification center (bell + unread badge + history drawer).
//
// The center is a pure sink: it records every notice it is handed (FR-011),
// keeps a session-scoped in-memory history (FR-013), and owns nothing about
// firing / dedup / gating (FR-016). The list itself is the shared toast-host
// stack mounted inside the drawer, so cap / newest-on-top / dropped / dismiss
// / clear-all / level rim come from the primitive.

import assert from "node:assert/strict";
import { test } from "node:test";
import { parseHTML } from "linkedom";

import { createNotificationCenter } from "../notification-center.js";

function setup(opts = {}) {
  const { document } = parseHTML(
    "<!doctype html><html><head></head><body></body></html>",
  );
  const center = createNotificationCenter({ document, ...opts });
  center.mount(document.body);
  return { document, center };
}

function items(document) {
  return [...document.querySelectorAll(".notification-center__item")];
}

function click(document, target) {
  target.dispatchEvent(new document.defaultView.Event("click", { bubbles: true }));
}

// --- T-001: history contract (FR-010 / FR-011 / FR-013, Sc 2, Sc 7) ---

test("requires a document; mount builds the drawer shell and the history log region", () => {
  assert.throws(() => createNotificationCenter({}), /document/);
  const { document } = setup();
  const drawer = document.querySelector(".notification-center-drawer");
  assert.ok(drawer, "drawer shell is mounted");
  assert.ok(drawer.classList.contains("op-drawer"), "reuses the shared .op-drawer primitive");
  assert.equal(drawer.getAttribute("role"), "dialog");
  assert.equal(drawer.getAttribute("aria-modal"), "true");
  assert.ok(drawer.getAttribute("aria-labelledby"), "dialog is labelled");
  assert.equal(drawer.dataset.open, "false");
  assert.equal(drawer.hidden, true, "closed by default");
  assert.ok(
    document.querySelector(".notification-center-backdrop.op-drawer-backdrop"),
    "shares the .op-drawer-backdrop primitive",
  );
  const log = document.querySelector(".notification-center");
  assert.ok(log, "history region is mounted inside the drawer");
  assert.ok(drawer.contains(log), "history lives inside the drawer body");
  assert.equal(log.getAttribute("role"), "log");
  assert.equal(log.getAttribute("aria-live"), "polite");
  assert.ok(document.querySelector(".notification-center__list"));
  assert.ok(document.querySelector(".notification-center__clear"), "clear-all control");
  assert.ok(document.querySelector(".notification-center__empty"), "empty state");
});

test("record() prepends newest-on-top with level, kind, title, message and issue suffix", () => {
  const { document, center } = setup();
  center.record({ kind: "issue-monitor", level: "info", title: "Issue Monitor", message: "started", issueNumber: 42 });
  center.record({ kind: "attention", level: "error", title: "Agent error", message: "boom" });
  const list = items(document);
  assert.equal(list.length, 2);
  assert.equal(list[0].dataset.level, "error");
  assert.equal(list[0].dataset.kind, "attention");
  assert.match(list[0].textContent, /Agent error/);
  assert.match(list[0].textContent, /boom/);
  assert.equal(list[1].dataset.level, "info");
  assert.equal(list[1].dataset.kind, "issue-monitor");
  assert.match(list[1].textContent, /Issue Monitor #42/, "issue number suffix is reproduced");
  assert.match(list[1].textContent, /started/);
  assert.equal(center.count(), 2);
  assert.equal(document.querySelector(".notification-center__empty").hidden, true, "empty state hides once history exists");
});

test("maxRetained caps the history from the oldest and keeps the dropped count", () => {
  const { document, center } = setup({ maxRetained: 3 });
  for (let i = 0; i < 10; i += 1) {
    center.record({ kind: "attention", level: "info", title: `n${i}` });
  }
  assert.equal(center.count(), 3);
  assert.equal(center.droppedCount(), 7, "overflow is counted, never silently lost");
  assert.match(items(document)[0].textContent, /n9/, "newest retained");
});

test("unknown level normalizes to info", () => {
  const { document, center } = setup();
  center.record({ kind: "attention", level: "bogus", title: "x" });
  assert.equal(items(document)[0].dataset.level, "info");
});

test("per-item × dismisses only that entry; clearAll() empties the history", () => {
  const { document, center } = setup();
  center.record({ kind: "attention", level: "info", title: "a" });
  center.record({ kind: "attention", level: "info", title: "b" });
  const dismiss = items(document)[0].querySelector(".notification-center__dismiss");
  assert.ok(dismiss, "history items are dismissible");
  click(document, dismiss);
  assert.equal(center.count(), 1);
  assert.match(items(document)[0].textContent, /a/);
  center.clearAll();
  assert.equal(center.count(), 0);
  assert.equal(document.querySelector(".notification-center__empty").hidden, false, "empty state returns");
});

test("the clear-all control clears through the same path", () => {
  const { document, center } = setup();
  center.record({ kind: "attention", level: "info", title: "a" });
  click(document, document.querySelector(".notification-center__clear"));
  assert.equal(center.count(), 0);
});

test("history items never auto-expire (no timeout is forwarded)", async () => {
  const { center } = setup();
  center.record({ kind: "agent-completion", level: "neutral", title: "done", timeoutMs: 5 });
  await new Promise((resolve) => setTimeout(resolve, 30));
  assert.equal(center.count(), 1, "history is sticky even if a caller passes timeoutMs");
});

test("activating a history item runs its jump-to and KEEPS the item (Sc 7)", () => {
  const { document, center } = setup();
  let jumps = 0;
  center.record({ kind: "attention", level: "warn", title: "needs input", onActivate: () => (jumps += 1) });
  const item = items(document)[0];
  assert.equal(item.getAttribute("role"), "button", "jump-to keeps the keyboard-button contract");
  click(document, item);
  click(document, item);
  assert.equal(jumps, 2, "jump-to is reusable");
  assert.equal(center.count(), 1, "history is not consumed by the jump");
});

test("same-kind notices never collapse into one row (no id is forwarded)", () => {
  const { document, center } = setup();
  center.record({ kind: "agent-completion", level: "neutral", title: "first", id: "agent-completion" });
  center.record({ kind: "agent-completion", level: "neutral", title: "second", id: "agent-completion" });
  center.record({ kind: "attention", level: "error", title: "w1 a", id: "attention-w1" });
  center.record({ kind: "attention", level: "error", title: "w1 b", id: "attention-w1" });
  assert.equal(center.count(), 4, "history has no dedup — every notice is a row");
  assert.equal(document.querySelectorAll("[data-toast-id]").length, 0, "singleton ids are not forwarded");
});

// --- T-002: unread semantics (FR-009 / FR-014, Sc 5) ---

test("records while closed count as unread; open() reads them all", () => {
  const { center } = setup();
  const seen = [];
  center.onUnreadChange((count, hasError) => seen.push([count, hasError]));
  assert.equal(center.unreadCount(), 0);
  center.record({ kind: "attention", level: "info", title: "a" });
  center.record({ kind: "attention", level: "warn", title: "b" });
  assert.equal(center.unreadCount(), 2);
  assert.equal(center.unreadHasError(), false);
  center.open();
  assert.equal(center.isOpen(), true);
  assert.equal(center.unreadCount(), 0, "opening the drawer marks everything read");
  assert.deepEqual(seen.at(-1), [0, false], "subscribers learn about the reset");
});

test("an unread error is flagged for the bell emphasis", () => {
  const { center } = setup();
  center.record({ kind: "attention", level: "error", title: "boom" });
  assert.equal(center.unreadCount(), 1);
  assert.equal(center.unreadHasError(), true);
  center.open();
  assert.equal(center.unreadHasError(), false);
});

test("records while open are read immediately but still enter the history (FR-011)", () => {
  const { center } = setup();
  center.open();
  center.record({ kind: "board-mention", level: "info", title: "reply" });
  assert.equal(center.count(), 1, "recorded regardless of drawer state");
  assert.equal(center.unreadCount(), 0, "not unread when the operator is looking");
  center.close();
  center.record({ kind: "board-mention", level: "info", title: "reply 2" });
  assert.equal(center.count(), 2);
  assert.equal(center.unreadCount(), 1);
});

test("dismissing or clearing unread items drops them from the unread count", () => {
  const { document, center } = setup();
  center.record({ kind: "attention", level: "error", title: "a" });
  center.record({ kind: "attention", level: "info", title: "b" });
  assert.equal(center.unreadCount(), 2);
  click(document, items(document)[0].querySelector(".notification-center__dismiss"));
  assert.equal(center.unreadCount(), 1);
  assert.equal(center.unreadHasError(), true, "the remaining unread is the error one");
  center.clearAll();
  assert.equal(center.unreadCount(), 0);
  assert.equal(center.unreadHasError(), false);
});

test("toggle() flips the drawer, syncs data-open / hidden and the backdrop", () => {
  const { document, center } = setup();
  const drawer = document.querySelector(".notification-center-drawer");
  const backdrop = document.querySelector(".notification-center-backdrop");
  center.toggle();
  assert.equal(center.isOpen(), true);
  assert.equal(drawer.dataset.open, "true");
  assert.equal(drawer.hidden, false);
  assert.equal(backdrop.dataset.open, "true");
  center.toggle();
  assert.equal(center.isOpen(), false);
  assert.equal(drawer.dataset.open, "false");
  assert.equal(drawer.hidden, true);
  assert.equal(backdrop.dataset.open, "false");
});

test("the close control and the backdrop both close the drawer", () => {
  const { document, center } = setup();
  center.open();
  click(document, document.querySelector(".notification-center-drawer .op-drawer__close"));
  assert.equal(center.isOpen(), false);
  center.open();
  click(document, document.querySelector(".notification-center-backdrop"));
  assert.equal(center.isOpen(), false);
});
