// Issue #2698 PR 4 — verify the System Settings Output Language
// `<select>` is protected by the same interaction-guard pattern as
// the Launch Wizard. Settings windows can stack and reflow, so the
// listeners delegate from `document` and filter by the unique
// `settings-select` class.
//
// If these patterns ever stop matching, run a manual smoke: open
// Settings on Windows, click the Output Language `<select>` open,
// have backend echo `system_settings_updated` (e.g. from another
// client), and confirm the open dropdown survives + commits.

import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
// SPEC-3064 Phase 3 (E4): the guard instance (and the System panel state it
// re-renders) moved into the extracted settings surface; app.js keeps the
// document-level listener wiring; the subscribed render owns deferral via
// destructured same-name consts.
const settingsSource = readFileSync(resolve(here, "../settings-surface.js"), "utf8");

test("settings surface instantiates systemSettingsInteractionGuard via the factory", () => {
  assert.match(
    settingsSource,
    /systemSettingsInteractionGuard\s*=\s*createInteractionGuard\(\s*\{\s*[\s\S]{0,400}?onFlush\s*:/,
    "expected `systemSettingsInteractionGuard = createInteractionGuard({ onFlush: ... })`",
  );
});

test("System replies commit immediately and defer destructive rendering only", () => {
  assert.match(appSource, /case "system_settings":\s*case "system_settings_updated":\s*applySystemSettingsEvent\(event\);/);
  assert.match(appSource, /case "system_settings_error":\s*applyGlobalUpdateSettings\(event\);\s*applySystemSettingsEvent\(event\);/);
  assert.match(settingsSource, /function applySystemSettingsEvent\(event\) \{\s*systemSettingsModel\.update\(/);
  assert.match(settingsSource, /systemSettingsModel\.subscribe\(state => state, state => \{\s*renderSystemPanelInAllSettingsWindows\(state\)/);
  assert.match(settingsSource, /onFlush: \(\) => renderSystemPanelInAllSettingsWindows\(\)/);
  assert.match(settingsSource, /const deferred = systemSettingsInteractionGuard\.defer\(true\);/);
  assert.match(settingsSource, /if \(deferred\) renderSystemPanelStatus\(panel, state\);\s*else renderSystemPanel\(panel, state\);/);
});

test("document-level pointerdown activates the guard for settings-select", () => {
  assert.match(
    appSource,
    /document\.addEventListener\(\s*"pointerdown"[\s\S]{0,400}?tagName\s*===\s*"SELECT"[\s\S]{0,200}?settings-select[\s\S]{0,200}?systemSettingsInteractionGuard\.activate\(\)/,
    "expected document pointerdown listener that activates on .settings-select",
  );
});

test("document-level change releases the guard for settings-select", () => {
  assert.match(
    appSource,
    /document\.addEventListener\(\s*"change"[\s\S]{0,400}?tagName\s*===\s*"SELECT"[\s\S]{0,200}?settings-select[\s\S]{0,200}?systemSettingsInteractionGuard\.release\(\)/,
    "expected document change listener that releases on .settings-select",
  );
});

test("document-level focusout releases the guard for settings-select", () => {
  assert.match(
    appSource,
    /document\.addEventListener\(\s*"focusout"[\s\S]{0,400}?tagName\s*===\s*"SELECT"[\s\S]{0,200}?settings-select[\s\S]{0,200}?systemSettingsInteractionGuard\.release\(\)/,
    "expected document focusout listener that releases on .settings-select",
  );
});

test("document-level Escape keydown releases the guard while active", () => {
  assert.match(
    appSource,
    /document\.addEventListener\(\s*"keydown"[\s\S]{0,400}?key\s*===\s*"Escape"[\s\S]{0,200}?systemSettingsInteractionGuard\.(?:isActive|release)/,
    "expected document keydown listener that releases the guard on Escape",
  );
});
