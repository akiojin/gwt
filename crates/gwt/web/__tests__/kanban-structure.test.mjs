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
