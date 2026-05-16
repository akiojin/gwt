// Issue #2744 — text-selection drag on `.project-tab` label swallows
// click events, so clicking the project name does not switch tabs.
// Adding `user-select: none` to the parent rule cascades to the label
// span and prevents drag-selection from intercepting clicks. This test
// guards against the rule being removed in future refactors.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import assert from "node:assert/strict";
import test from "node:test";

const here = path.dirname(fileURLToPath(import.meta.url));
const cssPath = path.resolve(here, "..", "styles", "app.css");
const css = readFileSync(cssPath, "utf8");

function findRuleBlock(selector) {
  const startToken = selector + " {";
  const startIdx = css.indexOf(startToken);
  if (startIdx < 0) {
    return null;
  }
  const bodyStart = startIdx + startToken.length;
  const endIdx = css.indexOf("}", bodyStart);
  if (endIdx < 0) {
    return null;
  }
  return css.slice(bodyStart, endIdx);
}

test(".project-tab CSS rule disables user-select so label clicks register", () => {
  const block = findRuleBlock(".project-tab");
  assert.ok(block !== null, "`.project-tab` rule not found in app.css");
  assert.match(
    block,
    /user-select\s*:\s*none\s*;?/,
    "`.project-tab` must declare `user-select: none` to prevent text-selection drag from swallowing click events on label text (Issue #2744)",
  );
});
