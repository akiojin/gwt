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

test("project tab agent state cue is static and visually quiet", () => {
  const baseBlock = findRuleBlock(".project-tab-state-cue");
  assert.ok(
    baseBlock !== null,
    "`.project-tab-state-cue` rule not found in app.css",
  );
  assert.match(
    baseBlock,
    /min-width\s*:\s*22px\s*;?/,
    "state cue must stay compact so project tabs do not feel visually heavy",
  );
  assert.match(
    baseBlock,
    /background\s*:\s*transparent\s*;?/,
    "state cue must not use a filled badge background",
  );
  assert.match(
    baseBlock,
    /border\s*:\s*0\s*;?/,
    "state cue must not render as a boxed pill",
  );
  assert.match(
    baseBlock,
    /font-size\s*:\s*10px\s*;?/,
    "state cue label must stay compact inside project tabs",
  );
  assert.match(
    css,
    /\.project-tab\[data-agent-state\]\s*\{[^}]*border-left:\s*3px\s+solid/,
    "project tab state must be carried by a slim left rail on the tab itself",
  );
  for (const [state, token] of [
    ["run", "--color-state-active"],
    ["start", "--color-state-idle"],
    ["block", "--color-state-blocked"],
  ]) {
    assert.match(
      css,
      new RegExp(
        String.raw`\.project-tab-state-cue\[data-state="${state}"\]\s*\{[^}]*var\(${token}\)`,
      ),
      `${state} cue must use ${token}`,
    );
  }
  assert.doesNotMatch(
    css,
    /@keyframes\s+project-tab-agent-running-pulse/,
    "project tab state cues must not rely on a running-agent blink animation",
  );
  assert.match(
    css,
    /\.project-tab-state-cue\[data-state="run"\]\s*\{[^}]*animation\s*:\s*none\s*;?/,
    "RUN cue must explicitly stay static",
  );
});
