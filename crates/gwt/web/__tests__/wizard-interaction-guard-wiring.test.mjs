// Issue #2698 PR 1 (B7) — verify the launch-wizard surface wires up the
// interaction-guard primitive so destructive `renderLaunchWizard()` calls
// cannot fire while a user has a native `<select>` dropdown open. The
// integration is too entangled with the rest of the bundle to exercise via
// a full DOM harness here, so we assert structural invariants on the
// source text instead.
//
// SPEC-3064 Phase 3 (E5): the wizard surface (state, guard, chrome
// listeners) moved from app.js to launch-wizard-surface.js. Guard wiring
// patterns are pinned against the extracted module; the receive() case
// arms in app.js are pinned as thin delegates into the surface appliers.
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
const wizardSource = readFileSync(
  resolve(here, "../launch-wizard-surface.js"),
  "utf8",
);

test("wizard surface imports createInteractionGuard from /interaction-guard.js", () => {
  assert.match(
    wizardSource,
    /import\s*\{\s*createInteractionGuard\s*\}\s*from\s*["']\/interaction-guard\.js["']/,
    "expected named import of createInteractionGuard from /interaction-guard.js",
  );
});

test("wizard surface instantiates wizardInteractionGuard with an onFlush callback", () => {
  assert.match(
    wizardSource,
    /wizardInteractionGuard\s*=\s*createInteractionGuard\(\s*\{\s*[\s\S]{0,400}?onFlush\s*:/,
    "expected `wizardInteractionGuard = createInteractionGuard({ onFlush: ... })`",
  );
});

test("launch_wizard_state applier defers via wizardInteractionGuard before mutating state", () => {
  // The app.js case arm delegates into the surface applier, which must
  // contain a guard.defer({...}) check that returns early before
  // assigning `launchWizard = ...`.
  assert.match(
    appSource,
    /case\s+"launch_wizard_state":[\s\S]{0,300}?applyLaunchWizardStateEvent\(event\);\s*break;/,
    "expected app.js launch_wizard_state case to delegate into the wizard surface",
  );
  assert.match(
    wizardSource,
    /function\s+applyLaunchWizardStateEvent\(event\)\s*\{[\s\S]{0,400}?wizardInteractionGuard\.defer\([\s\S]{0,200}?\)\s*\)\s*\{\s*return;\s*\}[\s\S]{0,400}?launchWizard\s*=\s*event\.wizard/,
    "expected guard.defer() short-circuit before launchWizard mutation",
  );
});

test("launch_wizard_open_error applier defers via wizardInteractionGuard", () => {
  assert.match(
    appSource,
    /case\s+"launch_wizard_open_error":[\s\S]{0,300}?applyLaunchWizardOpenErrorEvent\(event\);\s*break;/,
    "expected app.js launch_wizard_open_error case to delegate into the wizard surface",
  );
  assert.match(
    wizardSource,
    /function\s+applyLaunchWizardOpenErrorEvent\(event\)\s*\{[\s\S]{0,400}?wizardInteractionGuard\.defer\([\s\S]{0,200}?\)\s*\)\s*\{\s*return;\s*\}/,
    "expected guard.defer() short-circuit in the launch_wizard_open_error applier",
  );
});

test("closeLaunchWizardLocal discards pending guard state before re-render", () => {
  // Local user-initiated close must not be undone by replaying a
  // deferred backend event — discard() drops both pending value
  // and active flag without invoking onFlush.
  assert.match(
    wizardSource,
    /function\s+closeLaunchWizardLocal\(\)\s*\{[\s\S]{0,500}?wizardInteractionGuard\.discard\(\)[\s\S]{0,140}?renderLaunchWizard\(\)/,
    "expected closeLaunchWizardLocal() to discard guard before render",
  );
});

test("wizardBody activates the guard on pointerdown over a <select>", () => {
  assert.match(
    wizardSource,
    /wizardBody\.addEventListener\(\s*"pointerdown"[\s\S]{0,400}?tagName\s*===\s*"SELECT"[\s\S]{0,200}?wizardInteractionGuard\.activate\(\)/,
    "expected delegated pointerdown listener that activates the guard",
  );
});

test("wizardBody releases the guard on change over a <select>", () => {
  assert.match(
    wizardSource,
    /wizardBody\.addEventListener\(\s*"change"[\s\S]{0,400}?tagName\s*===\s*"SELECT"[\s\S]{0,200}?wizardInteractionGuard\.release\(\)/,
    "expected delegated change listener that releases the guard",
  );
});

test("wizardBody releases the guard on focusout over a <select>", () => {
  assert.match(
    wizardSource,
    /wizardBody\.addEventListener\(\s*"focusout"[\s\S]{0,400}?tagName\s*===\s*"SELECT"[\s\S]{0,200}?wizardInteractionGuard\.release\(\)/,
    "expected delegated focusout listener that releases the guard",
  );
});

test("wizardModal releases the guard when Escape is pressed during interaction", () => {
  assert.match(
    wizardSource,
    /wizardModal\.addEventListener\(\s*"keydown"[\s\S]{0,400}?key\s*===\s*"Escape"[\s\S]{0,200}?wizardInteractionGuard\.release\(\)/,
    "expected Escape keydown to release the guard",
  );
});

test("wizard chrome actions release any active guard before dispatch", () => {
  assert.match(
    wizardSource,
    /function\s+releaseWizardInteractionGuardForChromeAction\(\)\s*\{[\s\S]{0,300}?wizardInteractionGuard\.isActive\(\)[\s\S]{0,150}?wizardInteractionGuard\.release\(\)[\s\S]{0,200}?return\s+Boolean\(launchWizard\s*\|\|\s*launchWizardOpenError\)/,
    "expected a chrome-action helper that releases the guard and reports whether wizard state remains",
  );
  assert.match(
    wizardSource,
    /function\s+closeLaunchWizardFromChrome\(\)\s*\{[\s\S]{0,160}?releaseWizardInteractionGuardForChromeAction\(\)/,
    "expected Cancel/Close to release pending guard state before dispatching",
  );
  assert.match(
    wizardSource,
    /wizardBackButton\.addEventListener\(\s*"click"[\s\S]{0,220}?releaseWizardInteractionGuardForChromeAction\(\)[\s\S]{0,300}?kind:\s*"back"/,
    "expected Back to release pending guard state before dispatching",
  );
  assert.match(
    wizardSource,
    /function\s+handleLaunchWizardSubmitFromChrome\(\)[\s\S]{0,260}?releaseWizardInteractionGuardForChromeAction\(\)[\s\S]{0,320}?kind:\s*"submit"/,
    "expected Submit handler to release pending guard state before dispatching",
  );
  assert.match(
    wizardSource,
    /if\s*\(wizardModal\.classList\.contains\("open"\)\)\s*\{[\s\S]{0,260}?releaseWizardInteractionGuardForChromeAction\(\)[\s\S]{0,350}?kind:\s*"cancel"/,
    "expected Escape-close to release pending guard state before dispatching",
  );
});

test("wizard start method actions release guard before dispatch", () => {
  assert.match(
    wizardSource,
    /const\s+handleStartMethodLaunchAction\s*=\s*\(\)\s*=>\s*\{[\s\S]{0,260}?releaseWizardInteractionGuardForChromeAction\(\)[\s\S]{0,360}?kind:\s*"use_start_method"/,
    "expected Start methods direct actions to release pending guard state before dispatching",
  );
});

test("wizard launch pointer fallback routes submit and start methods", () => {
  assert.match(
    wizardSource,
    /function\s+handleLaunchWizardSubmitFromChrome\(\)[\s\S]{0,500}?kind:\s*"submit"/,
    "expected Launch Wizard submit to be centralized for click and pointer fallback",
  );
  assert.match(
    wizardSource,
    /wizardSubmitButton\.addEventListener\(\s*"pointerup"[\s\S]{0,420}?handleLaunchWizardSubmitFromChrome\(\)/,
    "expected Create and Launch pointerup fallback to route through submit handler",
  );
  assert.match(
    wizardSource,
    /button\.addEventListener\(\s*"pointerup"[\s\S]{0,420}?handleStartMethodLaunchAction\(\)/,
    "expected Start method pointerup fallback to route through the same action handler",
  );
});

test("wizard launch actions expose local pending feedback", () => {
  assert.match(
    wizardSource,
    /let\s+launchWizardPendingAction\s*=\s*null/,
    "expected Launch Wizard to track a local pending action",
  );
  assert.match(
    wizardSource,
    /function\s+setLaunchWizardPendingAction\(\s*action[\s\S]{0,500}?launchWizardPendingAction\s*=/,
    "expected a helper to set Launch Wizard pending state",
  );
  assert.match(
    wizardSource,
    /function\s+clearLaunchWizardPendingAction\(\)[\s\S]{0,300}?launchWizardPendingAction\s*=\s*null/,
    "expected a helper to clear Launch Wizard pending state",
  );
  assert.match(
    wizardSource,
    /wizardModal\.classList\.toggle\(\s*"is-launch-pending"[\s\S]{0,220}?wizardDialog\.setAttribute\(\s*"aria-busy"/,
    "expected modal busy class and aria-busy to mirror pending launch actions",
  );
  assert.match(
    wizardSource,
    /wizardSubmitButton\.textContent\s*=\s*isLaunchSubmitPending[\s\S]{0,160}?"Launching\.\.\."/,
    "expected final launch submit to show an immediate Launching label",
  );
  assert.match(
    wizardSource,
    /createNode\(\s*"div",\s*"launch-note launch-pending-note",\s*launchWizard\.launch_materialization_message\s*\|\|\s*"Preparing worktree\.\.\."/,
    "expected pending submit to render visible progress copy in the modal",
  );
});

test("wizard backend launch materialization state preserves pending feedback", () => {
  assert.match(
    wizardSource,
    /function\s+applyLaunchWizardStateEvent\(event\)[\s\S]{0,700}?event\.wizard\?\.launch_materialization_pending[\s\S]{0,180}?clearLaunchWizardPendingAction\(\)/,
    "backend launch materialization state must not clear local pending chrome",
  );
  // Issue #3192 — this derivation runs BEFORE the opening/openError early
  // returns, so `launchWizard` is null for the Start Work / Launch Agent
  // pending states. It must read the field null-safely (optional chaining);
  // a bare `launchWizard.launch_materialization_pending` throws and the modal
  // never receives its `.open` class (the button silently does nothing).
  assert.match(
    wizardSource,
    /const\s+isLaunchMaterializationPending\s*=\s*Boolean\(\s*launchWizard\?\.launch_materialization_pending,?\s*\)/,
    "expected renderer to derive backend launch materialization pending state null-safely",
  );
  assert.match(
    wizardSource,
    /launchWizard\.launch_materialization_message\s*\|\|\s*"Preparing worktree\.\.\."/,
    "expected backend materialization message to render as visible progress copy",
  );
});
