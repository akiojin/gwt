// Issue #3962 AC-5 — a saved launch profile pinned to a model that left the
// agent's catalog (`gpt-5.4` after the 2026-09-05 Codex picker snapshot) falls
// back to the current default model instead of failing the launch, and the
// wizard says so where the Model field is rendered.
//
// Source-pattern contract tests (matching launch-wizard-hermes-options.test.mjs):
// they pin the wiring between the backend view field and the surface so the
// fallback can never go back to being silent.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const surface = readFileSync(resolve(here, "../launch-wizard-surface.js"), "utf8");

function agentSettingsSection() {
  const start = surface.indexOf("if (launchWizard.show_agent_settings) {");
  const end = surface.indexOf("if (launchWizard.show_reasoning) {", start);
  assert.ok(start > 0 && end > start, "agent settings section must exist");
  return surface.slice(start, end);
}

test("the model fallback notice renders next to the Model field", () => {
  const section = agentSettingsSection();
  assert.match(
    section,
    /if \(launchWizard\.model_fallback_notice\) \{/,
    "the surface must read the backend's model_fallback_notice field",
  );
  assert.match(
    section,
    /createNode\(\s*"div",\s*"launch-note",\s*launchWizard\.model_fallback_notice,?\s*\)/,
    "the notice reuses the shared non-blocking launch-note style",
  );
});

test("the notice is a hint, not an error, so it never blocks the launch", () => {
  const section = agentSettingsSection();
  // It must not be routed through the wizard error banner, which drives
  // pending-action teardown (`shouldClearLaunchWizardPendingAction`).
  assert.doesNotMatch(section, /wizardError/);
  assert.doesNotMatch(section, /launchWizard\.error\s*=/);
});
