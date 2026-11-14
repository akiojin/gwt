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
import { createInteractionGuard } from "../interaction-guard.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
// SPEC-3064 Phase 3 (E4): the guard instance (and the System panel state it
// re-renders) moved into the extracted settings surface; app.js keeps the
// document-level listener wiring and the receive() defer call sites via
// destructured same-name consts.
const settingsSource = readFileSync(resolve(here, "../settings-surface.js"), "utf8");

function extractFunctionSource(source, name) {
  const start = source.indexOf(`function ${name}(`);
  assert.notEqual(start, -1, `missing function ${name}`);
  const braceStart = source.indexOf("{", start);
  let depth = 0;
  for (let index = braceStart; index < source.length; index += 1) {
    if (source[index] === "{") depth += 1;
    if (source[index] === "}") {
      depth -= 1;
      if (depth === 0) return source.slice(start, index + 1);
    }
  }
  assert.fail(`unterminated function ${name}`);
}

function loadAppFunctions(names, resultName) {
  const source = names
    .map((name) => extractFunctionSource(appSource, name))
    .join("\n");
  return Function(`"use strict"; ${source}; return ${resultName};`)();
}

function settingsControl(tagName, className) {
  return {
    tagName,
    classList: { contains: (candidate) => candidate === className },
  };
}

test("settings surface instantiates systemSettingsInteractionGuard via the factory", () => {
  assert.match(
    settingsSource,
    /systemSettingsInteractionGuard\s*=\s*createInteractionGuard\(\s*\{\s*[\s\S]{0,400}?onFlush\s*:/,
    "expected `systemSettingsInteractionGuard = createInteractionGuard({ onFlush: ... })`",
  );
});

test("system_settings handler defers via guard before mutating state", () => {
  assert.match(
    appSource,
    /case\s+"system_settings":[\s\S]{0,1400}?systemSettingsInteractionGuard\.defer\([\s\S]{0,700}?\)\s*\)\s*\{\s*break;\s*\}[\s\S]{0,400}?systemSettingsState\.language\s*=\s*event\.language/,
    "expected guard.defer() short-circuit before language mutation in system_settings case",
  );
});

test("system_settings_updated handler defers via guard before mutating state", () => {
  assert.match(
    appSource,
    /case\s+"system_settings_updated":[\s\S]{0,1400}?systemSettingsInteractionGuard\.defer\([\s\S]{0,700}?\)\s*\)\s*\{\s*break;\s*\}/,
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

test("settings interaction target includes select and numeric input primitives", () => {
  assert.match(
    appSource,
    /function\s+isSystemSettingsInteractionTarget\([\s\S]{0,500}?tagName\s*===\s*"SELECT"[\s\S]{0,200}?settings-select[\s\S]{0,300}?tagName\s*===\s*"INPUT"[\s\S]{0,200}?settings-input/,
    "expected the shared guard predicate to cover System select and input primitives",
  );
});

test("document-level pointerdown and focusin activate the settings guard", () => {
  assert.match(
    appSource,
    /document\.addEventListener\(\s*"pointerdown"[\s\S]{0,300}?isSystemSettingsInteractionTarget\(target\)[\s\S]{0,160}?systemSettingsInteractionGuard\.activate\(\)/,
    "expected pointer interaction to activate the settings guard",
  );
  assert.match(
    appSource,
    /document\.addEventListener\(\s*"focusin"[\s\S]{0,300}?isSystemSettingsInteractionTarget\(target\)[\s\S]{0,160}?systemSettingsInteractionGuard\.activate\(\)/,
    "expected keyboard focus on a settings input to activate the guard",
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
    /document\.addEventListener\(\s*"focusout"[\s\S]{0,500}?shouldReleaseSystemSettingsInteraction\(event\)[\s\S]{0,160}?systemSettingsInteractionGuard\.release\(\)/,
    "expected document focusout listener that releases on settings selects and inputs",
  );
});

test("protected control transitions retain deferred echoes until the next commit", () => {
  const shouldRelease = loadAppFunctions(
    [
      "isSystemSettingsInteractionTarget",
      "shouldReleaseSystemSettingsInteraction",
    ],
    "shouldReleaseSystemSettingsInteraction",
  );
  const number = settingsControl("INPUT", "settings-input");
  const checkbox = settingsControl("INPUT", "settings-checkbox");
  const select = settingsControl("SELECT", "settings-select");

  assert.equal(shouldRelease({ target: number, relatedTarget: checkbox }), false);
  assert.equal(shouldRelease({ target: select, relatedTarget: number }), false);
  assert.equal(shouldRelease({ target: number, relatedTarget: null }), true);

  const flushed = [];
  const guard = createInteractionGuard({ onFlush: (value) => flushed.push(value) });
  guard.activate();
  guard.defer("deferred backend echo");
  if (shouldRelease({ target: number, relatedTarget: checkbox })) guard.release();
  assert.equal(guard.isActive(), true);
  assert.equal(guard.hasPendingValue(), true);

  let nextCommitReached = false;
  nextCommitReached = true;
  guard.release();
  assert.equal(nextCommitReached, true);
  assert.deepEqual(flushed, ["deferred backend echo"]);
});

test("document-level Escape keydown releases the guard while active", () => {
  assert.match(
    appSource,
    /document\.addEventListener\(\s*"keydown"[\s\S]{0,400}?key\s*===\s*"Escape"[\s\S]{0,200}?systemSettingsInteractionGuard\.(?:isActive|release)/,
    "expected document keydown listener that releases the guard on Escape",
  );
});
