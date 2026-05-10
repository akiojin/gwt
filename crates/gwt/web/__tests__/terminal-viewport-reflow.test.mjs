// SPEC-2008 Phase 24 / T-184..T-186 — terminal viewport reflow on resize
// and tab visibility transitions. These tests pin the contract via
// source-level assertions on app.js. Behavior runs through the IIFE-scoped
// `fitTerminal` / `canRefreshTerminalViewport` / `scheduleTerminalFocusActivation`
// helpers, so we assert that the listener / activation hooks call into them
// rather than re-implementing xterm.js + window state inside the test.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

test("window.resize listener fans out fitTerminal(persist=true) across visible terminals (T-187)", () => {
  // The resize handler block must (1) keep the existing
  // `syncMaximizedWindowsToViewport()` call, (2) iterate over `terminalMap`
  // keys, (3) skip via `canRefreshTerminalViewport`, and (4) call
  // `fitTerminal(windowId, true)` so backend PTY receives
  // `UpdateWindowGeometry`.
  const resizeBlockMatch = appSource.match(
    /window\.addEventListener\("resize",\s*\(\)\s*=>\s*\{([\s\S]*?)\n\s*\}\);/,
  );
  assert.ok(resizeBlockMatch, "window.resize listener block must exist");
  const resizeBody = resizeBlockMatch[1];

  assert.match(resizeBody, /syncMaximizedWindowsToViewport\(\);/);
  assert.match(
    resizeBody,
    /for\s*\(\s*const\s+windowId\s+of\s+terminalMap\.keys\(\)\s*\)/,
    "resize listener must iterate over terminalMap windows",
  );
  assert.match(
    resizeBody,
    /canRefreshTerminalViewport\(windowId\)/,
    "resize listener must skip windows that fail the refresh predicate",
  );
  assert.match(
    resizeBody,
    /fitTerminal\(windowId,\s*true\)/,
    "resize listener must call fitTerminal with persist=true so backend PTY gets UpdateWindowGeometry",
  );
});

test("renderWindows reflows hidden -> visible terminal tabs after activation (T-188)", () => {
  // The transition detection block lives inside the workspace render. We
  // require an "wasHidden" capture before mutating element.hidden plus a
  // call to scheduleTerminalFocusActivation for any window that flips
  // hidden -> visible and has a live terminal runtime.
  assert.match(
    appSource,
    /const\s+wasHidden\s*=\s*element\.hidden;/,
    "render must capture pre-mutation hidden state for transition detection",
  );
  assert.match(
    appSource,
    /const\s+shouldHide\s*=\s*!visibleWindowData\(windowData\);/,
    "render must compute the new hidden state from workspace data",
  );
  assert.match(
    appSource,
    /element\.hidden\s*=\s*shouldHide;/,
    "render must apply the new hidden state to the element",
  );
  assert.match(
    appSource,
    /if\s*\(wasHidden\s*&&\s*!shouldHide\s*&&\s*terminalMap\.has\(windowData\.id\)\)/,
    "render must detect hidden -> visible transitions for terminal-bearing windows",
  );
  assert.match(
    appSource,
    /scheduleTerminalFocusActivation\(windowId\)/,
    "render must call scheduleTerminalFocusActivation on hidden -> visible transitions",
  );
});

test("canRefreshTerminalViewport refuses refresh while the host element is .hidden (T-186)", () => {
  // Match from the function header to the closing brace at column 6
  // (function body indentation in the IIFE). element.hidden must
  // short-circuit before the workspace minimized check so display:none
  // tabs skip both fit and refresh.
  const predicateMatch = appSource.match(
    /function\s+canRefreshTerminalViewport\(windowId\)\s*\{([\s\S]*?)\n      \}/,
  );
  assert.ok(predicateMatch, "canRefreshTerminalViewport function must exist");
  const body = predicateMatch[1];

  assert.match(body, /windowMap\.get\(windowId\)/);
  assert.match(
    body,
    /element\.hidden/,
    "predicate must consult element.hidden so display:none tabs skip fit/refresh",
  );
  assert.match(
    body,
    /workspaceWindowById\(windowId\)\?\.minimized/,
    "predicate must keep the existing minimized short-circuit",
  );
  // Order: element.hidden short-circuit must precede the minimized check.
  const hiddenIdx = body.indexOf("element.hidden");
  const minimizedIdx = body.indexOf("workspaceWindowById(windowId)?.minimized");
  assert.ok(
    hiddenIdx >= 0 && minimizedIdx > hiddenIdx,
    "element.hidden short-circuit must come before the minimized check",
  );
});
