// SPEC-2017 Kanban View — DOM 構造アサーション
//
// 6 カラム Kanban (Backlog / Draft / Planning / Implementation / Review /
// Done) が CSS とレンダラに実装されていることを検証する。実 DOM は
// app.js の renderKnowledgeBridge() が動的に作るので、テストは
// components.css のスタイル宣言と app.js のソースパターンを確認する
// （chrome-structure テストと同じ手法）。

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const componentsCss = readFileSync(
  resolve(here, "../styles/components.css"),
  "utf8",
);

const CANONICAL_PHASES = [
  "backlog",
  "draft",
  "planning",
  "implementation",
  "review",
  "done",
];

test("renderKnowledgeBridge emits all six Kanban columns by data-phase", () => {
  // The renderer must create one .kanban-column per canonical phase plus
  // Backlog. We assert the renderer mentions each data-phase value so the
  // skeleton is complete; missing one would lose a whole pipeline stage.
  for (const phase of CANONICAL_PHASES) {
    const pattern = new RegExp(`data-phase=\\\\?"${phase}\\\\?"`);
    assert.match(
      appSource,
      pattern,
      `expected app.js renderer to emit data-phase="${phase}"`,
    );
  }
});

test("Kanban toolbar exposes Hide done toggle id", () => {
  // The Hide done toggle is the new replacement for listScope (open/closed).
  // It is identifiable by id so localStorage persistence can wire to it.
  assert.match(
    appSource,
    /kanban-hide-done/,
    "expected Hide done toggle id (kanban-hide-done) referenced in app.js",
  );
});

test("Kanban removes legacy open and closed list scope controls", () => {
  assert.doesNotMatch(
    appSource,
    /data-knowledge-scope/,
    "Kanban should not render the legacy Open/Closed scope toggle",
  );
  assert.doesNotMatch(
    appSource,
    /listScope|list_scope|switchKnowledgeListScope/,
    "Kanban should not keep the legacy listScope request contract",
  );
});

test("renderKnowledgeBridge groups entries into kanban columns by phase", () => {
  // The renderer must use entry.phase to assign data-phase on each card,
  // and fall back to "backlog" when phase is null.
  assert.match(
    appSource,
    /entry\.phase\s*\|\|\s*"backlog"/,
    "expected app.js to fall back to 'backlog' when entry.phase is null",
  );
  assert.match(
    appSource,
    /\.kanban-column\[data-phase=/,
    "expected app.js to look up kanban column by data-phase",
  );
});

test("Plain (non-spec) entries fall into the Backlog column", () => {
  // Plain GitHub Issues (no gwt-spec label, is_spec=false) carry no phase
  // labels at all and must land in Backlog with a (plain) marker.
  assert.match(
    appSource,
    /entry\.is_spec\s*===\s*false/,
    "expected app.js to branch on entry.is_spec === false for plain issues",
  );
  assert.match(
    appSource,
    /\(plain\)/,
    "expected app.js to display a (plain) marker on non-spec cards",
  );
});

test("Closed issues land in the Done column", () => {
  // closed Issues carry GitHub state == "closed" but no phase/done label.
  // The renderer must promote them to the Done column so they appear with
  // phase/done open Issues under one header.
  assert.match(
    appSource,
    /entry\.state\s*===\s*"closed"/,
    "expected app.js to detect closed state",
  );
  assert.match(
    appSource,
    /"done"/,
    "expected app.js to assign closed entries to the done column",
  );
});

test("Hide done toggle persists to localStorage", () => {
  // The toggle state must persist via localStorage so reloads honor the
  // user's preference. This is the replacement for the old listScope.
  assert.match(
    appSource,
    /localStorage[\s\S]{0,200}?kanban-hide-done|kanban-hide-done[\s\S]{0,200}?localStorage/,
    "expected app.js to persist Hide done state via localStorage",
  );
});

test("components.css declares the kanban-board grid layout", () => {
  // The Kanban board must use CSS grid. Fixed-width columns risk being
  // unreadable on narrow viewports, so we want a responsive grid template
  // that allows horizontal scroll.
  const boardRule = componentsCss.match(/\.kanban-board\s*\{[^}]+\}/);
  assert.ok(boardRule, "expected .kanban-board { ... } rule in components.css");
  assert.match(
    boardRule[0],
    /grid-template-columns/,
    ".kanban-board should declare grid-template-columns",
  );
});

