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

test("app.js instantiates systemSettingsInteractionGuard via the factory", () => {
  assert.match(
    appSource,
    /systemSettingsInteractionGuard\s*=\s*createInteractionGuard\(\s*\{\s*[\s\S]{0,400}?onFlush\s*:/,
    "expected `systemSettingsInteractionGuard = createInteractionGuard({ onFlush: ... })`",
  );
});

test("system_settings handler defers via guard before mutating state", () => {
  assert.match(
    appSource,
    /case\s+"system_settings":[\s\S]{0,800}?systemSettingsInteractionGuard\.defer\([\s\S]{0,360}?\)\s*\)\s*\{\s*break;\s*\}[\s\S]{0,400}?systemSettingsState\.language\s*=\s*event\.language/,
    "expected guard.defer() short-circuit before language mutation in system_settings case",
  );
});

test("system_settings_updated handler defers via guard before mutating state", () => {
  assert.match(
    appSource,
    /case\s+"system_settings_updated":[\s\S]{0,800}?systemSettingsInteractionGuard\.defer\([\s\S]{0,360}?\)\s*\)\s*\{\s*break;\s*\}/,
    "expected guard.defer() short-circuit in system_settings_updated case",
  );
});

test("system_settings_error handler defers via guard before mutating state", () => {
  assert.match(
    appSource,
    /case\s+"system_settings_error":[\s\S]{0,800}?systemSettingsInteractionGuard\.defer\([\s\S]{0,200}?\)\s*\)\s*\{\s*break;\s*\}/,
    "expected guard.defer() short-circuit in system_settings_error case",
  );
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
