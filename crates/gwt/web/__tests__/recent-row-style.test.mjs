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
