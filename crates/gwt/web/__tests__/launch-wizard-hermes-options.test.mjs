// Issue #3863 — Hermes launch options (Model / Profile / Toolsets / Skills)
// are config-sourced pickers with an "Other…" free-text fallback, and the
// Hermes-specific values are restored from the previous launch.
//
// Source-pattern contract tests (matching launch-wizard-intake.test.mjs):
// they pin the wiring between the wizard surface and the backend view so a
// field cannot silently regress to free-text input or lose its option list.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const surface = readFileSync(resolve(here, "../launch-wizard-surface.js"), "utf8");
const css = readFileSync(resolve(here, "../styles/app.css"), "utf8");

function hermesSection() {
  const start = surface.indexOf("launchWizard.show_hermes_options)");
  // SPEC-3864 moved the OpenCode "not set up" hint (FR-009/010) out to the
  // agent-independent setup affordance, so the OpenCode options comment that
  // closes the Hermes section now names FR-008 only.
  const end = surface.indexOf("SPEC-3151 FR-008", start);
  assert.ok(start > 0 && end > start, "Hermes options section must exist");
  return surface.slice(start, end);
}

test("single-choice picker is generalized beyond Provider and keeps the 3-tier layout", () => {
  assert.match(surface, /function appendHermesChoiceField\(/);
  assert.doesNotMatch(surface, /function appendHermesProviderField\(/);
  const fn = surface.slice(
    surface.indexOf("function appendHermesChoiceField("),
    surface.indexOf("function splitCsvValues("),
  );
  // (use config default) / config choices / Other… free text.
  assert.match(fn, /"\(use config default\)"/);
  assert.match(fn, /addOption\("__other__", "Other…"\)/);
  assert.match(fn, /otherInput\.addEventListener\("change"/);
});

test("Provider, Model and Profile render through the single-choice picker with their option lists", () => {
  const section = hermesSection();
  assert.match(
    section,
    /appendHermesChoiceField\(\s*grid,\s*"Provider",\s*launchWizard\.hermes_provider,\s*launchWizard\.hermes_provider_options \|\| \[\]/,
  );
  assert.match(
    section,
    /appendHermesChoiceField\(\s*grid,\s*"Model",\s*launchWizard\.selected_model,\s*launchWizard\.hermes_model_options \|\| \[\]/,
  );
  assert.match(
    section,
    /appendHermesChoiceField\(\s*grid,\s*"Profile",\s*launchWizard\.hermes_profile,\s*launchWizard\.hermes_profile_options \|\| \[\]/,
  );
  // Model keeps dispatching set_model so "Other…" reaches the free-text state.
  assert.match(section, /kind:\s*"set_model"/);
});

test("Toolsets and Skills render through the multi-choice picker and still emit CSV", () => {
  assert.match(surface, /function appendHermesMultiChoiceField\(/);
  const section = hermesSection();
  assert.match(
    section,
    /appendHermesMultiChoiceField\(\s*grid,\s*"Toolsets",\s*launchWizard\.hermes_toolsets,\s*launchWizard\.hermes_toolset_options \|\| \[\]/,
  );
  assert.match(
    section,
    /appendHermesMultiChoiceField\(\s*grid,\s*"Skills",\s*launchWizard\.hermes_skills,\s*launchWizard\.hermes_skill_options \|\| \[\]/,
  );
  const fn = surface.slice(
    surface.indexOf("function appendHermesMultiChoiceField("),
    surface.indexOf("function appendCheckboxField("),
  );
  assert.match(fn, /checkbox\.type = "checkbox"/);
  assert.match(fn, /onChange\(values\.join\(","\)\)/);
  // Free-text fallback for values missing from config.
  assert.match(fn, /`Other \$\{label\.toLowerCase\(\)\}`/);
  // No candidates → only the CSV input remains (no empty group).
  assert.match(fn, /if \(known\.length > 0\)/);
});

test("Hermes fields no longer fall back to bare free-text inputs", () => {
  const section = hermesSection();
  assert.doesNotMatch(section, /appendTextField\(\s*grid,\s*"Model"/);
  assert.doesNotMatch(section, /appendTextField\(\s*grid,\s*"Profile"/);
  assert.doesNotMatch(section, /appendTextField\(\s*grid,\s*"Toolsets"/);
  assert.doesNotMatch(section, /appendTextField\(\s*grid,\s*"Skills"/);
});

test("multi-choice group styling uses Operator spacing tokens", () => {
  const block = css.slice(
    css.indexOf(".launch-multi-choice {"),
    css.indexOf("}", css.indexOf(".launch-multi-choice {")),
  );
  assert.match(block, /gap: var\(--space-2\) var\(--space-3\)/);
  assert.doesNotMatch(block, /#[0-9a-fA-F]{3,8}\b|rgba?\(/);
});
