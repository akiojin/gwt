// Issue #4079 AC-2 — the Issue Monitor Agent Settings form always writes
// candidate 1 of the launch candidate pool. When another provider already sits
// at index 0 the switch has to be visible before it is committed, otherwise the
// save looks like it did nothing (the reported symptom: picking Codex left the
// Monitor launching Claude).
//
// Source-pattern contract tests (matching launch-wizard-model-fallback.test.mjs):
// they pin the wiring between the backend view field and the surface.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const surface = readFileSync(resolve(here, "../launch-wizard-surface.js"), "utf8");
const css = readFileSync(resolve(here, "../styles/app.css"), "utf8");

test("the pool impact note renders inside the agent settings section", () => {
  const start = surface.indexOf("appendAgentSetupNote(section, launchWizard.agent_setup);");
  assert.ok(start > 0, "the agent settings section must exist");
  const section = surface.slice(start, start + 400);
  assert.match(
    section,
    /appendIssueMonitorPoolImpactNote\(\s*section,\s*launchWizard\.issue_monitor_pool_impact,?\s*\)/,
    "the surface must render the backend's issue_monitor_pool_impact next to the agent picker",
  );
});

test("the note states the replaced candidate and the resulting pool summary", () => {
  const start = surface.indexOf("function appendIssueMonitorPoolImpactNote(");
  assert.ok(start > 0, "the pool impact renderer must exist");
  const renderer = surface.slice(start, surface.indexOf("function appendChoiceField(", start));
  // The backend owns the wording (title = which candidate, detail = what the
  // pool becomes), so the surface must render both rather than re-deriving one.
  assert.match(renderer, /impact\.title/, "the note renders the backend title");
  assert.match(renderer, /impact\.detail/, "the note renders the backend detail");
  assert.match(
    renderer,
    /dataset\.poolAction\s*=\s*impact\.action/,
    "the chosen action (replace_head) must be readable from the DOM",
  );
  assert.match(
    renderer,
    /createNode\("div",\s*"launch-note launch-pool-impact"\)/,
    "the note reuses the shared non-blocking launch-note style",
  );
});

test("the note is a hint, not an error, so it never blocks the save", () => {
  const start = surface.indexOf("function appendIssueMonitorPoolImpactNote(");
  const renderer = surface.slice(start, surface.indexOf("function appendChoiceField(", start));
  assert.doesNotMatch(renderer, /wizardError/);
  assert.doesNotMatch(renderer, /launchWizard\.error\s*=/);
});

test("the pool impact note is styled with Operator design tokens only", () => {
  const start = css.indexOf(".launch-pool-impact {");
  assert.ok(start > 0, "the note must have a style rule");
  const block = css.slice(start, css.indexOf("}", css.indexOf(".launch-pool-impact__title")));
  assert.doesNotMatch(
    block,
    /#[0-9a-fA-F]{3,8}\b|rgba?\(/,
    "no raw hex / rgb colors: Operator tokens only",
  );
  assert.match(block, /var\(--color-border\)/);
});
