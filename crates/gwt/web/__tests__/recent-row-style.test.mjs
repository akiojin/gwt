// Issue #2746 — regression test: Recent project rows in the Open Project ▾
// split-button dropdown and in the project picker overlay must declare
// `user-select: none` so wry/WebKit does not promote pointerdown into a
// text-selection drag and swallow the click. This is the same root cause
// pattern Issue #2744 fixed for `.project-tab`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appCss = readFileSync(
  resolve(here, "..", "styles", "app.css"),
  "utf8",
);

function ruleBlock(selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = new RegExp(`(^|\\s)${escaped}\\s*\\{([^}]*)\\}`, "m");
  const match = appCss.match(pattern);
  assert.ok(match, `expected CSS rule for ${selector} in app.css`);
  return match[2];
}

test(".split-button-menu-recent-row declares user-select: none", () => {
  const body = ruleBlock(".split-button-menu-recent-row");
  assert.match(
    body,
    /user-select\s*:\s*none\s*;/,
    "expected user-select: none on .split-button-menu-recent-row "
      + "(see Issue #2746 — without this, span-children text drag-select "
      + "swallows clicks on Recent rows inside the Open Project dropdown)",
  );
});

test(".recent-project-row declares user-select: none", () => {
  const body = ruleBlock(".recent-project-row");
  assert.match(
    body,
    /user-select\s*:\s*none\s*;/,
    "expected user-select: none on .recent-project-row "
      + "(see Issue #2746 — picker overlay shares the same span-child pattern "
      + "as the dropdown Recent rows)",
  );
});

// Issue #2746 follow-up — without these constraints, the long meta path
// (e.g. `/Users/.../unity-cli/develop`) forces the grid track wider than
// the menu inner content area, so the hover background bleeds ~28px to the
// right of the dropdown. The fix combines a `minmax(0, 1fr)` grid column
// (so the column refuses to grow beyond the container) with `min-width: 0`
// on the row (so the flex column can shrink and the meta span's
// `text-overflow: ellipsis` actually activates).
test(".split-button-menu-recent uses minmax(0, 1fr) so rows cannot push beyond the menu", () => {
  const body = ruleBlock(".split-button-menu-recent");
  assert.match(
    body,
    /grid-template-columns\s*:\s*minmax\(\s*0\s*,\s*1fr\s*\)\s*;/,
    "expected grid-template-columns: minmax(0, 1fr) on "
      + ".split-button-menu-recent (see Issue #2746 follow-up — without it, "
      + "long meta paths expand the implicit auto column and the hover "
      + "background overflows the dropdown right edge)",
  );
});

test(".split-button-menu-recent-row sets min-width: 0 so meta text can truncate", () => {
  const body = ruleBlock(".split-button-menu-recent-row");
  assert.match(
    body,
    /min-width\s*:\s*0\s*;/,
    "expected min-width: 0 on .split-button-menu-recent-row (see Issue "
      + "#2746 follow-up — the row is a flex column whose default min-width "
      + "is the meta span's min-content; without 0 the ellipsis never "
      + "engages and the row pushes the dropdown wider on hover)",
  );
});

// Issue #2746 follow-up — once meta truncates with ellipsis the user can no
// longer tell which directory a Recent row points to, so each row's
// `title` attribute carries the full path for the native hover/focus
// tooltip. The text we point at is the same `${kind} · ${path}` payload as
// the meta span, kept identical so tooltip and visible text never disagree.
// SPEC-3064 Phase 3 (E7): the Recent Projects renderers moved from app.js
// to project-shell-surface.js; the tooltip pins read the surface module.
const projectShellJs = readFileSync(
  resolve(here, "..", "project-shell-surface.js"),
  "utf8",
);

test("renderRecentProjectsIntoMenu sets row.title for the hover tooltip", () => {
  // Anchor from the function header to a unique line near its end
  // (openProjectMenuRecent.appendChild(row)) so we capture the whole loop
  // body without being fooled by an early `if (!openProjectMenuRecent)`
  // return brace.
  const match = projectShellJs.match(
    /function\s+renderRecentProjectsIntoMenu\s*\([\s\S]*?openProjectMenuRecent\.appendChild\(row\);/,
  );
  assert.ok(
    match,
    "expected to locate renderRecentProjectsIntoMenu in project-shell-surface.js",
  );
  assert.match(
    match[0],
    /row\.title\s*=\s*`\$\{project\.kind\}\s*·\s*\$\{project\.path\}`/,
    "expected renderRecentProjectsIntoMenu to assign "
      + "`row.title = `${project.kind} · ${project.path}`` so the truncated "
      + "meta still surfaces the full path via the native hover tooltip",
  );
});

test("renderRecentProjects sets row.title for the hover tooltip", () => {
  const match = projectShellJs.match(
    /function\s+renderRecentProjects\s*\([\s\S]*?recentProjectList\.appendChild\(row\);/,
  );
  assert.ok(
    match,
    "expected to locate renderRecentProjects in project-shell-surface.js",
  );
  assert.match(
    match[0],
    /row\.title\s*=\s*`\$\{project\.kind\}\s*·\s*\$\{project\.path\}`/,
    "expected renderRecentProjects to assign the same "
      + "`row.title = `${project.kind} · ${project.path}`` for the picker "
      + "overlay so its Recent rows also surface the full path on hover",
  );
});
