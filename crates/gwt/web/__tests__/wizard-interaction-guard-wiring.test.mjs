// Issue #2698 PR 1 (B7) — verify the launch-wizard surface in
// `app.js` wires up the interaction-guard primitive so destructive
// `renderLaunchWizard()` calls cannot fire while a user has a native
// `<select>` dropdown open. The integration is too entangled with
// the rest of the bundle to exercise via a full DOM harness here,
// so we assert structural invariants on the source text instead.
//
// If these patterns ever stop matching, run the wizard manually on
// Windows / macOS and confirm a native `<select>` dropdown still
// commits its selection even when `launch_wizard_state` arrives
// mid-interaction. The patterns are minimal markers — they should
// remain stable across reasonable refactors.

import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

test("app.js imports createInteractionGuard from /interaction-guard.js", () => {
  assert.match(
    appSource,
    /import\s*\{\s*createInteractionGuard\s*\}\s*from\s*["']\/interaction-guard\.js["']/,
    "expected named import of createInteractionGuard from /interaction-guard.js",
  );
});

test("app.js instantiates wizardInteractionGuard with an onFlush callback", () => {
  assert.match(
    appSource,
    /wizardInteractionGuard\s*=\s*createInteractionGuard\(\s*\{\s*[\s\S]{0,400}?onFlush\s*:/,
    "expected `wizardInteractionGuard = createInteractionGuard({ onFlush: ... })`",
  );
});

test("launch_wizard_state handler defers via wizardInteractionGuard before mutating state", () => {
  // Window: locate the `case "launch_wizard_state":` block and
  // assert it contains a guard.defer({...}) check that returns
  // early via `break;` before assigning `launchWizard = ...`.
  assert.match(
    appSource,
    /case\s+"launch_wizard_state":[\s\S]{0,400}?wizardInteractionGuard\.defer\([\s\S]{0,200}?\)\s*\)\s*\{\s*break;\s*\}[\s\S]{0,200}?launchWizard\s*=\s*event\.wizard/,
    "expected guard.defer() short-circuit before launchWizard mutation",
  );
});

test("launch_wizard_open_error handler defers via wizardInteractionGuard", () => {
  assert.match(
    appSource,
    /case\s+"launch_wizard_open_error":[\s\S]{0,400}?wizardInteractionGuard\.defer\([\s\S]{0,200}?\)\s*\)\s*\{\s*break;\s*\}/,
    "expected guard.defer() short-circuit in launch_wizard_open_error case",
  );
});

test("closeLaunchWizardLocal discards pending guard state before re-render", () => {
  // Local user-initiated close must not be undone by replaying a
  // deferred backend event — discard() drops both pending value
  // and active flag without invoking onFlush.
  assert.match(
    appSource,
    /function\s+closeLaunchWizardLocal\(\)\s*\{[\s\S]{0,300}?wizardInteractionGuard\.discard\(\)[\s\S]{0,100}?renderLaunchWizard\(\)/,
    "expected closeLaunchWizardLocal() to discard guard before render",
  );
});

test("wizardBody activates the guard on pointerdown over a <select>", () => {
  assert.match(
    appSource,
    /wizardBody\.addEventListener\(\s*"pointerdown"[\s\S]{0,400}?tagName\s*===\s*"SELECT"[\s\S]{0,200}?wizardInteractionGuard\.activate\(\)/,
    "expected delegated pointerdown listener that activates the guard",
  );
});

test("wizardBody releases the guard on change over a <select>", () => {
  assert.match(
    appSource,
    /wizardBody\.addEventListener\(\s*"change"[\s\S]{0,400}?tagName\s*===\s*"SELECT"[\s\S]{0,200}?wizardInteractionGuard\.release\(\)/,
    "expected delegated change listener that releases the guard",
  );
});

test("wizardBody releases the guard on focusout over a <select>", () => {
  assert.match(
    appSource,
    /wizardBody\.addEventListener\(\s*"focusout"[\s\S]{0,400}?tagName\s*===\s*"SELECT"[\s\S]{0,200}?wizardInteractionGuard\.release\(\)/,
    "expected delegated focusout listener that releases the guard",
  );
});

test("wizardModal releases the guard when Escape is pressed during interaction", () => {
  assert.match(
    appSource,
    /wizardModal\.addEventListener\(\s*"keydown"[\s\S]{0,400}?key\s*===\s*"Escape"[\s\S]{0,200}?wizardInteractionGuard\.release\(\)/,
    "expected Escape keydown to release the guard",
  );
});
