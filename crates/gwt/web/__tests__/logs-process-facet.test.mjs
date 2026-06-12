// SPEC-2019 Amendment 2026-05-20 — Process kind facet behavior tests.
//
// app.js owns the Logs window controller (no separate module to import),
// so this test asserts the source-level invariants needed for the facet:
// the `logStateMap` entry carries `processKind`, the filter helper exists
// and AND-combines with severity + keyword, the render path syncs the
// chip element value, and the chip exposes the canonical KIND set.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
// SPEC-3064 Phase 3 (E6c): the Logs renderers moved from app.js into
// board-logs-surface.js; source-pattern asserts scan both files so the
// dispatcher case arms (app.js) and the moved renderers stay covered.
const appSource = [
  readFileSync(resolve(here, "../app.js"), "utf8"),
  readFileSync(resolve(here, "../board-logs-surface.js"), "utf8"),
].join("\n");
const indexSource = readFileSync(resolve(here, "../index.html"), "utf8");

const KINDS = ["gh", "git", "docker", "agent", "runner"];

test("Logs window state seeds processKind alongside severity/query", () => {
  // `ensureLogState` must initialize the new facet so existing call sites
  // that read `state.processKind` never see `undefined`.
  assert.match(
    appSource,
    /ensureLogState[\s\S]+processKind:\s*""/,
    "ensureLogState should initialize processKind to ''",
  );
});

test("filteredLogEntries AND-combines processKind with severity + query", () => {
  // The filter must consult logMatchesProcessKind in addition to the
  // existing severity rank + keyword query — otherwise the chip cannot
  // narrow the visible entries.
  assert.match(appSource, /function filteredLogEntries\(state\)/);
  assert.match(
    appSource,
    /logMatchesProcessKind\(entry,\s*processKind\)/,
    "filteredLogEntries must call logMatchesProcessKind",
  );
  assert.match(
    appSource,
    /function logMatchesProcessKind\(entry,\s*processKind\)/,
    "helper logMatchesProcessKind must exist",
  );
  assert.match(
    appSource,
    /String\(fields\.kind\s*\|\|\s*""\)\s*===\s*processKind/,
    "helper compares entry.fields.kind to the selected processKind",
  );
});

test("renderLogs syncs the chip element and bails when missing", () => {
  // Without these the chip would never reflect external state changes
  // (e.g., persisted `processKind`) and rendering would race the DOM.
  assert.match(
    appSource,
    /\.logs-process-kind-select/,
    "renderLogs must look up the chip element",
  );
  assert.match(
    appSource,
    /processKindSelect\.value\s*=\s*state\.processKind/,
    "renderLogs must sync the select value from state",
  );
});

test("Process kind chip wires a change handler that updates state and re-renders", () => {
  assert.match(
    appSource,
    /\.logs-process-kind-select['"]?\s*\)[\s\S]*?addEventListener\("change",[\s\S]*?state\.processKind\s*=\s*event\.target\.value/,
    "Process kind select change must update state.processKind",
  );
});

test("Logs window scaffold exposes the canonical KIND options", () => {
  // The scaffold lives as a template literal inside app.js; index.html
  // does not host the Logs window markup directly. Asserting against
  // the template keeps the chip honest if anyone refactors the markup.
  assert.match(
    appSource,
    /<select class="logs-process-kind-select">/,
    "logs filter bar must include the process kind select",
  );
  assert.match(appSource, /<option value="">All<\/option>/);
  for (const kind of KINDS) {
    const re = new RegExp(`<option value="${kind}">${kind}</option>`);
    assert.match(
      appSource,
      re,
      `process kind select must offer the canonical ${kind} option`,
    );
  }
});

test("index.html does not duplicate the Logs window scaffold", () => {
  // Defensive: index.html should not own Logs window markup because the
  // scaffold lives in app.js (see the Logs render branch). If anyone moves
  // it here we want CI to flag the duplication so the chip wiring stays
  // single-sourced.
  assert.doesNotMatch(
    indexSource,
    /logs-process-kind-select/,
    "logs filter bar must not be defined in index.html",
  );
});
