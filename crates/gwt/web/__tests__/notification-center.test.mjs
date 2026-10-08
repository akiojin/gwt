// SPEC #3206 v2 — notification center (bell + unread badge + history drawer).
//
// The center is a pure sink: it records every notice it is handed (FR-011),
// keeps a session-scoped in-memory history (FR-013), and owns nothing about
// firing / dedup / gating (FR-016). The list itself is the shared toast-host
// stack mounted inside the drawer, so cap / newest-on-top / dropped / dismiss
// / clear-all / level rim come from the primitive.

import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

import { createNotificationCenter, renderNotificationBell } from "../notification-center.js";

const here = dirname(fileURLToPath(import.meta.url));
const appCss = readFileSync(resolve(here, "..", "styles", "app.css"), "utf8");

/** Every `selector { ... }` rule in app.css that mentions the notification center. */
function notificationCenterRules() {
  const rules = [];
  const pattern = /([^{}]+)\{([^{}]*)\}/g;
  let match = pattern.exec(appCss);
  while (match) {
    const selector = match[1].trim();
    if (selector.includes("notification-center")) {
      rules.push({ selector, body: match[2] });
    }
    match = pattern.exec(appCss);
  }
  return rules;
}

function ruleBlock(selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = appCss.match(new RegExp(`(^|\\s)${escaped}\\s*\\{([^}]*)\\}`, "m"));
  assert.ok(match, `expected CSS rule for ${selector} in app.css`);
  return match[2];
}

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
  const seen = [];
  center.onOpenChange((open) => seen.push(open));
  center.toggle();
  assert.equal(center.isOpen(), true);
  assert.deepEqual(seen, [true], "open-state subscribers (bell aria-expanded) are told");
  assert.equal(drawer.dataset.open, "true");
  assert.equal(drawer.hidden, false);
  assert.equal(backdrop.dataset.open, "true");
  center.toggle();
  assert.equal(center.isOpen(), false);
  assert.equal(drawer.dataset.open, "false");
  assert.equal(drawer.hidden, true);
  assert.equal(backdrop.dataset.open, "false");
  assert.deepEqual(seen, [true, false]);
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

// --- T-006 (model half): bell badge renderer (FR-009) ---

test("renderNotificationBell syncs count, visibility, error emphasis, aria-label and aria-expanded", () => {
  const { document } = parseHTML(
    '<!doctype html><button id="b" aria-label="Notifications" aria-expanded="false"><span class="op-rail__badge" hidden aria-hidden="true"></span></button>',
  );
  const button = document.getElementById("b");
  const badge = button.querySelector(".op-rail__badge");

  renderNotificationBell({ button, badge, count: 0, hasError: false, open: false });
  assert.equal(badge.hidden, true, "0 unread hides the badge");
  assert.equal(button.dataset.unread, "0");
  assert.equal(button.getAttribute("aria-label"), "Notifications");
  assert.equal(button.getAttribute("aria-expanded"), "false");

  renderNotificationBell({ button, badge, count: 3, hasError: false, open: false });
  assert.equal(badge.hidden, false);
  assert.equal(badge.textContent, "3");
  assert.equal(badge.dataset.hasError, "false");
  assert.equal(button.dataset.unread, "3");
  assert.equal(button.getAttribute("aria-label"), "Notifications (3 unread)");

  renderNotificationBell({ button, badge, count: 120, hasError: true, open: true });
  assert.equal(badge.textContent, "99+", "count is clamped for the tiny rail badge");
  assert.equal(badge.dataset.hasError, "true", "state token emphasis hook");
  assert.equal(button.getAttribute("aria-label"), "Notifications (120 unread, includes errors)");
  assert.equal(button.getAttribute("aria-expanded"), "true");
});

// --- FR-017: surface errors — dedup by key, occurrence count, auto-read on resolve ---

