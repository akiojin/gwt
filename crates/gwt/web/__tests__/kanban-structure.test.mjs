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
// SPEC-3064 Phase 3 (E6d): the Kanban renderers moved from app.js into
// knowledge-kanban-surface.js; source-pattern asserts scan both files so
// shared helpers kept in app.js (clamp, createKnowledgeMarkdownBody) and
// the moved renderers stay covered.
const appSource = [
  readFileSync(resolve(here, "../app.js"), "utf8"),
  readFileSync(resolve(here, "../knowledge-kanban-surface.js"), "utf8"),
].join("\n");
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

test("Knowledge windows install periodic cache-first refresh for external phase changes", () => {
  assert.match(
    appSource,
    /KNOWLEDGE_AUTO_REFRESH_INTERVAL_MS/,
    "expected a named Knowledge auto-refresh interval constant",
  );
  assert.match(
    appSource,
    /ensureKnowledgeAutoRefresh/,
    "expected Knowledge windows to install an auto-refresh helper",
  );
  assert.match(
    appSource,
    /setInterval[\s\S]{0,900}?requestKnowledgeBridge\(\s*windowId,\s*knowledgeKind,\s*false\s*\)/,
    "expected auto-refresh to stay cache-first instead of forcing remote sync",
  );
  assert.doesNotMatch(
    appSource,
    /setInterval[\s\S]{0,900}?requestKnowledgeBridge\(\s*windowId,\s*knowledgeKind,\s*true\s*\)/,
    "auto-refresh must not surface GitHub auth failures while cached entries are visible",
  );
  assert.match(
    appSource,
    /state\.loading\s*\|\|\s*state\.refreshing\s*\|\|\s*state\.searching\s*\|\|\s*state\.searchInFlight/,
    "expected auto-refresh to skip while user-visible work is active",
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
  // The renderer must use the shared effective lifecycle helper to assign
  // cards to columns, including the "backlog" fallback for entries with no
  // canonical phase.
  assert.match(
    appSource,
    /function effectiveKnowledgePhase[\s\S]{0,600}?"backlog"/,
    "expected app.js to fall back to 'backlog' in effectiveKnowledgePhase",
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
  // closed Issues carry GitHub state == "closed" but may have no phase/done
  // label or a stale non-done phase. The effective lifecycle helper must
  // promote them to the Done column under one header.
  assert.match(
    appSource,
    /entry\?\.state === "closed"/,
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

test("Workspace Overview List + Detail panes own their scroll boundaries", () => {
  const listPaneRule = componentsCss.match(/\.workspace-overview-list-pane\s*\{[^}]+\}/);
  assert.ok(
    listPaneRule,
    "expected .workspace-overview-list-pane rule in components.css",
  );
  assert.match(
    listPaneRule[0],
    /overflow:\s*auto/,
    "Workspace list pane must own overflow for long Workspace and agent lists",
  );

  const queueRule = componentsCss.match(/\.workspace-agent-queue\s*\{[^}]+\}/);
  assert.ok(queueRule, "expected .workspace-agent-queue rule in components.css");
  assert.match(
    queueRule[0],
    /border-top:/,
    "Unassigned Agents must remain visually separated from Workspace rows",
  );

  const detailRule = componentsCss.match(/\.workspace-overview-detail-pane\s*\{[^}]+\}/);
  assert.ok(
    detailRule,
    "expected .workspace-overview-detail-pane rule in components.css",
  );
  assert.match(detailRule[0], /min-height:\s*0/);
  assert.match(detailRule[0], /overflow:\s*auto/);
  assert.doesNotMatch(
    componentsCss,
    /\.workspace-kanban-board|\.workspace-unassigned\b|\.workspace-kanban-detail-pane/,
    "Workspace Overview must not keep retired Workspace-specific Kanban CSS hooks",
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

// SPEC-2017 US-10: plain Issue cards must surface a (plain) chip and
// stay non-draggable. The renderer derives both off entry.is_spec ===
// false and never lets a plain Issue acquire phase metadata.
test("Plain and closed Issue cards declare non-draggable constraints", () => {
  // is_spec=false or state=closed cards must stay clickable but cannot be
  // dragged into lifecycle columns because that would write invalid phase
  // metadata for the card's source of truth.
  assert.match(
    appSource,
    /const isPlain = entry\.is_spec === false/,
    "expected renderKanbanCard to derive isPlain from is_spec",
  );
  assert.match(
    appSource,
    /const isClosed = String\(entry\?\.state \|\| ""\)\.toLowerCase\(\) === "closed"/,
    "expected renderKanbanCard to derive isClosed from state",
  );
  assert.match(
    appSource,
    /card\.draggable\s*=\s*!isPlain && !isClosed/,
    "expected card.draggable to require both not plain and not closed",
  );
  assert.match(
    appSource,
    /if \(!isPlain && !isClosed\)/,
    "expected D&D handlers to be skipped for closed cards",
  );
  assert.match(
    appSource,
    /kanban-card-chip--plain[\s\S]{0,200}?\(plain\)/,
    "expected the (plain) chip to use the kanban-card-chip--plain class",
  );
});

// SPEC-2017 US-11 follow-up: closed Issue routing into the Done column must
// be reflected as the card's primary lifecycle label. GitHub state is a data
// source, not the user-facing Kanban vocabulary, so closed cards should not
// render a primary CLOSED chip next to the DONE column heading.
test("Closed Issue cards land in Done with a lifecycle phase chip", () => {
  assert.match(
    appSource,
    /effectiveKnowledgePhase/,
    "expected a shared effective lifecycle phase helper",
  );
  assert.match(
    appSource,
    /kanban-card-chip--phase-\$\{effectivePhase\}/,
    "expected the card chip class to use the effective lifecycle phase",
  );
  assert.doesNotMatch(
    appSource,
    /kanban-card-chip--state-\$\{entry\.state\}|kanban-card-chip--state-closed/,
    "card primary chip must not expose raw GitHub state as CLOSED",
  );
});

test("Knowledge detail hides raw phase labels from primary chips", () => {
  assert.match(
    appSource,
    /visibleKnowledgeLabels/,
    "expected Knowledge detail labels to be filtered through a display helper",
  );
  assert.match(
    appSource,
    /!isKnowledgePhaseLabel\(label\)/,
    "expected raw phase/* labels to be excluded from primary label chips",
  );
  assert.match(
    appSource,
    /staleKnowledgePhaseWarning/,
    "expected stale closed-vs-phase metadata to be downgraded to a warning",
  );
});

test("Knowledge lifecycle parsing is exact and nullable-safe", () => {
  const canonicalBody = appSource.match(
    /function canonicalKnowledgePhase\(phase\) \{[\s\S]*?\n      \}/,
  )?.[0];
  assert.ok(canonicalBody, "expected canonicalKnowledgePhase helper");
  assert.doesNotMatch(
    canonicalBody,
    /toLowerCase/,
    "canonical phase parsing must stay case-sensitive",
  );
  assert.match(
    canonicalBody,
    /KNOWLEDGE_PHASES\.has\(value\)/,
    "expected canonical phase parsing to require exact phase keys",
  );
  assert.match(
    appSource,
    /Array\.isArray\(labels\)/,
    "expected label helpers to normalize nullable or non-array labels",
  );
});

test("Kanban drawer uses the same display labels as the detail pane", () => {
  const drawerStart = appSource.indexOf("function renderKanbanDrawerBody()");
  // SPEC-3064 Phase 3 (E6d): the drawer renderer lives at the top of the
  // extracted knowledge surface, right before ensureKnowledgeBridgeState;
  // clamp stays behind in app.js.
  const drawerEnd = appSource.indexOf(
    "function ensureKnowledgeBridgeState(",
    drawerStart,
  );
  assert.ok(drawerStart >= 0, "expected renderKanbanDrawerBody");
  assert.ok(drawerEnd > drawerStart, "expected drawer body boundary");
  const drawerBody = appSource.slice(drawerStart, drawerEnd);
  assert.match(
    drawerBody,
    /visibleKnowledgeLabels/,
    "expected drawer labels to hide raw phase/* labels",
  );
  assert.match(
    drawerBody,
    /staleKnowledgePhaseWarning/,
    "expected drawer to surface stale closed-vs-phase metadata",
  );
  assert.match(
    drawerBody,
    /kanban-card-chip kanban-card-chip--warning/,
    "expected drawer stale phase warning to use the warning chip class",
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

test("Knowledge detail pane renders section bodies through sanitized Markdown helper", () => {
  assert.match(
    appSource,
    /function\s+createKnowledgeMarkdownBody\(/,
    "expected a shared Markdown body helper for Knowledge detail sections",
  );
  assert.match(
    appSource,
    /section\.body_html/,
    "expected renderer to prefer sanitized backend-generated section.body_html",
  );
  assert.match(
    appSource,
    /createKnowledgeMarkdownBody\(section\)/,
    "expected right detail pane to append section bodies through the Markdown helper",
  );
  assert.doesNotMatch(
    appSource,
    /createNode\("pre",\s*"knowledge-section-body",\s*section\.body\)/,
    "right detail pane must not render Markdown as plaintext pre blocks",
  );
  assert.doesNotMatch(
    appSource,
    /\.innerHTML\s*=\s*section\.body\b/,
    "raw section.body must never be assigned to innerHTML",
  );
});
