// SPEC-2359 US-80 — the Launch Wizard exposes an optional, always-skippable
// work-registration prompt that drives a non-blocking duplicate-work advisory.
// These source-assertions lock in the wiring contract between the wizard
// surface and the app shell.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import vm from "node:vm";

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

test("work-registration panel is gated on Plan Agent launch and is skippable copy", () => {
  assert.match(surface, /isStartWorkLaunch/);
  assert.match(surface, /Register an Issue/);
  assert.match(surface, /Plan Agent/);
  assert.match(surface, /You can skip this/);
});

test("advisory ignores stale responses and stays quiet when empty", () => {
  // stale-response guard
  assert.match(surface, /request_id\s*<\s*wizardAdvisoryLatestRequestId/);
  // empty results keep the panel hidden
  assert.match(surface, /wizardAdvisoryResults\.length/);
});

test("clearing the prompt invalidates an in-flight advisory request", () => {
  // SPEC-2359 US-80: clearing the work-registration prompt must advance the latest
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

test("advisory reconnect recovers a lost read without duplicating queued or unfinished debounce work", () => {
  const timers = new Map(), requests = [];
  let timerId = 0;
  const context = vm.createContext({
    launchWizard: { show_branch_controls: false }, wizardPromptDraft: "  shared state  ",
    wizardAdvisoryRequestSeq: 0, wizardAdvisoryLatestRequestId: 0,
    wizardAdvisoryTimer: 0, wizardAdvisoryLoading: false, wizardAdvisoryResults: [],
    requestWorkAdvisory: request => requests.push(request), renderWorkAdvisoryPanel() {},
    setTimeout: callback => { timers.set(++timerId, callback); return timerId; },
    clearTimeout: id => timers.delete(id),
  });
  for (const name of ["isStartWorkLaunch", "scheduleWorkAdvisory", "applyWorkAdvisoryResultEvent", "resyncWorkAdvisory"]) {
    const start = surface.indexOf(`      function ${name}(`);
    const end = surface.indexOf("\n      }", start) + 8;
    vm.runInContext(start < 0 ? `function ${name}() {}` : surface.slice(start, end), context);
  }
  const flush = () => {
    const callbacks = [...timers.values()]; timers.clear(); callbacks.forEach(callback => callback());
  };
  context.scheduleWorkAdvisory(); flush();
  assert.equal(requests.length, 1);
  context.resyncWorkAdvisory([]); flush();
  assert.equal(requests.length, 2, "a lost sent reply gets one replacement read");
  assert.equal(requests[1].query, "shared state");
  assert.equal(context.wizardPromptDraft, "  shared state  ");
  context.applyWorkAdvisoryResultEvent({ request_id: requests[0].request_id, results: [] });
  assert.equal(context.wizardAdvisoryLoading, true, "the old reply cannot settle recovery");
  context.applyWorkAdvisoryResultEvent({ request_id: requests[1].request_id, results: [] });
  context.resyncWorkAdvisory([]); flush();
  assert.equal(requests.length, 2, "settled reads stay settled");
  context.scheduleWorkAdvisory();
  context.resyncWorkAdvisory([]);
  assert.equal(timers.size, 1, "an existing debounce keeps its owner");
  flush();
  context.resyncWorkAdvisory([requests.at(-1).request_id]); flush();
  assert.equal(requests.length, 3, "the read flushed from the queue is already in flight");
  context.resyncWorkAdvisory([]);
  context.launchWizard = null;
  flush();
  context.resyncWorkAdvisory([]); flush();
  assert.equal(requests.length, 3, "closing before the retry debounce prevents its send");
});