test("recordError dedups by key: repeats bump the occurrence count on ONE error row", () => {
  const { document, center } = setup();
  const first = center.recordError({ key: "issue-monitor:last_error", title: "Issue Monitor", message: "scan failed" });
  const again = center.recordError({ key: "issue-monitor:last_error", title: "Issue Monitor", message: "scan failed" });
  assert.equal(first, again, "same key reuses the row");
  assert.equal(center.count(), 1);
  assert.equal(first.dataset.level, "error");
  assert.equal(first.dataset.kind, "error");
  assert.equal(first.dataset.errorKey, "issue-monitor:last_error");
  assert.equal(first.querySelector(".notification-center__count")?.textContent, "×2", "occurrence count is visible");
  assert.equal(center.unreadCount(), 1, "one unread row, not two");
  assert.equal(center.unreadHasError(), true);
  center.recordError({ key: "issue-window:w1:load", title: "Issue window", message: "gh: rate_limited" });
  assert.equal(center.count(), 2, "different keys are different rows");
});

test("resolveError marks the row resolved and read; a later recurrence re-flags the SAME row unread", () => {
  const { document, center } = setup();
  const row = center.recordError({ key: "k", title: "Issue window", message: "boom" });
  assert.equal(center.unreadCount(), 1);
  assert.equal(center.resolveError("k"), true);
  assert.equal(row.dataset.resolved, "true");
  assert.equal(center.unreadCount(), 0, "resolved errors fall to read automatically");
  assert.equal(center.unreadHasError(), false);
  assert.equal(center.count(), 1, "history keeps the record");
  assert.equal(center.resolveError("unknown"), false);

  const again = center.recordError({ key: "k", title: "Issue window", message: "boom" });
  assert.equal(again, row, "recurrence reuses the row instead of piling up duplicates");
  assert.equal(row.dataset.resolved, "false");
  assert.equal(row.querySelector(".notification-center__count")?.textContent, "×2");
  assert.equal(center.unreadCount(), 1, "the recurrence is unread again");
});

test("a dismissed error row does not resurrect on recurrence", () => {
  const { document, center } = setup();
  const row = center.recordError({ key: "k", title: "t", message: "m" });
  click(document, row.querySelector(".notification-center__dismiss"));
  assert.equal(center.count(), 0);
  const fresh = center.recordError({ key: "k", title: "t", message: "m" });
  assert.notEqual(fresh, row, "a new row is created after the old one was dismissed");
  assert.equal(fresh.querySelector(".notification-center__count"), null, "count starts over (single occurrence shows no ×N)");
});

// --- Issue #3979: triage row layout (user ruling 2026-09-05) ---
//
// The drawer is where unhandled notices get CLEARED, not read, so every row
// leads with a severity marker and keeps title / time / dismiss on one scan
// line. Severity is encoded twice — color AND marker shape/glyph — and never
// through the accent, so it survives color-blind viewing and theme swaps.

const SEVERITY_CASES = [
  { level: "error", severity: "critical" },
  { level: "warn", severity: "warning" },
  { level: "success", severity: "good" },
  { level: "info", severity: "neutral" },
  { level: "done", severity: "neutral" },
  { level: "neutral", severity: "neutral" },
];

test("every row carries a severity derived from its level", () => {
  const { document, center } = setup();
  for (const { level } of SEVERITY_CASES) {
    center.record({ kind: "attention", level, title: level });
  }
  const rows = items(document);
  // newest-on-top, so the recorded order is reversed in the DOM.
  const recorded = [...SEVERITY_CASES].reverse();
  rows.forEach((row, index) => {
    assert.equal(
      row.dataset.severity,
      recorded[index].severity,
      `level ${recorded[index].level} must triage as ${recorded[index].severity}`,
    );
  });
});

test("the severity marker leads the row with a per-severity glyph and an accessible name", () => {
  const { document, center } = setup();
  const glyphs = new Map();
  for (const { level, severity } of SEVERITY_CASES) {
    center.record({ kind: "attention", level, title: level, message: "why" });
    const row = items(document)[0];
    const marker = row.querySelector(".notification-center__severity");
    assert.ok(marker, `level ${level} must render a severity marker`);
    assert.equal(row.firstElementChild, marker, "the marker leads the row (DOM + reading order)");
    assert.equal(marker.dataset.severity, severity);
    assert.ok(marker.textContent.trim().length > 0, "the marker carries a glyph, not just a color");
    assert.equal(marker.getAttribute("role"), "img");
    assert.ok(marker.getAttribute("aria-label"), "the severity is named for screen readers");
    glyphs.set(severity, marker.textContent.trim());
  }
  assert.equal(
    new Set(glyphs.values()).size,
    glyphs.size,
    "each severity needs its own glyph so color is never the only cue",
  );
});

