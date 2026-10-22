// Issue #4911 — the Issue Monitor settings form repeats the Agent Settings
// block once per launch candidate. The backend owns the set list (order, which
// set is open, why `＋` / `−` are unavailable), so the surface must render that
// view and send the four set actions rather than keep a list of its own.
//
// Source-pattern contract tests (matching launch-wizard-pool-impact.test.mjs);
// the rendered behavior is covered in Chromium by
// playwright/tests/launch-wizard-agent-settings-sets.spec.ts.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const surface = readFileSync(resolve(here, "../launch-wizard-surface.js"), "utf8");
const css = readFileSync(resolve(here, "../styles/app.css"), "utf8");

function renderer() {
  const start = surface.indexOf("function appendAgentSettingsSets(");
  assert.ok(start > 0, "the Agent Settings set renderer must exist");
  return surface.slice(start, surface.indexOf("function appendChoiceField(", start));
}

test("the settings form renders its sets from the backend view", () => {
  assert.match(
    surface,
    /appendAgentSettingsSets\(\s*panel,\s*launchWizard\.issue_monitor_pool,?\s*\)/,
    "the surface must render the backend's issue_monitor_pool",
  );
  const body = renderer();
  assert.match(body, /pool\.active_index/, "the open set comes from the backend");
  assert.match(body, /pool\.add_disabled_reason/);
  assert.match(body, /pool\.remove_disabled_reason/);
});

test("every set edit is a wizard action the backend applies", () => {
  const body = renderer();
  for (const kind of [
    "add_agent_settings_set",
    "remove_agent_settings_set",
    "move_agent_settings_set",
    "select_agent_settings_set",
  ]) {
    assert.match(body, new RegExp(`kind:\\s*"${kind}"`), `${kind} must be dispatched`);
  }
});

test("the existing form sections render inside the open set", () => {
  const start = surface.indexOf("appendAgentSettingsSets(");
  const tail = surface.slice(surface.indexOf("appendAgentSettingsSets(", start + 1));
  assert.match(
    tail,
    /"Choose what to launch on the selected branch\."[\s\S]*?setupParent\.appendChild\(section\)/,
    "the Launch section must go into the open set, not next to the list",
  );
});

test("the sets are styled with Operator design tokens only", () => {
  const start = css.indexOf(".launch-agent-sets {");
  assert.ok(start > 0, "the set list must have a style rule");
  const block = css.slice(start, css.indexOf(".launch-agent-sets__footer {", start) + 400);
  assert.doesNotMatch(
    block,
    /#[0-9a-fA-F]{3,8}\b|rgba?\(/,
    "no raw hex / rgb colors: Operator tokens only",
  );
  assert.match(block, /var\(--color-border\)/);
});