test("Kanban shell lets column bodies own vertical scrolling", () => {
  const listPaneRule = componentsCss.match(/\.kanban-shell\s+\.kanban-list-pane\s*\{[^}]+\}/);
  assert.ok(
    listPaneRule,
    "expected .kanban-shell .kanban-list-pane rule to constrain the board viewport",
  );
  assert.match(listPaneRule[0], /display:\s*flex/);
  assert.match(listPaneRule[0], /flex-direction:\s*column/);
  assert.match(
    listPaneRule[0],
    /overflow:\s*hidden/,
    "Kanban list pane should not own vertical scroll; column bodies should",
  );

  const boardRule = componentsCss.match(/\.kanban-board\s*\{[^}]+\}/);
  assert.ok(boardRule, "expected .kanban-board { ... } rule in components.css");
  assert.match(boardRule[0], /overflow-x:\s*auto/);
  assert.match(boardRule[0], /overflow-y:\s*hidden/);

  const columnBodyRule = componentsCss.match(/\.kanban-column-body\s*\{[^}]+\}/);
  assert.ok(columnBodyRule, "expected .kanban-column-body { ... } rule");
  assert.match(columnBodyRule[0], /overflow-y:\s*auto/);
  assert.match(columnBodyRule[0], /min-height:\s*0/);
});

test("components.css declares column and card classes", () => {
  // Cards and columns need their own selectors so the renderer can style
  // them independently. Without these classes the renderer can't apply
  // Operator tokens consistently.
  assert.match(
    componentsCss,
    /\.kanban-column\b/,
    "expected .kanban-column rule in components.css",
  );
  assert.match(
    componentsCss,
    /\.kanban-card\b/,
    "expected .kanban-card rule in components.css",
  );
});

test("Kanban cards declare warning indicator hook for unknown phase", () => {
  // Issues with phase/legacy or multiple canonical labels expose
  // has_unknown_phase = true. The renderer must surface a visual warning
  // so users notice malformed metadata.
  assert.match(
    appSource,
    /has_unknown_phase/,
    "expected app.js to honor entry.has_unknown_phase",
  );
});

// SPEC-2017 US-10: plain Issue cards must surface a (plain) chip and
// stay non-draggable. The renderer derives both off entry.is_spec ===
// false and never lets a plain Issue acquire phase metadata.
test("Plain Issue cards declare draggable=!isPlain and a (plain) chip", () => {
  // is_spec=false → isPlain=true → draggable=false. The negation
  // pattern (`!isPlain`) is the canonical form in renderKanbanCard.
  assert.match(
    appSource,
    /const isPlain = entry\.is_spec === false/,
    "expected renderKanbanCard to derive isPlain from is_spec",
  );
  assert.match(
    appSource,
    /card\.draggable\s*=\s*!isPlain/,
    "expected card.draggable to flip on isPlain",
  );
  assert.match(
    appSource,
    /kanban-card-chip--plain[\s\S]{0,200}?\(plain\)/,
    "expected the (plain) chip to use the kanban-card-chip--plain class",
  );
});

// SPEC-2017 US-11: closed Issue routing into the Done column. The
// renderer overrides entry.phase with "done" whenever entry.state is
// "closed", and the state chip uses kanban-card-chip--state-closed.
test("Closed Issue cards land in Done with the closed state chip class", () => {
  assert.match(
    appSource,
    /entry\.state === "closed" \? "done"/,
    "expected closed entries to be routed to the done column",
  );
  assert.match(
    appSource,
    /kanban-card-chip--state-\$\{entry\.state\}|kanban-card-chip--state-closed/,
    "expected the state chip class to be derived from entry.state",
  );
});

test("Kanban card click keeps the selected item in the right detail pane", () => {
  assert.match(
    appSource,
    /knowledge-detail-pane/,
    "expected Knowledge Bridge to retain the right-side detail pane",
  );
  assert.match(
    appSource,
    /addEventListener\("click"[\s\S]{0,500}?requestKnowledgeDetail[\s\S]{0,500}?renderKnowledgeBridge/,
    "expected card click to refresh detail inside the split-pane Kanban surface",
  );
  assert.doesNotMatch(
    appSource,
    /addEventListener\("click"[\s\S]{0,500}?openKanbanDrawer/,
    "Kanban card click must not open a small Drawer as the primary detail UI",
  );
});
