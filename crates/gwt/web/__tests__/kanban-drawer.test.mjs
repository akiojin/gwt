// SPEC-2017 US-9 Kanban legacy Drawer — secondary detail surface
//
// The Kanban view now keeps the primary selected item in the right-side
// detail pane. The legacy Drawer scaffold may remain for compatibility,
// but card clicks must not route the primary UX into a small Drawer.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const indexHtml = readFileSync(resolve(here, "../index.html"), "utf8");
// Combined source for chrome-structure assertions that may live in
// either app.js (when injected dynamically) or index.html (when part
// of the static modal layer).
const chromeSource = `${indexHtml}\n${appSource}`;
const componentsCss = readFileSync(
  resolve(here, "../styles/components.css"),
  "utf8",
);

test("Knowledge Bridge Drawer has role=dialog + aria-modal=true", () => {
  // SPEC-2017 US-9 + SPEC-2356: the Drawer is a modal surface, not a
  // panel, so it must announce itself to assistive tech with both
  // role=dialog and aria-modal=true. Without aria-modal screen
  // readers leak focus to the underlying surface during navigation.
  assert.match(
    chromeSource,
    /class="op-drawer kanban-drawer"|class="kanban-drawer op-drawer"/,
    "expected Knowledge Bridge to use the SPEC-2356 .op-drawer pattern (with .kanban-drawer modifier)",
  );
  // The kanban-drawer block in index.html declares role=dialog +
  // aria-modal=true — assert the pair appears within a 500-char
  // window of the kanban-drawer class so we don't accidentally
  // accept role=dialog from a sibling modal.
  assert.match(
    chromeSource,
    /kanban-drawer[\s\S]{0,500}?role="dialog"[\s\S]{0,500}?aria-modal="true"|kanban-drawer[\s\S]{0,500}?aria-modal="true"[\s\S]{0,500}?role="dialog"/,
    "expected Knowledge Bridge Drawer to declare role=dialog + aria-modal=true",
  );
});

test("Drawer is wired to focus-trap on open", () => {
  // SPEC-2017 US-9: opening the Drawer must call createFocusTrap so
  // Tab cannot leak focus back to the Kanban Board behind the
  // backdrop. The release function returned by createFocusTrap is
  // stored so close can dismantle the trap symmetrically.
  assert.match(
    appSource,
    /kanban-drawer[\s\S]{0,500}?createFocusTrap|createFocusTrap[\s\S]{0,500}?kanbanDrawerFocusTrap/,
    "expected Knowledge Bridge Drawer to invoke createFocusTrap on open",
  );
});

test("Drawer Esc / backdrop click closes the surface", () => {
  // SPEC-2017 US-9: per WAI-ARIA dialog pattern, Esc must dismiss
  // the modal. Backdrop click is also expected (parallel to existing
  // wizard / migration / branch-cleanup modals).
  assert.match(
    appSource,
    /closeKnowledgeBridgeDrawer|closeKanbanDrawer/,
    "expected app.js to expose a close function for the Kanban Drawer",
  );
  // The handler may use either `if (event.key !== "Escape") return`
  // (early-return style, which is how the rest of the codebase
  // handles global Esc) or `if (event.key === "Escape")` directly.
  // Both patterns are acceptable; we just need the global Esc handler
  // to route the open Drawer branch to closeKanbanDrawer.
  const escapeHandlerIndex = appSource.indexOf(
    'document.addEventListener("keydown", (event) => {\n        if (event.key !== "Escape") return;',
  );
  assert.notEqual(
    escapeHandlerIndex,
    -1,
    "expected a global Escape keydown handler",
  );
  const kanbanBranchIndex = appSource.indexOf(
    'if (kanbanDrawer && kanbanDrawer.dataset.open === "true")',
    escapeHandlerIndex,
  );
  assert.notEqual(
    kanbanBranchIndex,
    -1,
    "expected the Escape handler to inspect Kanban Drawer open state",
  );
  const nextBranchIndex = appSource.indexOf(
    "if (windowListOpen)",
    kanbanBranchIndex,
  );
  const closeIndex = appSource.indexOf(
    "closeKanbanDrawer();",
    kanbanBranchIndex,
  );
  assert.ok(
    closeIndex > kanbanBranchIndex
      && (nextBranchIndex === -1 || closeIndex < nextBranchIndex),
    "expected Esc keydown to close the Kanban Drawer",
  );
});

test("Drawer reuses .op-drawer styles via .kanban-drawer modifier class", () => {
  // We don't redefine the Drawer geometry — we add a modifier class
  // for Kanban-specific tweaks (e.g. detail section spacing). The
  // base .op-drawer / .op-drawer-backdrop / @media reduced-motion
  // rules already exist (SPEC-2356) so we just verify the modifier
  // is declared.
  assert.match(
    componentsCss,
    /\.kanban-drawer\b/,
    "expected .kanban-drawer modifier rule in components.css",
  );
});

test("Drawer footer hosts a Launch Agent action when launch_issue_number is set", () => {
  // SPEC-2017 US-9 + existing renderer behavior: when the detail has
  // a launch_issue_number we surface a Launch Agent button so the
  // user can jump straight to wizard. This was already in the
  // pre-Drawer renderer — keep the behavior, move the button into
  // the Drawer footer.
  assert.match(
    appSource,
    /Launch Agent[\s\S]{0,400}?openIssueLaunchWizard|openIssueLaunchWizard[\s\S]{0,400}?Launch Agent/,
    "expected the Drawer to include the Launch Agent action",
  );
});

test("Card click no longer opens the Drawer as the primary detail UI", () => {
  // The renderer wires .kanban-card click to request fresh detail and
  // rerender the split-pane surface. The Drawer open function can
  // remain available, but the click handler must not call it.
  assert.match(
    appSource,
    /openKnowledgeBridgeDrawer|openKanbanDrawer/,
    "expected the legacy Kanban Drawer function to remain available",
  );
  assert.doesNotMatch(
    appSource,
    /addEventListener\("click"[\s\S]{0,500}?(openKanbanDrawer|openKnowledgeBridgeDrawer)|kanban-card[\s\S]{0,500}?(openKanbanDrawer|openKnowledgeBridgeDrawer)/,
    "card click must keep detail in the right pane, not open the Drawer",
  );
});

test("Drawer renders section bodies through sanitized Markdown helper", () => {
  assert.match(
    appSource,
    /function\s+createKnowledgeMarkdownBody\(/,
    "expected a shared Markdown body helper for Knowledge detail sections",
  );
  assert.match(
    appSource,
    /section\.body_html/,
    "expected Drawer renderer to use sanitized backend-generated section.body_html",
  );
  assert.match(
    appSource,
    /createKnowledgeMarkdownBody\(section,\s*"kanban-drawer-section-body"\)/,
    "expected Drawer section bodies to flow through the Markdown helper",
  );
  assert.doesNotMatch(
    appSource,
    /createNode\("pre",\s*"kanban-drawer-section-body",\s*section\.body\)/,
    "Drawer must not render Markdown as plaintext pre blocks",
  );
  assert.doesNotMatch(
    appSource,
    /\.innerHTML\s*=\s*section\.body\b/,
    "raw section.body must never be assigned to innerHTML",
  );
});
