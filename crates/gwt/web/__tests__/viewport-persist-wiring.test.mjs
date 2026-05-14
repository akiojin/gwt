// Issue #2698 PR 2 (B1) — verify app.js wires the viewport-persist
// throttle into the persist path and the pointerup pan-end flush.

import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

test("app.js imports createViewportPersistThrottle", () => {
  assert.match(
    appSource,
    /import\s*\{\s*createViewportPersistThrottle\s*\}\s*from\s*["']\/viewport-persist-throttle\.js["']/,
    "expected named import of createViewportPersistThrottle",
  );
});

test("app.js instantiates persistViewportThrottle via the factory", () => {
  assert.match(
    appSource,
    /persistViewportThrottle\s*=\s*createViewportPersistThrottle\(/,
    "expected `persistViewportThrottle = createViewportPersistThrottle(...)` instantiation",
  );
});

test("persistViewport() forwards through the throttle (no direct send)", () => {
  // The throttle's send callback is the only place that should
  // emit `update_viewport` from the persist path. The function
  // body itself must just schedule.
  assert.match(
    appSource,
    /function\s+persistViewport\(\)\s*\{\s*persistViewportThrottle\.schedule\(viewport\)\s*;?\s*\}/,
    "expected persistViewport() to delegate to throttle.schedule(viewport)",
  );
});

test("flushPersistViewport() forces an immediate throttle drain", () => {
  assert.match(
    appSource,
    /function\s+flushPersistViewport\(\)\s*\{[\s\S]*?persistViewportThrottle\.schedule\(viewport\)[\s\S]*?persistViewportThrottle\.flushNow\(\)/,
    "expected flushPersistViewport() to schedule then flushNow",
  );
});

test("pointerup pan-end calls flushPersistViewport (not a raw send)", () => {
  // After the pan-state branch, flushPersistViewport() must run
  // and the legacy inline `send({ kind: "update_viewport", ... })`
  // must be gone from that branch.
  assert.match(
    appSource,
    /if\s*\(\s*panState\s*&&\s*panState\.pointerId\s*===\s*event\.pointerId\s*\)\s*\{[\s\S]{0,400}?flushPersistViewport\(\)[\s\S]{0,200}?panState\s*=\s*null;/,
    "expected pointerup pan-end branch to call flushPersistViewport()",
  );
});
