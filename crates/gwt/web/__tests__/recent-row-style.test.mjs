// Issue #2746 — regression test: Recent project rows in the project picker
// overlay must declare `user-select: none` so wry/WebKit does not promote
// pointerdown into a text-selection drag and swallow the click. This is the
// same root cause pattern Issue #2744 fixed for `.project-tab`.
//
// SPEC-2013 Phase 8: the Open Project split-button dropdown (and its mirrored
// `.split-button-menu-recent*` Recent rows) was retired in favor of the
// consolidated `Projects ▾` switcher, so the dropdown-specific assertions are
// gone. The picker overlay Recent rows remain and keep this protection.

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

test(".recent-project-row declares user-select: none", () => {
  const body = ruleBlock(".recent-project-row");
  assert.match(
    body,
    /user-select\s*:\s*none\s*;/,
    "expected user-select: none on .recent-project-row "
      + "(see Issue #2746 — picker overlay Recent rows share the same "
      + "span-child pattern that would otherwise drag-select and swallow clicks)",
  );
});

// Issue #2746 follow-up — once meta truncates with ellipsis the user can no
// longer tell which directory a Recent row points to, so each row's
// `title` attribute carries the full path for the native hover/focus
// tooltip. The text we point at is the same `${kind} · ${path}` payload as
// the meta span, kept identical so tooltip and visible text never disagree.
// SPEC-3064 Phase 3 (E7): the Recent Projects renderer moved from app.js
// to project-shell-surface.js; the tooltip pin reads the surface module.
const projectShellJs = readFileSync(
  resolve(here, "..", "project-shell-surface.js"),
  "utf8",
);

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
    "expected renderRecentProjects to assign "
      + "`row.title = `${project.kind} · ${project.path}`` for the picker "
      + "overlay so its Recent rows surface the full path on hover",
  );
});
