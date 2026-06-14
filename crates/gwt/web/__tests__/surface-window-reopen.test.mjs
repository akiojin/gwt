import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

function extractFunctionBody(source, name) {
  const start = source.indexOf(`function ${name}(`);
  assert.notEqual(start, -1, `expected function ${name} in app.js`);
  const paramsOpen = source.indexOf("(", start);
  assert.notEqual(paramsOpen, -1, `expected function ${name} parameters`);
  let parenDepth = 0;
  let paramsClose = -1;
  for (let i = paramsOpen; i < source.length; i += 1) {
    const char = source[i];
    if (char === "(") parenDepth += 1;
    if (char === ")") {
      parenDepth -= 1;
      if (parenDepth === 0) {
        paramsClose = i;
        break;
      }
    }
  }
  assert.notEqual(paramsClose, -1, `expected function ${name} parameter close`);
  const open = source.indexOf("{", paramsClose);
  assert.notEqual(open, -1, `expected function ${name} body`);
  let depth = 0;
  for (let i = open; i < source.length; i += 1) {
    const char = source[i];
    if (char === "{") depth += 1;
    if (char === "}") {
      depth -= 1;
      if (depth === 0) {
        return source.slice(open + 1, i);
      }
    }
  }
  assert.fail(`expected function ${name} body to close`);
}

test("surface preset reopen focuses and reveals existing windows instead of creating duplicates", () => {
  const focusBody = extractFunctionBody(appSource, "focusOrSpawnPreset");

  assert.doesNotMatch(
    focusBody,
    /&&\s*!\s*w\.minimized/,
    "existing minimized surface windows must be reusable",
  );
  assert.match(
    focusBody,
    /openExistingSurfaceWindow\(\s*existing\s*\)/,
    "existing surface windows must use the shared restore/focus helper",
  );
  assert.match(
    focusBody,
    /kind:\s*"create_window"[\s\S]*preset[\s\S]*bounds:\s*visibleBounds\(\)/,
    "missing surface windows should still be created at the current viewport",
  );
});

test("existing surface helper restores minimized windows and centers focus with visible bounds", () => {
  const helperBody = extractFunctionBody(appSource, "openExistingSurfaceWindow");

  assert.match(
    helperBody,
    /focusWindowLocally\(\s*windowData\.id\s*\)/,
    "frontend should provide immediate local focus feedback",
  );
  assert.match(
    helperBody,
    /kind:\s*"focus_window"[\s\S]*id:\s*windowData\.id[\s\S]*bounds:\s*visibleBounds\(\)/,
    "backend focus must include visibleBounds so offscreen windows are centered",
  );
  assert.match(
    helperBody,
    /windowData\.minimized[\s\S]*kind:\s*"restore_window"[\s\S]*id:\s*windowData\.id/,
    "minimized existing surface windows must be restored",
  );
});

test("existing grouped surface tabs are activated before focus", () => {
  const helperBody = extractFunctionBody(appSource, "openExistingSurfaceWindow");

  assert.match(
    helperBody,
    /windowData\.tab_group_id[\s\S]*kind:\s*"activate_window_tab"[\s\S]*id:\s*windowData\.id/,
    "inactive grouped surface tabs must be activated before focus",
  );
  assert.ok(
    helperBody.indexOf('kind: "activate_window_tab"') < helperBody.indexOf('kind: "focus_window"'),
    "tab activation must be sent before focus_window so the requested tab is revealed",
  );
});

test("Add Window surface buttons use the same reopen path as rail and palette commands", () => {
  const modalLoop = appSource.match(
    /for\s*\(\s*const\s+button\s+of\s+modal\.querySelectorAll\("\[data-preset\]"\)\s*\)\s*\{([\s\S]*?)\n\s*\}\n\s*\n\s*\/\/ SPEC-2356 "Surface Deck"/,
  );
  assert.ok(modalLoop, "expected Add Window preset button loop");
  assert.match(
    modalLoop[1],
    /focusOrSpawnPreset\(\s*button\.dataset\.preset\s*\)/,
    "Add Window buttons must reuse existing surface windows before creating",
  );
  assert.doesNotMatch(
    modalLoop[1],
    /kind:\s*"create_window"/,
    "Add Window buttons must not bypass the shared surface reopen helper",
  );
});

test("work surface aliases are normalized before singleton lookup", () => {
  const normalizeBody = extractFunctionBody(appSource, "normalizeSurfacePreset");
  assert.match(
    normalizeBody,
    /preset\s*===\s*"branches"[\s\S]*preset\s*===\s*"workspace"[\s\S]*return\s+"work"/,
    "branches and legacy workspace presets must normalize to work",
  );

  const focusBody = extractFunctionBody(appSource, "focusOrSpawnPreset");
  assert.match(
    focusBody,
    /preset\s*=\s*normalizeSurfacePreset\(preset\)/,
    "requested preset must be canonicalized before lookup",
  );
  assert.match(
    focusBody,
    /normalizeSurfacePreset\(w\.preset\)\s*===\s*preset/,
    "existing legacy workspace windows must match canonical work requests",
  );
});

test("session presets are excluded from singleton surface reuse", () => {
  const predicateBody = extractFunctionBody(appSource, "isSingletonSurfacePreset");
  for (const preset of ["agent", "shell", "claude", "codex"]) {
    assert.match(
      predicateBody,
      new RegExp(`"${preset}"`),
      `expected ${preset} to be named in the session-preset exclusion`,
    );
  }
  assert.match(
    predicateBody,
    /return\s+!\s*sessionPresets\.has\(preset\)/,
    "only non-session surface presets should be reused as singletons",
  );

  const focusBody = extractFunctionBody(appSource, "focusOrSpawnPreset");
  assert.match(
    focusBody,
    /isSingletonSurfacePreset\(preset\)[\s\S]*allWindows\.find/,
    "focusOrSpawnPreset must gate reuse behind the singleton surface predicate",
  );
});