test("the triage row keeps marker, title, dismiss, message and time in one scan structure", () => {
  const { document, center } = setup();
  center.record({ kind: "attention", level: "error", title: "Agent error", message: "boom" });
  const row = items(document)[0];
  assert.deepEqual(
    [...row.children].map((child) => child.className),
    [
      "notification-center__severity",
      "notification-center__title",
      "notification-center__dismiss",
      "notification-center__message",
      "notification-center__time",
    ],
    "triage row children (grid areas place them; DOM order stays reader-friendly)",
  );
});

test("the row grid puts severity, title, time and dismiss on the header line", () => {
  const body = ruleBlock(".notification-center__item");
  assert.match(body, /display:\s*grid/, "the triage row is a grid, not a stacked card");
  const areas = body.match(/grid-template-areas:([^;]*);/);
  assert.ok(areas, "the triage row declares named grid areas");
  const [headerRow, messageRow] = areas[1].match(/"[^"]*"/g) || [];
  assert.ok(headerRow && messageRow, "the triage row has a header line and a message line");
  for (const area of ["severity", "headline", "time", "dismiss"]) {
    assert.ok(headerRow.includes(area), `${area} belongs on the header scan line`);
  }
  assert.ok(messageRow.includes("message"), "the message wraps under the header line");
  for (const [selector, area] of [
    [".notification-center__severity", "severity"],
    [".notification-center__title", "headline"],
    [".notification-center__time", "time"],
    [".notification-center__dismiss", "dismiss"],
    [".notification-center__message", "message"],
  ]) {
    assert.match(ruleBlock(selector), new RegExp(`grid-area:\\s*${area}\\s*;`), `${selector} → ${area}`);
  }
});

test("severity markers differ in shape, not only in color", () => {
  const shapes = new Map();
  for (const severity of ["critical", "warning", "good", "neutral"]) {
    const body = ruleBlock(`.notification-center__severity[data-severity="${severity}"]`);
    const radius = body.match(/border-radius:\s*([^;]+);/);
    assert.ok(radius, `${severity} marker must declare its own border-radius`);
    shapes.set(severity, radius[1].trim());
  }
  assert.ok(
    new Set(shapes.values()).size >= 3,
    `severity markers must be tellable apart by shape alone, got ${JSON.stringify([...shapes])}`,
  );
});

test("the row stripe is driven by severity, and the retired per-level rims are gone", () => {
  for (const severity of ["critical", "warning", "good", "neutral"]) {
    assert.match(
      ruleBlock(`.notification-center__item[data-severity="${severity}"]`),
      /border-left-color:\s*var\(--[a-z-]+\)/,
      `${severity} rows need a token-driven severity stripe`,
    );
  }
  assert.deepEqual(
    notificationCenterRules()
      .map((rule) => rule.selector)
      .filter((selector) => selector.includes("__item[data-level=")),
    [],
    "the pre-triage per-level rim rules must not survive alongside the severity stripe",
  );
});

test("notification-center CSS is token-only (no raw hex / rgb / rgba)", () => {
  for (const { selector, body } of notificationCenterRules()) {
    assert.ok(
      !/#[0-9a-fA-F]{3,8}\b/.test(body),
      `${selector} must not hard-code a hex color: ${body.trim()}`,
    );
    assert.ok(
      !/\brgba?\s*\(/.test(body),
      `${selector} must not hard-code an rgb/rgba color: ${body.trim()}`,
    );
  }
});

test("severity styling never borrows the accent color", () => {
  for (const { selector, body } of notificationCenterRules()) {
    if (!selector.includes("severity")) {
      continue;
    }
    assert.ok(
      !/--color-focus-ring|--color-state-active/.test(body),
      `${selector} must encode severity independently of the accent: ${body.trim()}`,
    );
  }
});
