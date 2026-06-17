// SPEC-2359 US-80 — the Launch Wizard exposes an optional, always-skippable
// Start Work intake prompt that drives a non-blocking duplicate-work advisory.
// These source-assertions lock in the wiring contract between the wizard
// surface and the app shell.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const surface = readFileSync(resolve(here, "../launch-wizard-surface.js"), "utf8");
const appJs = readFileSync(resolve(here, "../app.js"), "utf8");

test("wizard surface accepts requestWorkAdvisory and exports the result handler", () => {
  assert.match(surface, /createLaunchWizardSurface\(\{[\s\S]*requestWorkAdvisory[\s\S]*\}\)/);
  assert.match(surface, /applyWorkAdvisoryResultEvent/);
});

test("wizard surface dispatches set_initial_prompt and request_work_advisory", () => {
  assert.match(surface, /kind:\s*"set_initial_prompt"/);
  assert.match(surface, /requestWorkAdvisory\(\{/);
});

test("intake panel is gated on the Start Work launch and is skippable copy", () => {
  assert.match(surface, /isStartWorkLaunch/);
  assert.match(surface, /What are you working on\?/);
  assert.match(surface, /You can skip this/);
});

test("advisory ignores stale responses and stays quiet when empty", () => {
  // stale-response guard
  assert.match(surface, /request_id\s*<\s*wizardAdvisoryLatestRequestId/);
  // empty results keep the panel hidden
  assert.match(surface, /wizardAdvisoryResults\.length/);
});

test("clearing the prompt invalidates an in-flight advisory request", () => {
  // SPEC-2359 US-80: clearing the intake prompt must advance the latest
  // request id so a late response for deleted text cannot repopulate the
  // cleared panel. The empty-query branch advances the seq before returning.
  const emptyBranch = surface.slice(
    surface.indexOf("Empty prompt"),
    surface.indexOf("Empty prompt") + 400,
  );
  assert.match(emptyBranch, /wizardAdvisoryRequestSeq\s*\+=\s*1/);
  assert.match(emptyBranch, /wizardAdvisoryLatestRequestId\s*=\s*wizardAdvisoryRequestSeq/);
});

test("advisory shows an in-flight loading indicator during the search", () => {
  // The semantic search cold-loads the embedding model (several seconds), so a
  // non-empty prompt must surface a loading indicator immediately rather than a
  // blank/unresponsive panel.
  assert.match(surface, /wizardAdvisoryLoading\s*=\s*true/);
  assert.match(surface, /wizardAdvisoryLoading\s*=\s*false/);
  assert.match(surface, /Searching related work/);
  // Reuses the existing project-index loading-dots animation.
  assert.match(surface, /index-search-loading-dot/);
});

test("app shell wires the advisory sender and routes the result event", () => {
  assert.match(appJs, /function requestWorkAdvisory\(/);
  assert.match(appJs, /kind:\s*"request_work_advisory"/);
  assert.match(appJs, /case "work_advisory_result":/);
  assert.match(appJs, /applyWorkAdvisoryResultEvent\(event\)/);
});
