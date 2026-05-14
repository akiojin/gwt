// Issue #2704 — verify the workspace-render surface in `app.js` wires up
// the terminal-focus guard so `scheduleTerminalFocusActivation` skips its
// xterm `terminal.focus()` step while a modal is open or while a text
// input owns focus.
//
// The integration is too entangled with the rest of the bundle to drive
// via a full DOM harness here, so we assert structural invariants on the
// source text instead — mirroring the
// `wizard-interaction-guard-wiring.test.mjs` pattern.
//
// If these patterns ever stop matching, run the Clone Project modal
// manually while a background agent is Running and confirm typing still
// reaches the URL/Search input. The patterns are minimal markers — they
// should remain stable across reasonable refactors.

import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const appJs = readFileSync(resolve(here, "..", "app.js"), "utf8");

test("app.js imports shouldSkipTerminalFocusActivation from /clone-modal-focus-guard.js", () => {
  assert.match(
    appJs,
    /import\s*\{\s*shouldSkipTerminalFocusActivation\s*\}\s*from\s*["']\/clone-modal-focus-guard\.js["']/,
    "expected named import of shouldSkipTerminalFocusActivation from /clone-modal-focus-guard.js",
  );
});

// Extract the `scheduleTerminalFocusActivation` body once so both
// follow-up assertions look at the same scoped slice instead of
// re-matching against the whole bundle (which trips the lazy regex
// quantifier when neighbouring helpers grow over time).
function scheduleTerminalFocusActivationBody() {
  const match = appJs.match(
    /function\s+scheduleTerminalFocusActivation\b[\s\S]*?\n\s{6}\}\s*\n/,
  );
  assert.ok(
    match,
    "expected to locate scheduleTerminalFocusActivation function body",
  );
  return match[0];
}

test("scheduleTerminalFocusActivation derives shouldFocus via shouldSkipTerminalFocusActivation", () => {
  const body = scheduleTerminalFocusActivationBody();
  // The guard must run inside the rAF callback so we sample the latest
  // modal/activeElement state at activation time, not at scheduling time.
  assert.match(
    body,
    /shouldSkipTerminalFocusActivation\s*\(/,
    "expected scheduleTerminalFocusActivation to call shouldSkipTerminalFocusActivation()",
  );
  // The result must reach `runTerminalActivationSequence` either via a
  // `shouldFocus,` shorthand or an explicit `shouldFocus: <expr>` pair.
  assert.match(
    body,
    /runTerminalActivationSequence\(\s*\{[\s\S]*?shouldFocus(?:\s*[,:])/,
    "expected runTerminalActivationSequence to receive a shouldFocus value",
  );
});

test("scheduleTerminalFocusActivation no longer hardcodes shouldFocus: true", () => {
  // Belt-and-braces: ensure the previous hardcoded form is gone so a
  // future refactor cannot quietly re-introduce the focus-stealing
  // regression without flipping this test.
  const body = scheduleTerminalFocusActivationBody();
  assert.doesNotMatch(
    body,
    /shouldFocus\s*:\s*true\b/,
    "scheduleTerminalFocusActivation must not hardcode shouldFocus: true anymore",
  );
});

test("clone-modal-focus-guard module file is reachable from the web bundle root", () => {
  // The Rust bundle registration (embedded_web.rs::ROOT_JS_MODULE_ASSETS)
  // and the Playwright `embedded-frontend.ts` ROOT_MODULES set both
  // serve this path. We just sanity-check the file exists alongside its
  // sibling modules — the deeper bundle registration is covered by the
  // Rust-side tests.
  const guardPath = resolve(here, "..", "clone-modal-focus-guard.js");
  const source = readFileSync(guardPath, "utf8");
  assert.match(
    source,
    /export\s+function\s+shouldSkipTerminalFocusActivation\b/,
    "clone-modal-focus-guard.js must export shouldSkipTerminalFocusActivation",
  );
});
